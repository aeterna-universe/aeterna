//! Integration Tests for CCA Summary Storage and Retrieval
//!
//! Tests the full summary lifecycle across Redis (cache) and PostgreSQL
//! (persistence).

use mk_core::types::{LayerSummary, MemoryEntry, MemoryLayer, SummaryConfig, SummaryDepth};
use std::collections::HashMap;
use storage::redis::RedisStorage;
use testing::{redis, unique_id};

fn create_test_summary(depth: SummaryDepth, content: &str) -> LayerSummary {
    LayerSummary {
        depth,
        content: content.to_string(),
        token_count: content.split_whitespace().count() as u32,
        generated_at: chrono::Utc::now().timestamp(),
        source_hash: format!("hash_{}", content.len()),
        content_hash: None,
        personalized: false,
        personalization_context: None,
    }
}

fn create_test_memory_entry(id: &str, content: &str, layer: MemoryLayer) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        content: content.to_string(),
        embedding: None,
        layer,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: chrono::Utc::now().timestamp(),
        updated_at: chrono::Utc::now().timestamp(),
    }
}

#[tokio::test]
async fn test_summary_cache_set_and_get() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let summary = create_test_summary(SummaryDepth::Sentence, "Test sentence summary");

    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Project, &entry_id, &summary, None)
        .await?;

    let retrieved = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Project,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;

    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.content, "Test sentence summary");
    assert_eq!(retrieved.depth, SummaryDepth::Sentence);

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_all_depths() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let sentence = create_test_summary(SummaryDepth::Sentence, "Short summary");
    let paragraph = create_test_summary(
        SummaryDepth::Paragraph,
        "Medium length paragraph summary with more detail",
    );
    let detailed = create_test_summary(
        SummaryDepth::Detailed,
        "Comprehensive detailed summary with extensive context and background information",
    );

    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Team, &entry_id, &sentence, None)
        .await?;
    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Team, &entry_id, &paragraph, None)
        .await?;
    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Team, &entry_id, &detailed, None)
        .await?;

    let all_summaries = redis
        .get_all_summaries_for_entry(&tenant_id, &MemoryLayer::Team, &entry_id)
        .await?;

    assert_eq!(all_summaries.len(), 3);
    assert!(all_summaries.contains_key(&SummaryDepth::Sentence));
    assert!(all_summaries.contains_key(&SummaryDepth::Paragraph));
    assert!(all_summaries.contains_key(&SummaryDepth::Detailed));

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_invalidation_single_depth() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let sentence = create_test_summary(SummaryDepth::Sentence, "To be invalidated");
    let paragraph = create_test_summary(SummaryDepth::Paragraph, "Should remain");

    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Org, &entry_id, &sentence, None)
        .await?;
    redis
        .set_summary_cache(&tenant_id, &MemoryLayer::Org, &entry_id, &paragraph, None)
        .await?;

    let deleted = redis
        .invalidate_summary_cache(
            &tenant_id,
            &MemoryLayer::Org,
            &entry_id,
            Some(&SummaryDepth::Sentence),
        )
        .await?;
    assert_eq!(deleted, 1);

    let sentence_result = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Org,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;
    assert!(sentence_result.is_none());

    let paragraph_result = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Org,
            &entry_id,
            &SummaryDepth::Paragraph,
        )
        .await?;
    assert!(paragraph_result.is_some());

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_invalidation_all_depths() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    for depth in [
        SummaryDepth::Sentence,
        SummaryDepth::Paragraph,
        SummaryDepth::Detailed,
    ] {
        let summary = create_test_summary(depth, &format!("{:?} summary", depth));
        redis
            .set_summary_cache(&tenant_id, &MemoryLayer::Tenant, &entry_id, &summary, None)
            .await?;
    }

    let deleted = redis
        .invalidate_summary_cache(&tenant_id, &MemoryLayer::Tenant, &entry_id, None)
        .await?;
    assert_eq!(deleted, 3);

    let all = redis
        .get_all_summaries_for_entry(&tenant_id, &MemoryLayer::Tenant, &entry_id)
        .await?;
    assert!(all.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_with_ttl() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let summary = create_test_summary(SummaryDepth::Sentence, "Expires soon");

    redis
        .set_summary_cache(
            &tenant_id,
            &MemoryLayer::Session,
            &entry_id,
            &summary,
            Some(1),
        )
        .await?;

    let immediate = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Session,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;
    assert!(immediate.is_some());

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let after_expiry = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Session,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;
    assert!(after_expiry.is_none());

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_tenant_isolation() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_a = unique_id("tenant_a");
    let tenant_b = unique_id("tenant_b");
    let entry_id = "shared_entry_id";

    let summary_a = create_test_summary(SummaryDepth::Sentence, "Tenant A summary");
    let summary_b = create_test_summary(SummaryDepth::Sentence, "Tenant B summary");

    redis
        .set_summary_cache(&tenant_a, &MemoryLayer::Project, entry_id, &summary_a, None)
        .await?;
    redis
        .set_summary_cache(&tenant_b, &MemoryLayer::Project, entry_id, &summary_b, None)
        .await?;

    let retrieved_a = redis
        .get_summary_cache(
            &tenant_a,
            &MemoryLayer::Project,
            entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;
    let retrieved_b = redis
        .get_summary_cache(
            &tenant_b,
            &MemoryLayer::Project,
            entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;

    assert_eq!(retrieved_a.unwrap().content, "Tenant A summary");
    assert_eq!(retrieved_b.unwrap().content, "Tenant B summary");

    Ok(())
}

#[tokio::test]
async fn test_summary_cache_layer_isolation() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let project_summary = create_test_summary(SummaryDepth::Sentence, "Project layer");
    let team_summary = create_test_summary(SummaryDepth::Sentence, "Team layer");

    redis
        .set_summary_cache(
            &tenant_id,
            &MemoryLayer::Project,
            &entry_id,
            &project_summary,
            None,
        )
        .await?;
    redis
        .set_summary_cache(
            &tenant_id,
            &MemoryLayer::Team,
            &entry_id,
            &team_summary,
            None,
        )
        .await?;

    let project_result = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Project,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;
    let team_result = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::Team,
            &entry_id,
            &SummaryDepth::Sentence,
        )
        .await?;

    assert_eq!(project_result.unwrap().content, "Project layer");
    assert_eq!(team_result.unwrap().content, "Team layer");

    Ok(())
}

