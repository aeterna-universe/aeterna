use mk_core::traits::StorageBackend;
use mk_core::types::{
    CodeChange, ErrorSignature, HindsightNote, OrganizationalUnit, RecordSource, Resolution,
    TenantId, UnitType,
};
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};
use tokio::sync::OnceCell;

use knowledge::context_architect::{LlmClient, LlmError, ViewMode};
use knowledge::hindsight::{
    HindsightNoteGenerationMode, HindsightNoteGenerator, HindsightNoteGeneratorConfig,
    HindsightNoteRequest,
};

static SCHEMA_INITIALIZED: OnceCell<bool> = OnceCell::const_new();

async fn create_test_storage() -> Option<std::sync::Arc<PostgresBackend>> {
    let fixture = postgres().await?;
    let storage = std::sync::Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);

    SCHEMA_INITIALIZED
        .get_or_init(|| async {
            storage.initialize_schema().await.ok();
            true
        })
        .await;

    Some(storage)
}

struct MockLlmClient {
    responses: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, _prompt: &str) -> Result<String, LlmError> {
        let mut responses = self.responses.lock().await;
        responses
            .pop()
            .ok_or_else(|| LlmError::InvalidResponse("No mock response".into()))
    }

    async fn complete_with_system(&self, _system: &str, _user: &str) -> Result<String, LlmError> {
        let mut responses = self.responses.lock().await;
        responses
            .pop()
            .ok_or_else(|| LlmError::InvalidResponse("No mock response".into()))
    }
}

async fn create_tenant(storage: &PostgresBackend, tenant_id: &str) -> Result<(), anyhow::Error> {
    let unit = OrganizationalUnit {
        id: tenant_id.to_string(),
        name: format!("Test Tenant {}", tenant_id),
        unit_type: UnitType::Organization,
        parent_id: None,
        tenant_id: TenantId::new(tenant_id.to_string()).unwrap(),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&unit).await?;
    Ok(())
}

fn create_test_error_signature() -> ErrorSignature {
    ErrorSignature {
        error_type: "NullPointerException".to_string(),
        message_pattern: "Cannot read property '.*' of undefined".to_string(),
        stack_patterns: vec!["at processData".to_string(), "at handleRequest".to_string()],
        context_patterns: vec!["user-service".to_string()],
        embedding: Some(vec![0.1, 0.2, 0.3, 0.4, 0.5]),
    }
}

fn create_test_resolution(error_signature_id: &str) -> Resolution {
    Resolution {
        id: unique_id("res"),
        error_signature_id: error_signature_id.to_string(),
        description: "Add null check before accessing property".to_string(),
        changes: vec![CodeChange {
            file_path: "src/service.ts".to_string(),
            diff: "+ if (data) { data.value }".to_string(),
            description: Some("Added null guard".to_string()),
        }],
        success_rate: 0.95,
        application_count: 10,
        last_success_at: chrono::Utc::now().timestamp(),
    }
}

