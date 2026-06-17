//! Round-trip + idempotency tests for `HierarchyStore` (§2.2-B, B3).
//!
//! Docker-gated; mirrors the skip-on-no-Docker convention used by
//! `migration_028_tenant_scoped_hierarchy_test.rs`.

use sqlx::postgres::PgPoolOptions;
use storage::hierarchy_store::{HierarchyStore, OrgInput, TeamInput, slugify};
use storage::migrations::apply_all;
use storage::postgres::PostgresBackend;
use testing::postgres;
use uuid::Uuid;

async fn fresh_tenant(pool: &sqlx::PgPool, tag: &str) -> Uuid {
    let slug = format!("{tag}-{}", Uuid::new_v4().simple());
    sqlx::query_scalar("INSERT INTO tenants (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(&slug)
        .fetch_one(pool)
        .await
        .expect("insert tenant")
}

#[tokio::test]
async fn hierarchy_store_round_trip_and_idempotent() {
    let Some(pg) = postgres().await else {
        eprintln!("Skipping hierarchy_store test: Docker not available");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(pg.url())
        .await
        .expect("connect fixture");
    PostgresBackend::new(pg.url())
        .await
        .expect("backend")
        .initialize_schema()
        .await
        .expect("init schema");
    apply_all(&pool).await.expect("apply_all");

    let store = HierarchyStore::new(pool.clone());
    let tenant_id = fresh_tenant(&pool, "hs-rt").await;

    let input = vec![
        OrgInput {
            slug: "platform".into(),
            name: "Platform".into(),
            teams: vec![
                TeamInput {
                    slug: "admins".into(),
                    name: "Admins".into(),
                },
                TeamInput {
                    slug: "sre".into(),
                    name: "SRE".into(),
                },
            ],
        },
        OrgInput {
            slug: "product".into(),
            name: "Product".into(),
            teams: vec![],
        },
    ];

    // ---- First apply ----
    let s1 = store
        .upsert_hierarchy(tenant_id, &input)
        .await
        .expect("upsert 1");
    assert_eq!(s1.orgs_upserted, 2);
    assert_eq!(s1.teams_upserted, 2);

    // ---- Read back ----
    let readback = store.get_hierarchy(tenant_id).await.expect("get_hierarchy");
    assert_eq!(readback.len(), 2);
    let org0 = &readback[0];
    let org1 = &readback[1];
    assert_eq!(org0.tenant_id, tenant_id);
    assert_eq!(org1.tenant_id, tenant_id);

    // `ORDER BY org_slug` → platform before product.
    assert_eq!(org0.slug, "platform");
    assert_eq!(org0.teams.len(), 2);
    // `ORDER BY team_slug` → admins before sre.
    assert_eq!(org0.teams[0].slug, "admins");
    assert_eq!(org0.teams[1].slug, "sre");

    assert_eq!(org1.slug, "product");
    assert!(org1.teams.is_empty());

    // ---- Idempotent re-apply of identical input ----
    let s2 = store
        .upsert_hierarchy(tenant_id, &input)
        .await
        .expect("upsert 2");
    assert_eq!(
        s2, s1,
        "repeat apply with identical input must report the same summary"
    );

    let readback2 = store
        .get_hierarchy(tenant_id)
        .await
        .expect("get_hierarchy 2");
    assert_eq!(
        readback, readback2,
        "hierarchy must be stable across repeat applies"
    );

    // ---- Rename (same slug, new name) flows through ON CONFLICT DO UPDATE ----
    let mut renamed = input.clone();
    renamed[0].name = "Platform Eng".into();
    renamed[0].teams[0].name = "Platform Admins".into();
    let _ = store
        .upsert_hierarchy(tenant_id, &renamed)
        .await
        .expect("upsert rename");

    let renamed_read = store.get_hierarchy(tenant_id).await.expect("get renamed");
    assert_eq!(renamed_read[0].name, "Platform Eng");
    assert_eq!(renamed_read[0].teams[0].name, "Platform Admins");
    // IDs must remain stable across rename.
    assert_eq!(renamed_read[0].id, readback[0].id);
}

#[tokio::test]
async fn hierarchy_store_is_tenant_isolated() {
    let Some(pg) = postgres().await else {
        eprintln!("Skipping hierarchy_store isolation test: Docker not available");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(pg.url())
        .await
        .expect("connect fixture");
    PostgresBackend::new(pg.url())
        .await
        .expect("backend")
        .initialize_schema()
        .await
        .expect("init schema");
    apply_all(&pool).await.expect("apply_all");

    let store = HierarchyStore::new(pool.clone());
    let tenant_a = fresh_tenant(&pool, "hs-iso-a").await;
    let tenant_b = fresh_tenant(&pool, "hs-iso-b").await;

    // Same slug 'shared' under both tenants — migration 028 allows this.
    let input_a = vec![OrgInput {
        slug: "shared".into(),
        name: "A's Shared".into(),
        teams: vec![TeamInput {
            slug: "t1".into(),
            name: "Team One A".into(),
        }],
    }];
    let input_b = vec![OrgInput {
        slug: "shared".into(),
        name: "B's Shared".into(),
        teams: vec![TeamInput {
            slug: "t1".into(),
            name: "Team One B".into(),
        }],
    }];

    store.upsert_hierarchy(tenant_a, &input_a).await.expect("a");
    store.upsert_hierarchy(tenant_b, &input_b).await.expect("b");

    let read_a = store.get_hierarchy(tenant_a).await.expect("read a");
    let read_b = store.get_hierarchy(tenant_b).await.expect("read b");

    assert_eq!(read_a.len(), 1);
    assert_eq!(read_b.len(), 1);
    assert_eq!(read_a[0].name, "A's Shared");
    assert_eq!(read_b[0].name, "B's Shared");
    assert_ne!(
        read_a[0].id, read_b[0].id,
        "different tenants with same slug must have distinct organization UUIDs"
    );
    assert_ne!(read_a[0].teams[0].id, read_b[0].teams[0].id);
    assert_eq!(read_a[0].teams[0].name, "Team One A");
    assert_eq!(read_b[0].teams[0].name, "Team One B");
}

#[test]
fn slugify_matches_bootstrap_convention() {
    // Sanity bridge between slugify() and the slugs bootstrap.rs/idp-sync
    // are known to produce. Not exhaustive — see hierarchy_store::slugify_tests
    // for the full rule-level coverage.
    assert_eq!(slugify("Default"), "default");
    assert_eq!(slugify("Platform"), "platform");
    assert_eq!(slugify("Admins"), "admins");
}
