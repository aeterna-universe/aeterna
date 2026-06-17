//! Regression tests for tenant metadata after the account/environment refactor.
//!
//! This file intentionally keeps its historical filename to preserve the
//! original test target path, but the semantics now validate the canonical
//! account-owned tenant model instead of the removed `legal_entity_name`
//! stepping-stone field.

use mk_core::types::RecordSource;
use storage::account_store::AccountStore;
use storage::postgres::PostgresBackend;
use storage::tenant_store::TenantStore;
use testing::{postgres, unique_id};

async fn create_stores() -> Option<(TenantStore, AccountStore)> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;
    backend.initialize_schema().await.ok()?;
    let pool = backend.pool().clone();
    Some((TenantStore::new(pool.clone()), AccountStore::new(pool)))
}

#[tokio::test]
async fn tenant_metadata_defaults_to_no_account_and_no_environment_on_create() {
    let Some((tenant_store, _account_store)) = create_stores().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let slug = unique_id("t-default");
    let created = tenant_store
        .create_tenant_with_source(&slug, "Default Tenant", RecordSource::Admin)
        .await
        .expect("create_tenant_with_source");

    assert_eq!(created.account, None);
    assert_eq!(created.environment, None);

    let fetched = tenant_store
        .get_tenant(&slug)
        .await
        .expect("get_tenant")
        .expect("tenant exists");
    assert_eq!(fetched.account, None);
    assert_eq!(fetched.environment, None);
}

#[tokio::test]
async fn tenant_environment_can_be_set_via_update() {
    let Some((tenant_store, _account_store)) = create_stores().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let slug = unique_id("t-env");
    tenant_store
        .create_tenant_with_source(&slug, "Acme Prod", RecordSource::Admin)
        .await
        .unwrap();

    let updated = tenant_store
        .update_tenant(&slug, None, None, Some("prod"))
        .await
        .expect("update_tenant")
        .expect("tenant exists");

    assert_eq!(updated.environment.as_deref(), Some("prod"));
}

#[tokio::test]
async fn tenant_environment_is_left_alone_when_omitted() {
    let Some((tenant_store, _account_store)) = create_stores().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let slug = unique_id("t-keep");
    tenant_store
        .create_tenant_with_source(&slug, "Keep Tenant", RecordSource::Admin)
        .await
        .unwrap();

    tenant_store
        .update_tenant(&slug, None, None, Some("staging"))
        .await
        .unwrap()
        .unwrap();

    let after = tenant_store
        .update_tenant(&slug, None, Some("Renamed Tenant"), None)
        .await
        .expect("update_tenant")
        .expect("tenant exists");

    assert_eq!(after.name, "Renamed Tenant");
    assert_eq!(after.environment.as_deref(), Some("staging"));
}

#[tokio::test]
async fn tenant_can_attach_and_detach_account() {
    let Some((tenant_store, account_store)) = create_stores().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let slug = unique_id("t-account");
    tenant_store
        .create_tenant_with_source(&slug, "Account Tenant", RecordSource::Admin)
        .await
        .unwrap();

    let account = account_store
        .create(&unique_id("acct"), "Acme")
        .await
        .expect("create account");

    let attached = tenant_store
        .attach_account(&slug, &account.id)
        .await
        .expect("attach_account")
        .expect("tenant exists");

    let attached_account = attached.account.expect("attached account metadata");
    assert_eq!(attached_account.id, account.id);
    assert_eq!(attached_account.slug, account.slug);
    assert_eq!(attached_account.name, account.name);

    let detached = tenant_store
        .detach_account(&slug)
        .await
        .expect("detach_account")
        .expect("tenant exists");

    assert_eq!(detached.account, None);
}