fn create_test_hindsight_note(
    error_signature: ErrorSignature,
    resolutions: Vec<Resolution>,
) -> HindsightNote {
    let now = chrono::Utc::now().timestamp();
    HindsightNote {
        id: unique_id("note"),
        error_signature,
        resolutions,
        content: "When encountering NullPointerException, always add null checks before property \
                  access."
            .to_string(),
        tags: vec!["null-safety".to_string(), "typescript".to_string()],
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn test_hindsight_note_generation_storage() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let resolution = create_test_resolution("sig");

    let client = MockLlmClient {
        responses: tokio::sync::Mutex::new(vec!["## Summary\nUse null checks.".to_string()]),
    };
    let generator = HindsightNoteGenerator::new(
        std::sync::Arc::new(client),
        HindsightNoteGeneratorConfig {
            mode: HindsightNoteGenerationMode::Single,
            ..Default::default()
        },
    )
    .with_storage(storage.clone());

    let request = HindsightNoteRequest::new(
        signature,
        vec![resolution],
        Some("running tests".to_string()),
        vec!["manual".to_string()],
        ViewMode::Dx,
    );

    let result = generator.generate_and_store(&tenant_id, &request).await?;

    let stored = storage
        .get_hindsight_note(&tenant_id, &result.note.id)
        .await?;
    assert!(stored.is_some());
    let stored = stored.unwrap();
    assert!(stored.tags.contains(&"manual".to_string()));
    assert!(stored.tags.contains(&"nullpointerexception".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_error_signature_crud() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;

    let retrieved = storage.get_error_signature(&tenant_id, &id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.error_type, signature.error_type);
    assert_eq!(retrieved.message_pattern, signature.message_pattern);
    assert_eq!(retrieved.stack_patterns, signature.stack_patterns);

    let deleted = storage.delete_error_signature(&tenant_id, &id).await?;
    assert!(deleted);

    let after_delete = storage.get_error_signature(&tenant_id, &id).await?;
    assert!(after_delete.is_none());

    Ok(())
}

#[tokio::test]
async fn test_error_signature_with_embedding() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = ErrorSignature {
        error_type: "TypeError".to_string(),
        message_pattern: "Expected string got number".to_string(),
        stack_patterns: vec![],
        context_patterns: vec![],
        embedding: Some(vec![0.1, 0.2, 0.3, 0.4, 0.5]),
    };

    let id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;
    let retrieved = storage.get_error_signature(&tenant_id, &id).await?;

    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert!(retrieved.embedding.is_some());
    assert_eq!(retrieved.embedding.unwrap().len(), 5);

    Ok(())
}

#[tokio::test]
async fn test_resolution_crud() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let sig_id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;

    let resolution = create_test_resolution(&sig_id);
    storage.create_resolution(&tenant_id, &resolution).await?;

    let retrieved = storage.get_resolution(&tenant_id, &resolution.id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.description, resolution.description);
    assert_eq!(retrieved.success_rate, resolution.success_rate);
    assert_eq!(retrieved.application_count, resolution.application_count);

    let deleted = storage
        .delete_resolution(&tenant_id, &resolution.id)
        .await?;
    assert!(deleted);

    Ok(())
}

#[tokio::test]
async fn test_resolutions_for_error() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let sig_id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;

    let res1 = Resolution {
        id: unique_id("res"),
        error_signature_id: sig_id.clone(),
        description: "First resolution".to_string(),
        changes: vec![],
        success_rate: 0.8,
        application_count: 5,
        last_success_at: 0,
    };

    let res2 = Resolution {
        id: unique_id("res"),
        error_signature_id: sig_id.clone(),
        description: "Second resolution".to_string(),
        changes: vec![],
        success_rate: 0.95,
        application_count: 15,
        last_success_at: 0,
    };

    storage.create_resolution(&tenant_id, &res1).await?;
    storage.create_resolution(&tenant_id, &res2).await?;

    let resolutions = storage
        .get_resolutions_for_error(&tenant_id, &sig_id)
        .await?;
    assert_eq!(resolutions.len(), 2);
    assert_eq!(resolutions[0].description, "Second resolution");

    Ok(())
}

#[tokio::test]
async fn test_hindsight_note_crud() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let note = create_test_hindsight_note(signature, vec![]);

    storage.create_hindsight_note(&tenant_id, &note).await?;

    let retrieved = storage.get_hindsight_note(&tenant_id, &note.id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.content, note.content);
    assert_eq!(retrieved.tags, note.tags);

    let deleted = storage.delete_hindsight_note(&tenant_id, &note.id).await?;
    assert!(deleted);

    let after_delete = storage.get_hindsight_note(&tenant_id, &note.id).await?;
    assert!(after_delete.is_none());

    Ok(())
}