#[tokio::test]
async fn test_memory_entry_needs_summary_update() {
    let config = SummaryConfig {
        layer: MemoryLayer::Project,
        update_interval_secs: Some(3600),
        update_on_changes: None,
        skip_if_unchanged: false,
        personalized: false,
        depths: vec![SummaryDepth::Sentence, SummaryDepth::Paragraph],
    };

    let mut entry = create_test_memory_entry("test-1", "Original content", MemoryLayer::Project);
    let current_time = chrono::Utc::now().timestamp();

    assert!(
        entry.needs_summary_update(&config, current_time),
        "Entry without summaries should need update"
    );

    let summary = LayerSummary {
        depth: SummaryDepth::Sentence,
        content: "Summary of original content".to_string(),
        token_count: 4,
        generated_at: current_time,
        source_hash: entry.compute_content_hash(),
        content_hash: None,
        personalized: false,
        personalization_context: None,
    };
    entry.summaries.insert(SummaryDepth::Sentence, summary);

    assert!(
        entry.needs_summary_update(&config, current_time),
        "Entry missing Paragraph summary should need update"
    );

    let para_summary = LayerSummary {
        depth: SummaryDepth::Paragraph,
        content: "Paragraph summary".to_string(),
        token_count: 2,
        generated_at: current_time,
        source_hash: entry.compute_content_hash(),
        content_hash: None,
        personalized: false,
        personalization_context: None,
    };
    entry
        .summaries
        .insert(SummaryDepth::Paragraph, para_summary);

    assert!(
        !entry.needs_summary_update(&config, current_time),
        "Entry with all current summaries should not need update"
    );

    entry.content = "Modified content".to_string();
    assert!(
        entry.needs_summary_update(&config, current_time),
        "Entry with changed content should need update"
    );
}

#[tokio::test]
async fn test_memory_entry_summary_staleness() {
    let config = SummaryConfig {
        layer: MemoryLayer::Team,
        update_interval_secs: Some(60),
        update_on_changes: None,
        skip_if_unchanged: false,
        personalized: false,
        depths: vec![SummaryDepth::Detailed],
    };

    let mut entry = create_test_memory_entry("test-2", "Some content", MemoryLayer::Team);
    let old_time = chrono::Utc::now().timestamp() - 120;
    let current_time = chrono::Utc::now().timestamp();

    let old_summary = LayerSummary {
        depth: SummaryDepth::Detailed,
        content: "Old detailed summary".to_string(),
        token_count: 3,
        generated_at: old_time,
        source_hash: entry.compute_content_hash(),
        content_hash: None,
        personalized: false,
        personalization_context: None,
    };
    entry.summaries.insert(SummaryDepth::Detailed, old_summary);

    assert!(
        entry.needs_summary_update(&config, current_time),
        "Entry with stale summary (older than interval) should need update"
    );
}

#[tokio::test]
async fn test_personalized_summary_storage() -> Result<(), anyhow::Error> {
    let Some(redis_fixture) = redis().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let redis = RedisStorage::new(redis_fixture.url()).await?;
    let tenant_id = unique_id("tenant");
    let entry_id = unique_id("entry");

    let personalized_summary = LayerSummary {
        depth: SummaryDepth::Paragraph,
        content: "Personalized summary for senior Rust developer".to_string(),
        token_count: 6,
        generated_at: chrono::Utc::now().timestamp(),
        source_hash: "abc123".to_string(),
        content_hash: None,
        personalized: true,
        personalization_context: Some(
            "Senior Rust developer, prefers concise explanations".to_string(),
        ),
    };

    redis
        .set_summary_cache(
            &tenant_id,
            &MemoryLayer::User,
            &entry_id,
            &personalized_summary,
            None,
        )
        .await?;

    let retrieved = redis
        .get_summary_cache(
            &tenant_id,
            &MemoryLayer::User,
            &entry_id,
            &SummaryDepth::Paragraph,
        )
        .await?
        .expect("Summary should exist");

    assert!(retrieved.personalized);
    assert_eq!(
        retrieved.personalization_context,
        Some("Senior Rust developer, prefers concise explanations".to_string())
    );

    Ok(())
}