#[tokio::test]
async fn test_hindsight_note_update() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let mut note = create_test_hindsight_note(signature, vec![]);
    storage.create_hindsight_note(&tenant_id, &note).await?;

    note.content = "Updated content with more details".to_string();
    note.tags.push("updated".to_string());
    note.updated_at = chrono::Utc::now().timestamp();

    let updated = storage.update_hindsight_note(&tenant_id, &note).await?;
    assert!(updated);

    let retrieved = storage.get_hindsight_note(&tenant_id, &note.id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.content, "Updated content with more details");
    assert!(retrieved.tags.contains(&"updated".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_hindsight_note_list() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    for i in 0..5 {
        let signature = ErrorSignature {
            error_type: format!("Error{}", i),
            message_pattern: format!("Pattern {}", i),
            stack_patterns: vec![],
            context_patterns: vec![],
            embedding: None,
        };
        let note = create_test_hindsight_note(signature, vec![]);
        storage.create_hindsight_note(&tenant_id, &note).await?;
    }

    let notes = storage.list_hindsight_notes(&tenant_id, 3, 0).await?;
    assert_eq!(notes.len(), 3);

    let notes = storage.list_hindsight_notes(&tenant_id, 10, 2).await?;
    assert_eq!(notes.len(), 3);

    Ok(())
}

#[tokio::test]
async fn test_tenant_isolation() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant1 = unique_id("tenant");
    let tenant2 = unique_id("tenant");
    create_tenant(&storage, &tenant1).await?;
    create_tenant(&storage, &tenant2).await?;

    let signature = create_test_error_signature();
    let id = storage.create_error_signature(&tenant1, &signature).await?;

    let from_tenant1 = storage.get_error_signature(&tenant1, &id).await?;
    assert!(from_tenant1.is_some());

    let from_tenant2 = storage.get_error_signature(&tenant2, &id).await?;
    assert!(from_tenant2.is_none());

    Ok(())
}

#[tokio::test]
async fn test_hindsight_note_with_resolutions() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let sig_id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;

    let resolution = create_test_resolution(&sig_id);
    storage.create_resolution(&tenant_id, &resolution).await?;

    let note = HindsightNote {
        id: unique_id("note"),
        error_signature: signature.clone(),
        resolutions: vec![resolution.clone()],
        content: "Note with resolution".to_string(),
        tags: vec![],
        created_at: chrono::Utc::now().timestamp(),
        updated_at: chrono::Utc::now().timestamp(),
    };

    storage.create_hindsight_note(&tenant_id, &note).await?;

    let retrieved = storage.get_hindsight_note(&tenant_id, &note.id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.resolutions.len(), 1);
    assert_eq!(retrieved.resolutions[0].id, resolution.id);

    Ok(())
}

#[tokio::test]
async fn test_code_change_serialization() -> Result<(), anyhow::Error> {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let tenant_id = unique_id("tenant");
    create_tenant(&storage, &tenant_id).await?;

    let signature = create_test_error_signature();
    let sig_id = storage
        .create_error_signature(&tenant_id, &signature)
        .await?;

    let resolution = Resolution {
        id: unique_id("res"),
        error_signature_id: sig_id,
        description: "Multi-file fix".to_string(),
        changes: vec![
            CodeChange {
                file_path: "src/a.ts".to_string(),
                diff: "+ line1".to_string(),
                description: Some("Fix A".to_string()),
            },
            CodeChange {
                file_path: "src/b.ts".to_string(),
                diff: "+ line2".to_string(),
                description: None,
            },
        ],
        success_rate: 1.0,
        application_count: 1,
        last_success_at: 0,
    };

    storage.create_resolution(&tenant_id, &resolution).await?;

    let retrieved = storage.get_resolution(&tenant_id, &resolution.id).await?;
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.changes.len(), 2);
    assert_eq!(retrieved.changes[0].file_path, "src/a.ts");
    assert_eq!(retrieved.changes[1].description, None);

    Ok(())
}
