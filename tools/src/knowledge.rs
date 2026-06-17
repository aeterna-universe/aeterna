use crate::tools::Tool;
use async_trait::async_trait;
use knowledge::manager::KnowledgeManager;
use memory::manager::MemoryManager;
use mk_core::traits::KnowledgeRepository;
use mk_core::types::{
    KnowledgeRelation, KnowledgeRelationType, KnowledgeVariantRole, PROVIDER_GITHUB,
    PromotionDecision, PromotionMode, PromotionRequest, PromotionRequestStatus, TenantContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use validator::Validate;

// ── Precedence helpers ────────────────────────────────────────────────────────

/// Maps a KnowledgeLayer to its canonical authority rank.
/// Lower = more authoritative.  Tenant is most authoritative (rank 0).
fn knowledge_layer_authority(layer: mk_core::types::KnowledgeLayer) -> u8 {
    match layer {
        mk_core::types::KnowledgeLayer::Tenant => 0,
        mk_core::types::KnowledgeLayer::Org => 1,
        mk_core::types::KnowledgeLayer::Team => 2,
        mk_core::types::KnowledgeLayer::Project => 3,
    }
}

pub struct KnowledgeGetTool {
    repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
}

impl KnowledgeGetTool {
    pub fn new(
        repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
    ) -> Self {
        Self { repo }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeGetParams {
    pub path: String,
    pub layer: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeListParams {
    pub layer: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeQueryParams {
    pub query: String,
    #[serde(default)]
    pub layers: Vec<String>,
    pub limit: Option<usize>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgeGetTool {
    fn name(&self) -> &str {
        "knowledge_get"
    }

    fn description(&self) -> &str {
        "Retrieve a specific knowledge entry by path and layer."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "layer": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["path", "layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeGetParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "tenant" => mk_core::types::KnowledgeLayer::Tenant,
            "org" => mk_core::types::KnowledgeLayer::Org,
            "team" => mk_core::types::KnowledgeLayer::Team,
            "project" => mk_core::types::KnowledgeLayer::Project,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        let entry = self.repo.get(ctx, layer, &p.path).await?;
        Ok(json!({ "success": true, "entry": entry }))
    }
}

pub struct KnowledgeListTool {
    repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
}

impl KnowledgeListTool {
    pub fn new(
        repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
    ) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Tool for KnowledgeListTool {
    fn name(&self) -> &str {
        "knowledge_list"
    }

    fn description(&self) -> &str {
        "List knowledge entries in a specific layer, optionally filtered by prefix."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "layer": { "type": "string" },
                "prefix": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeListParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "tenant" => mk_core::types::KnowledgeLayer::Tenant,
            "org" => mk_core::types::KnowledgeLayer::Org,
            "team" => mk_core::types::KnowledgeLayer::Team,
            "project" => mk_core::types::KnowledgeLayer::Project,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        let entries = self.repo.list(ctx, layer, &p.prefix).await?;
        Ok(json!({ "success": true, "entries": entries }))
    }
}

pub struct KnowledgeQueryTool {
    memory_manager: Arc<MemoryManager>,
    repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
}

impl KnowledgeQueryTool {
    pub fn new(
        memory_manager: Arc<MemoryManager>,
        repo: Arc<dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>>,
    ) -> Self {
        Self {
            memory_manager,
            repo,
        }
    }
}

#[async_trait]
impl Tool for KnowledgeQueryTool {
    fn name(&self) -> &str {
        "knowledge_query"
    }

    fn description(&self) -> &str {
        "Search for knowledge entries across layers using semantic or keyword search."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "layers": { "type": "array", "items": { "type": "string" } },
                "limit": { "type": "integer" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeQueryParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let mut layers = Vec::new();
        if p.layers.is_empty() {
            layers = vec![
                mk_core::types::KnowledgeLayer::Tenant,
                mk_core::types::KnowledgeLayer::Org,
                mk_core::types::KnowledgeLayer::Team,
                mk_core::types::KnowledgeLayer::Project,
            ];
        } else {
            for l in &p.layers {
                let layer = match l.to_lowercase().as_str() {
                    "tenant" => mk_core::types::KnowledgeLayer::Tenant,
                    "org" => mk_core::types::KnowledgeLayer::Org,
                    "team" => mk_core::types::KnowledgeLayer::Team,
                    "project" => mk_core::types::KnowledgeLayer::Project,
                    _ => continue,
                };
                layers.push(layer);
            }
        }

        let (vector_results, _trace) = self
            .memory_manager
            .search_text_with_reasoning(
                ctx.clone(),
                &p.query,
                p.limit.unwrap_or(10),
                0.7,
                std::collections::HashMap::new(),
                None,
            )
            .await
            .unwrap_or((Vec::new(), None));

        // --- Task 6.1/6.3: apply canonical precedence and attach relation context ---

        let mut repo_results = self
            .repo
            .search(ctx.clone(), &p.query, layers, p.limit.unwrap_or(10))
            .await?;

        // Sort by canonical-vs-residual precedence:
        //   1. Layer authority: Tenant(0) > Org(1) > Team(2) > Project(3)
        //   2. Variant-role precedence: Canonical(5) > Clarification(4) > Specialization(3) > Applicability(2) > Exception(1)
        //   3. Most-recently-updated first within same role+layer
        repo_results.sort_by(|a, b| {
            let la = knowledge_layer_authority(a.layer);
            let lb = knowledge_layer_authority(b.layer);
            if la != lb {
                return la.cmp(&lb);
            }
            let pa = a.variant_precedence();
            let pb = b.variant_precedence();
            if pa != pb {
                return pb.cmp(&pa);
            }
            b.updated_at.cmp(&a.updated_at)
        });

        // Enrich each keyword result with its semantic relations (task 6.3)
        let mut enriched_keyword: Vec<Value> = Vec::with_capacity(repo_results.len());
        for entry in repo_results {
            let relations = self
                .repo
                .get_relations_for_item(ctx.clone(), &entry.path)
                .await
                .unwrap_or_default();
            enriched_keyword.push(json!({
                "path": entry.path,
                "layer": entry.layer,
                "kind": entry.kind,
                "status": entry.status,
                "variantRole": entry.variant_role(),
                "content": entry.content,
                "metadata": entry.metadata,
                "updatedAt": entry.updated_at,
                "relations": relations,
            }));
        }

        Ok(json!({
            "success": true,
            "results": {
                "semantic": vector_results,
                "keyword": enriched_keyword
            }
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeProposal {
    pub proposal_id: String,
    pub draft_id: String,
    pub title: String,
    pub content: String,
    pub kind: mk_core::types::KnowledgeType,
    pub layer: mk_core::types::KnowledgeLayer,
    pub proposed_by: String,
    pub proposed_at: chrono::DateTime<chrono::Utc>,
    pub status: KnowledgeProposalStatus,
    pub approvers: Vec<String>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeProposalStatus {
    Draft,
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDraft {
    pub draft_id: String,
    pub title: String,
    pub description: String,
    pub content: String,
    pub kind: mk_core::types::KnowledgeType,
    pub layer: mk_core::types::KnowledgeLayer,
    pub status: KnowledgeDraftStatus,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeDraftStatus {
    Draft,
    Validated,
    Submitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterpretedKnowledge {
    pub kind: mk_core::types::KnowledgeType,
    pub title: String,
    pub summary: String,
    pub structure: KnowledgeStructure,
    pub suggested_layer: mk_core::types::KnowledgeLayer,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeStructure {
    pub context: Option<String>,
    pub decision: Option<String>,
    pub consequences: Option<String>,
    pub alternatives: Option<String>,
    pub pattern_description: Option<String>,
    pub applicability: Option<String>,
}

pub trait KnowledgeProposalStorage: Send + Sync {
    fn store_draft(
        &self,
        draft: KnowledgeDraft,
    ) -> impl std::future::Future<Output = Result<(), KnowledgeToolError>> + Send;

    fn get_draft(
        &self,
        draft_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<KnowledgeDraft>, KnowledgeToolError>> + Send;

    fn update_draft(
        &self,
        draft: KnowledgeDraft,
    ) -> impl std::future::Future<Output = Result<(), KnowledgeToolError>> + Send;

    fn store_proposal(
        &self,
        proposal: KnowledgeProposal,
    ) -> impl std::future::Future<Output = Result<(), KnowledgeToolError>> + Send;

    fn get_proposal(
        &self,
        proposal_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<KnowledgeProposal>, KnowledgeToolError>> + Send;

    fn list_pending(
        &self,
        layer: Option<mk_core::types::KnowledgeLayer>,
    ) -> impl std::future::Future<Output = Result<Vec<KnowledgeProposal>, KnowledgeToolError>> + Send;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum KnowledgeToolError {
    #[error("Draft not found: {0}")]
    DraftNotFound(String),

    #[error("Proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("Draft already submitted: {0}")]
    DraftAlreadySubmitted(String),

    #[error("Invalid knowledge type: {0}")]
    InvalidKnowledgeType(String),

    #[error("Invalid layer: {0}")]
    InvalidLayer(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Interpretation error: {0}")]
    InterpretationError(String),
}

pub trait KnowledgeInterpreter: Send + Sync {
    fn interpret(
        &self,
        description: &str,
        context: Option<&str>,
    ) -> impl std::future::Future<Output = Result<InterpretedKnowledge, KnowledgeToolError>> + Send;
}

pub struct SimpleKnowledgeInterpreter;

impl SimpleKnowledgeInterpreter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimpleKnowledgeInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl KnowledgeInterpreter for SimpleKnowledgeInterpreter {
    async fn interpret(
        &self,
        description: &str,
        _context: Option<&str>,
    ) -> Result<InterpretedKnowledge, KnowledgeToolError> {
        let lower = description.to_lowercase();

        let (kind, confidence) = if lower.contains("decide")
            || lower.contains("decision")
            || lower.contains("we should")
            || lower.contains("must use")
            || lower.contains("adopt")
            || lower.contains("choose")
        {
            (mk_core::types::KnowledgeType::Adr, 0.8)
        } else if lower.contains("pattern")
            || lower.contains("approach")
            || lower.contains("how to")
            || lower.contains("best practice")
        {
            (mk_core::types::KnowledgeType::Pattern, 0.75)
        } else if lower.contains("policy")
            || lower.contains("rule")
            || lower.contains("must not")
            || lower.contains("forbidden")
            || lower.contains("required")
        {
            (mk_core::types::KnowledgeType::Policy, 0.8)
        } else if lower.contains("spec")
            || lower.contains("specification")
            || lower.contains("requirement")
        {
            (mk_core::types::KnowledgeType::Spec, 0.7)
        } else {
            (mk_core::types::KnowledgeType::Adr, 0.5)
        };

        let suggested_layer = if lower.contains("tenant")
            || lower.contains("organization-wide")
            || lower.contains("all teams")
        {
            mk_core::types::KnowledgeLayer::Tenant
        } else if lower.contains("org") || lower.contains("department") {
            mk_core::types::KnowledgeLayer::Org
        } else if lower.contains("team") {
            mk_core::types::KnowledgeLayer::Team
        } else {
            mk_core::types::KnowledgeLayer::Project
        };

        let title = extract_title_from_description(description);
        let summary = if description.len() > 200 {
            format!("{}...", &description[..200])
        } else {
            description.to_string()
        };

        let structure = match kind {
            mk_core::types::KnowledgeType::Adr => KnowledgeStructure {
                context: Some("Context to be filled".to_string()),
                decision: Some(description.to_string()),
                consequences: Some("Consequences to be determined".to_string()),
                alternatives: Some("Alternatives to be documented".to_string()),
                pattern_description: None,
                applicability: None,
            },
            mk_core::types::KnowledgeType::Pattern => KnowledgeStructure {
                context: None,
                decision: None,
                consequences: None,
                alternatives: None,
                pattern_description: Some(description.to_string()),
                applicability: Some("Applicability to be defined".to_string()),
            },
            _ => KnowledgeStructure {
                context: Some(description.to_string()),
                decision: None,
                consequences: None,
                alternatives: None,
                pattern_description: None,
                applicability: None,
            },
        };

        Ok(InterpretedKnowledge {
            kind,
            title,
            summary,
            structure,
            suggested_layer,
            confidence,
        })
    }
}

fn extract_title_from_description(description: &str) -> String {
    let first_sentence = description
        .split(&['.', '!', '?'][..])
        .next()
        .unwrap_or(description)
        .trim();

    if first_sentence.len() > 80 {
        format!("{}...", &first_sentence[..77])
    } else {
        first_sentence.to_string()
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeProposeParams {
    pub description: String,
    #[serde(rename = "knowledgeType")]
    pub knowledge_type: Option<String>,
    pub layer: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "proposedBy")]
    pub proposed_by: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct KnowledgeProposeTool<S, I>
where
    S: KnowledgeProposalStorage,
    I: KnowledgeInterpreter,
{
    storage: Arc<S>,
    interpreter: Arc<I>,
}

impl<S, I> KnowledgeProposeTool<S, I>
where
    S: KnowledgeProposalStorage,
    I: KnowledgeInterpreter,
{
    pub fn new(storage: Arc<S>, interpreter: Arc<I>) -> Self {
        Self {
            storage,
            interpreter,
        }
    }

    pub async fn propose(
        &self,
        description: &str,
        knowledge_type: Option<&str>,
        layer: Option<&str>,
        title: Option<&str>,
        proposed_by: &str,
    ) -> Result<KnowledgeDraft, KnowledgeToolError> {
        let interpreted = self.interpreter.interpret(description, None).await?;

        let kind = if let Some(kt) = knowledge_type {
            parse_knowledge_type(kt)?
        } else {
            interpreted.kind
        };

        let layer = if let Some(l) = layer {
            parse_knowledge_layer(l)?
        } else {
            interpreted.suggested_layer
        };

        let title = title.map(|t| t.to_string()).unwrap_or(interpreted.title);

        let content = generate_draft_content(&kind, &title, &interpreted.structure);

        let draft = KnowledgeDraft {
            draft_id: uuid::Uuid::new_v4().to_string(),
            title,
            description: description.to_string(),
            content,
            kind,
            layer,
            status: KnowledgeDraftStatus::Draft,
            created_by: proposed_by.to_string(),
            created_at: chrono::Utc::now(),
        };

        self.storage.store_draft(draft.clone()).await?;

        Ok(draft)
    }
}

fn parse_knowledge_type(s: &str) -> Result<mk_core::types::KnowledgeType, KnowledgeToolError> {
    match s.to_lowercase().as_str() {
        "adr" => Ok(mk_core::types::KnowledgeType::Adr),
        "policy" => Ok(mk_core::types::KnowledgeType::Policy),
        "pattern" => Ok(mk_core::types::KnowledgeType::Pattern),
        "spec" => Ok(mk_core::types::KnowledgeType::Spec),
        "hindsight" => Ok(mk_core::types::KnowledgeType::Hindsight),
        _ => Err(KnowledgeToolError::InvalidKnowledgeType(s.to_string())),
    }
}

fn parse_knowledge_layer(s: &str) -> Result<mk_core::types::KnowledgeLayer, KnowledgeToolError> {
    match s.to_lowercase().as_str() {
        "tenant" => Ok(mk_core::types::KnowledgeLayer::Tenant),
        "org" => Ok(mk_core::types::KnowledgeLayer::Org),
        "team" => Ok(mk_core::types::KnowledgeLayer::Team),
        "project" => Ok(mk_core::types::KnowledgeLayer::Project),
        _ => Err(KnowledgeToolError::InvalidLayer(s.to_string())),
    }
}

fn generate_draft_content(
    kind: &mk_core::types::KnowledgeType,
    title: &str,
    structure: &KnowledgeStructure,
) -> String {
    match kind {
        mk_core::types::KnowledgeType::Adr => {
            format!(
                "# {}\n\n## Status\n\nProposed\n\n## Context\n\n{}\n\n## Decision\n\n{}\n\n## \
                 Consequences\n\n{}\n\n## Alternatives Considered\n\n{}",
                title,
                structure.context.as_deref().unwrap_or("_To be filled_"),
                structure.decision.as_deref().unwrap_or("_To be filled_"),
                structure
                    .consequences
                    .as_deref()
                    .unwrap_or("_To be determined_"),
                structure
                    .alternatives
                    .as_deref()
                    .unwrap_or("_To be documented_")
            )
        }
        mk_core::types::KnowledgeType::Pattern => {
            format!(
                "# Pattern: {}\n\n## Description\n\n{}\n\n## Applicability\n\n{}\n\n## \
                 Implementation\n\n_To be detailed_\n\n## Examples\n\n_To be provided_",
                title,
                structure
                    .pattern_description
                    .as_deref()
                    .unwrap_or("_To be described_"),
                structure
                    .applicability
                    .as_deref()
                    .unwrap_or("_To be defined_")
            )
        }
        mk_core::types::KnowledgeType::Policy => {
            format!(
                "# Policy: {}\n\n## Scope\n\n_To be defined_\n\n## Rules\n\n{}\n\n## \
                 Enforcement\n\n_To be specified_\n\n## Exceptions\n\n_None documented_",
                title,
                structure.context.as_deref().unwrap_or("_To be defined_")
            )
        }
        mk_core::types::KnowledgeType::Spec => {
            format!(
                "# Specification: {}\n\n## Overview\n\n{}\n\n## Requirements\n\n_To be \
                 detailed_\n\n## Acceptance Criteria\n\n_To be defined_",
                title,
                structure.context.as_deref().unwrap_or("_To be described_")
            )
        }
        mk_core::types::KnowledgeType::Hindsight => {
            format!(
                "# Hindsight: {}\n\n## What Happened\n\n{}\n\n## Lessons Learned\n\n_To be \
                 documented_\n\n## Recommendations\n\n_To be provided_",
                title,
                structure.context.as_deref().unwrap_or("_To be described_")
            )
        }
    }
}

#[async_trait]
impl<S, I> Tool for KnowledgeProposeTool<S, I>
where
    S: KnowledgeProposalStorage + 'static,
    I: KnowledgeInterpreter + 'static,
{
    fn name(&self) -> &str {
        "aeterna_knowledge_propose"
    }

    fn description(&self) -> &str {
        "Propose new knowledge (ADR, pattern, policy, spec) from natural language description."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Natural language description of the knowledge to propose"
                },
                "knowledgeType": {
                    "type": "string",
                    "enum": ["adr", "pattern", "policy", "spec", "hindsight"],
                    "description": "Optional: Override interpreted knowledge type"
                },
                "layer": {
                    "type": "string",
                    "enum": ["tenant", "org", "team", "project"],
                    "description": "Optional: Target layer for the knowledge"
                },
                "title": {
                    "type": "string",
                    "description": "Optional: Override auto-generated title"
                },
                "proposedBy": {
                    "type": "string",
                    "description": "User ID or email of the proposer"
                }
            },
            "required": ["description", "proposedBy"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeProposeParams = serde_json::from_value(params)?;
        p.validate()?;

        let draft = self
            .propose(
                &p.description,
                p.knowledge_type.as_deref(),
                p.layer.as_deref(),
                p.title.as_deref(),
                &p.proposed_by,
            )
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(json!({
            "success": true,
            "draft": {
                "draftId": draft.draft_id,
                "title": draft.title,
                "kind": format!("{:?}", draft.kind).to_lowercase(),
                "layer": format!("{:?}", draft.layer).to_lowercase(),
                "content": draft.content,
                "status": format!("{:?}", draft.status).to_lowercase()
            },
            "nextSteps": [
                "Review and edit the generated draft content",
                "Submit the draft for approval using the governance workflow",
                "Once approved, the knowledge will be added to the repository"
            ]
        }))
    }
}

pub struct InMemoryKnowledgeProposalStorage {
    drafts: tokio::sync::RwLock<std::collections::HashMap<String, KnowledgeDraft>>,
    proposals: tokio::sync::RwLock<std::collections::HashMap<String, KnowledgeProposal>>,
    pr_storage: Option<Arc<knowledge::pr_proposal_storage::PrProposalStorage>>,
    proposal_provider: String,
}

const DEFAULT_PROPOSAL_PROVIDER: &str = PROVIDER_GITHUB;

impl InMemoryKnowledgeProposalStorage {
    pub fn new() -> Self {
        Self {
            drafts: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            proposals: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            pr_storage: None,
            proposal_provider: DEFAULT_PROPOSAL_PROVIDER.to_string(),
        }
    }

    pub fn with_git_provider(
        git_provider: Arc<dyn knowledge::git_provider::GitProvider>,
        default_branch: impl Into<String>,
    ) -> Self {
        Self {
            drafts: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            proposals: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            pr_storage: Some(Arc::new(
                knowledge::pr_proposal_storage::PrProposalStorage::new(
                    git_provider,
                    default_branch.into(),
                ),
            )),
            proposal_provider: DEFAULT_PROPOSAL_PROVIDER.to_string(),
        }
    }

    pub fn with_optional_git_provider(
        git_provider: Option<Arc<dyn knowledge::git_provider::GitProvider>>,
        default_branch: impl Into<String>,
    ) -> Self {
        if let Some(provider) = git_provider {
            Self::with_git_provider(provider, default_branch)
        } else {
            Self::new()
        }
    }
}

impl Default for InMemoryKnowledgeProposalStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl KnowledgeProposalStorage for InMemoryKnowledgeProposalStorage {
    async fn store_draft(&self, draft: KnowledgeDraft) -> Result<(), KnowledgeToolError> {
        let mut drafts = self.drafts.write().await;
        drafts.insert(draft.draft_id.clone(), draft);
        Ok(())
    }

    async fn get_draft(
        &self,
        draft_id: &str,
    ) -> Result<Option<KnowledgeDraft>, KnowledgeToolError> {
        let drafts = self.drafts.read().await;
        Ok(drafts.get(draft_id).cloned())
    }

    async fn update_draft(&self, draft: KnowledgeDraft) -> Result<(), KnowledgeToolError> {
        let mut drafts = self.drafts.write().await;
        drafts.insert(draft.draft_id.clone(), draft);
        Ok(())
    }

    async fn store_proposal(&self, proposal: KnowledgeProposal) -> Result<(), KnowledgeToolError> {
        let mut enriched = proposal.clone();

        if let Some(pr_storage) = &self.pr_storage {
            let tenant_id = proposal
                .metadata
                .get("tenantId")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let layer = format!("{:?}", proposal.layer).to_lowercase();
            let path = format!("{}-{}.md", proposal.draft_id, proposal.proposal_id);
            let message = format!("propose knowledge: {}", proposal.title);

            let proposal_info = pr_storage
                .propose(tenant_id, &layer, &path, &proposal.content, &message)
                .await
                .map_err(|e| KnowledgeToolError::StorageError(e.to_string()))?;

            let pr = pr_storage
                .submit(&proposal_info, &proposal.title, None)
                .await
                .map_err(|e| KnowledgeToolError::StorageError(e.to_string()))?;

            enriched
                .metadata
                .insert("prNumber".to_string(), serde_json::json!(pr.number));
            enriched
                .metadata
                .insert("prUrl".to_string(), serde_json::json!(pr.html_url));
            enriched.metadata.insert(
                "prBranch".to_string(),
                serde_json::json!(proposal_info.branch_name),
            );
            enriched.metadata.insert(
                "prFilePath".to_string(),
                serde_json::json!(proposal_info.file_path),
            );
        }

        let mut proposals = self.proposals.write().await;
        proposals.insert(enriched.proposal_id.clone(), enriched);
        Ok(())
    }

    async fn get_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<Option<KnowledgeProposal>, KnowledgeToolError> {
        let proposals = self.proposals.read().await;
        Ok(proposals.get(proposal_id).cloned())
    }

    async fn list_pending(
        &self,
        layer: Option<mk_core::types::KnowledgeLayer>,
    ) -> Result<Vec<KnowledgeProposal>, KnowledgeToolError> {
        if let Some(pr_storage) = &self.pr_storage {
            let prs = pr_storage
                .list_pending()
                .await
                .map_err(|e| KnowledgeToolError::StorageError(e.to_string()))?;

            let mapped = prs
                .into_iter()
                .filter_map(|pr| {
                    let fallback_layer = mk_core::types::KnowledgeLayer::Project;
                    if layer.is_some_and(|l| l != fallback_layer) {
                        return None;
                    }

                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("prNumber".to_string(), serde_json::json!(pr.number));
                    metadata.insert("prUrl".to_string(), serde_json::json!(pr.html_url));

                    Some(KnowledgeProposal {
                        proposal_id: format!("pr-{}", pr.number),
                        draft_id: format!("pr-{}", pr.number),
                        title: pr.title,
                        content: pr.body.unwrap_or_default(),
                        kind: mk_core::types::KnowledgeType::Adr,
                        layer: fallback_layer,
                        proposed_by: self.proposal_provider.clone(),
                        proposed_at: chrono::Utc::now(),
                        status: KnowledgeProposalStatus::Pending,
                        approvers: Vec::new(),
                        metadata,
                    })
                })
                .collect();

            return Ok(mapped);
        }

        let proposals = self.proposals.read().await;
        let pending: Vec<_> = proposals
            .values()
            .filter(|p| {
                p.status == KnowledgeProposalStatus::Pending && layer.is_none_or(|l| p.layer == l)
            })
            .cloned()
            .collect();
        Ok(pending)
    }
}

pub trait GovernanceIntegration: Send + Sync {
    fn submit_for_approval(
        &self,
        draft: &KnowledgeDraft,
        justification: Option<&str>,
        notify: &[String],
    ) -> impl std::future::Future<Output = Result<String, KnowledgeToolError>> + Send;

    fn get_approval_status(
        &self,
        proposal_id: &str,
    ) -> impl std::future::Future<Output = Result<KnowledgeProposalStatus, KnowledgeToolError>> + Send;
}

#[allow(dead_code)]
pub struct SimpleGovernanceIntegration {
    required_approvals: u32,
    auto_approve_project_level: bool,
}

impl SimpleGovernanceIntegration {
    pub fn new() -> Self {
        Self {
            required_approvals: 1,
            auto_approve_project_level: true,
        }
    }

    pub fn with_config(required_approvals: u32, auto_approve_project_level: bool) -> Self {
        Self {
            required_approvals,
            auto_approve_project_level,
        }
    }
}

impl Default for SimpleGovernanceIntegration {
    fn default() -> Self {
        Self::new()
    }
}

impl GovernanceIntegration for SimpleGovernanceIntegration {
    async fn submit_for_approval(
        &self,
        draft: &KnowledgeDraft,
        _justification: Option<&str>,
        _notify: &[String],
    ) -> Result<String, KnowledgeToolError> {
        let proposal_id = uuid::Uuid::new_v4().to_string();

        if self.auto_approve_project_level && draft.layer == mk_core::types::KnowledgeLayer::Project
        {
            return Ok(proposal_id);
        }

        Ok(proposal_id)
    }

    async fn get_approval_status(
        &self,
        _proposal_id: &str,
    ) -> Result<KnowledgeProposalStatus, KnowledgeToolError> {
        Ok(KnowledgeProposalStatus::Pending)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeSubmitParams {
    #[serde(rename = "draftId")]
    pub draft_id: String,
    pub justification: Option<String>,
    #[serde(default)]
    pub notify: Vec<String>,
    #[serde(rename = "proposedBy")]
    pub proposed_by: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct KnowledgeProposalSubmitTool<S, G>
where
    S: KnowledgeProposalStorage,
    G: GovernanceIntegration,
{
    storage: Arc<S>,
    governance: Arc<G>,
}

impl<S, G> KnowledgeProposalSubmitTool<S, G>
where
    S: KnowledgeProposalStorage,
    G: GovernanceIntegration,
{
    pub fn new(storage: Arc<S>, governance: Arc<G>) -> Self {
        Self {
            storage,
            governance,
        }
    }

    pub async fn submit(
        &self,
        draft_id: &str,
        justification: Option<&str>,
        notify: &[String],
        proposed_by: &str,
    ) -> Result<KnowledgeProposal, KnowledgeToolError> {
        let draft = self
            .storage
            .get_draft(draft_id)
            .await?
            .ok_or_else(|| KnowledgeToolError::DraftNotFound(draft_id.to_string()))?;

        if draft.status == KnowledgeDraftStatus::Submitted {
            return Err(KnowledgeToolError::DraftAlreadySubmitted(
                draft_id.to_string(),
            ));
        }

        let proposal_id = self
            .governance
            .submit_for_approval(&draft, justification, notify)
            .await?;

        let status = self.governance.get_approval_status(&proposal_id).await?;

        let proposal = KnowledgeProposal {
            proposal_id: proposal_id.clone(),
            draft_id: draft_id.to_string(),
            title: draft.title.clone(),
            content: draft.content.clone(),
            kind: draft.kind,
            layer: draft.layer,
            proposed_by: proposed_by.to_string(),
            proposed_at: chrono::Utc::now(),
            status,
            approvers: notify.to_vec(),
            metadata: std::collections::HashMap::new(),
        };

        self.storage.store_proposal(proposal.clone()).await?;

        let mut updated_draft = draft;
        updated_draft.status = KnowledgeDraftStatus::Submitted;
        self.storage.update_draft(updated_draft).await?;

        Ok(proposal)
    }
}

#[async_trait]
impl<S, G> Tool for KnowledgeProposalSubmitTool<S, G>
where
    S: KnowledgeProposalStorage + 'static,
    G: GovernanceIntegration + 'static,
{
    fn name(&self) -> &str {
        "aeterna_knowledge_submit"
    }

    fn description(&self) -> &str {
        "Submit a knowledge draft for approval through the governance workflow."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draftId": {
                    "type": "string",
                    "description": "ID of the draft to submit"
                },
                "justification": {
                    "type": "string",
                    "description": "Reason for proposing this knowledge"
                },
                "notify": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional approvers to notify"
                },
                "proposedBy": {
                    "type": "string",
                    "description": "User ID or email of the proposer"
                }
            },
            "required": ["draftId", "proposedBy"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeSubmitParams = serde_json::from_value(params)?;
        p.validate()?;

        let proposal = self
            .submit(
                &p.draft_id,
                p.justification.as_deref(),
                &p.notify,
                &p.proposed_by,
            )
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let status_str = match proposal.status {
            KnowledgeProposalStatus::Draft => "draft",
            KnowledgeProposalStatus::Pending => "pending",
            KnowledgeProposalStatus::Approved => "approved",
            KnowledgeProposalStatus::Rejected => "rejected",
        };

        Ok(json!({
            "success": true,
            "proposal": {
                "proposalId": proposal.proposal_id,
                "draftId": proposal.draft_id,
                "title": proposal.title,
                "kind": format!("{:?}", proposal.kind).to_lowercase(),
                "layer": format!("{:?}", proposal.layer).to_lowercase(),
                "status": status_str,
                "proposedBy": proposal.proposed_by,
                "proposedAt": proposal.proposed_at.to_rfc3339()
            },
            "message": match proposal.status {
                KnowledgeProposalStatus::Approved => "Knowledge proposal auto-approved",
                KnowledgeProposalStatus::Pending => "Knowledge proposal submitted for approval",
                _ => "Knowledge proposal created"
            }
        }))
    }
}

pub struct KnowledgePendingListTool<S>
where
    S: KnowledgeProposalStorage,
{
    storage: Arc<S>,
}

impl<S: KnowledgeProposalStorage> KnowledgePendingListTool<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgePendingListParams {
    pub layer: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl<S: KnowledgeProposalStorage + 'static> Tool for KnowledgePendingListTool<S> {
    fn name(&self) -> &str {
        "aeterna_knowledge_pending"
    }

    fn description(&self) -> &str {
        "List pending knowledge proposals awaiting approval."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["tenant", "org", "team", "project"],
                    "description": "Filter by target layer"
                }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgePendingListParams = serde_json::from_value(params)?;
        p.validate()?;

        let layer = if let Some(l) = p.layer.as_ref() {
            Some(parse_knowledge_layer(l)?)
        } else {
            None
        };

        let pending = self
            .storage
            .list_pending(layer)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let proposals: Vec<_> = pending
            .iter()
            .map(|p| {
                json!({
                    "proposalId": p.proposal_id,
                    "title": p.title,
                    "kind": format!("{:?}", p.kind).to_lowercase(),
                    "layer": format!("{:?}", p.layer).to_lowercase(),
                    "proposedBy": p.proposed_by,
                    "proposedAt": p.proposed_at.to_rfc3339()
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "count": proposals.len(),
            "proposals": proposals
        }))
    }
}

pub struct KnowledgePromotionPreviewTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgePromotionPreviewTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePromotionPreviewParams {
    pub source_item_id: String,
    pub target_layer: String,
    pub mode: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgePromotionPreviewTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_promotion_preview"
    }

    fn description(&self) -> &str {
        "Preview a knowledge promotion split before creating a promotion request."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sourceItemId": { "type": "string" },
                "targetLayer": { "type": "string", "enum": ["tenant", "org", "team", "project"] },
                "mode": { "type": "string", "enum": ["full", "partial"] },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["sourceItemId", "targetLayer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgePromotionPreviewParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let target_layer = parse_knowledge_layer(&p.target_layer)?;
        let mode = parse_promotion_mode(p.mode.as_deref())?;
        let preview = self
            .manager
            .preview_promotion(ctx, &p.source_item_id, target_layer, mode)
            .await?;
        Ok(json!({ "success": true, "preview": preview }))
    }
}

pub struct KnowledgePromoteTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgePromoteTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePromoteParams {
    pub source_item_id: String,
    pub target_layer: String,
    pub mode: Option<String>,
    pub shared_content: String,
    pub residual_content: Option<String>,
    pub residual_role: Option<String>,
    pub justification: Option<String>,
    pub source_version: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgePromoteTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_promote"
    }

    fn description(&self) -> &str {
        "Create a governed knowledge promotion request."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sourceItemId": { "type": "string" },
                "targetLayer": { "type": "string", "enum": ["tenant", "org", "team", "project"] },
                "mode": { "type": "string", "enum": ["full", "partial"] },
                "sharedContent": { "type": "string" },
                "residualContent": { "type": "string" },
                "residualRole": { "type": "string", "enum": ["canonical", "specialization", "applicability", "exception", "clarification"] },
                "justification": { "type": "string" },
                "sourceVersion": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["sourceItemId", "targetLayer", "sharedContent"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgePromoteParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let existing = self
            .manager
            .repository_get(ctx.clone(), &p.source_item_id)
            .await?
            .ok_or_else(|| format!("Knowledge item not found: {}", p.source_item_id))?;

        let now = chrono::Utc::now().timestamp();
        let request = PromotionRequest {
            id: String::new(),
            source_item_id: p.source_item_id,
            source_layer: existing.layer,
            source_status: existing.status,
            target_layer: parse_knowledge_layer(&p.target_layer)?,
            promotion_mode: parse_promotion_mode(p.mode.as_deref())?,
            shared_content: p.shared_content,
            residual_content: p.residual_content,
            residual_role: parse_variant_role(p.residual_role.as_deref())?,
            justification: p.justification,
            status: PromotionRequestStatus::Draft,
            requested_by: ctx.user_id.clone(),
            tenant_id: ctx.tenant_id.clone(),
            source_version: p
                .source_version
                .unwrap_or_else(|| existing.commit_hash.unwrap_or_default()),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: now,
            updated_at: now,
        };

        let request = self.manager.create_promotion_request(ctx, request).await?;
        Ok(json!({ "success": true, "promotionRequest": request }))
    }
}

pub struct KnowledgeReviewPendingTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgeReviewPendingTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReviewPendingParams {
    pub status: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgeReviewPendingTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_review_pending"
    }

    fn description(&self) -> &str {
        "List pending or filtered knowledge promotion requests for review."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["draft", "pendingReview", "approved", "rejected", "applied", "cancelled"] },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeReviewPendingParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let status = parse_promotion_status(p.status.as_deref())?;
        let requests = self.manager.list_promotion_requests(ctx, status).await?;
        Ok(json!({ "success": true, "count": requests.len(), "requests": requests }))
    }
}

pub struct KnowledgeApproveTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgeApproveTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeApproveParams {
    pub promotion_id: String,
    pub decision: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgeApproveTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_approve"
    }

    fn description(&self) -> &str {
        "Approve a knowledge promotion request with a structured decision."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "promotionId": { "type": "string" },
                "decision": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["promotionId", "decision"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeApproveParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let decision = PromotionDecision::from_str(&p.decision)?;
        let request = self
            .manager
            .approve_promotion(ctx, &p.promotion_id, decision, None)
            .await?;
        Ok(json!({ "success": true, "promotionRequest": request }))
    }
}

pub struct KnowledgeRejectTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgeRejectTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRejectParams {
    pub promotion_id: String,
    pub reason: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgeRejectTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_reject"
    }

    fn description(&self) -> &str {
        "Reject a knowledge promotion request with a reason."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "promotionId": { "type": "string" },
                "reason": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["promotionId", "reason"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeRejectParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let request = self
            .manager
            .reject_promotion(ctx, &p.promotion_id, &p.reason, None)
            .await?;
        Ok(json!({ "success": true, "promotionRequest": request }))
    }
}

pub struct KnowledgeLinkTool {
    manager: Arc<KnowledgeManager>,
}

impl KnowledgeLinkTool {
    pub fn new(manager: Arc<KnowledgeManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeLinkParams {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for KnowledgeLinkTool {
    fn name(&self) -> &str {
        "aeterna_knowledge_link"
    }

    fn description(&self) -> &str {
        "Create an explicit semantic relation between two knowledge items."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sourceId": { "type": "string" },
                "targetId": { "type": "string" },
                "relationType": { "type": "string" },
                "metadata": { "type": "object" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["sourceId", "targetId", "relationType"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: KnowledgeLinkParams = serde_json::from_value(params)?;
        p.validate()?;
        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let relation = KnowledgeRelation {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: p.source_id,
            target_id: p.target_id,
            relation_type: KnowledgeRelationType::from_str(&p.relation_type)?,
            tenant_id: ctx.tenant_id.clone(),
            created_by: ctx.user_id.clone(),
            created_at: chrono::Utc::now().timestamp(),
            metadata: p.metadata,
        };
        let relation = self.manager.create_relation(ctx, relation).await?;
        Ok(json!({ "success": true, "relation": relation }))
    }
}

fn parse_promotion_mode(mode: Option<&str>) -> Result<PromotionMode, KnowledgeToolError> {
    match mode {
        None => Ok(PromotionMode::Partial),
        Some(value) => PromotionMode::from_str(value).map_err(|_| {
            KnowledgeToolError::StorageError(format!("Invalid promotion mode: {value}"))
        }),
    }
}

fn parse_variant_role(
    role: Option<&str>,
) -> Result<Option<KnowledgeVariantRole>, KnowledgeToolError> {
    match role {
        None => Ok(None),
        Some(value) => KnowledgeVariantRole::from_str(value)
            .map(Some)
            .map_err(|_| {
                KnowledgeToolError::StorageError(format!("Invalid residual role: {value}"))
            }),
    }
}

fn parse_promotion_status(
    status: Option<&str>,
) -> Result<Option<PromotionRequestStatus>, KnowledgeToolError> {
    match status {
        None => Ok(None),
        Some(value) => PromotionRequestStatus::from_str(value)
            .map(Some)
            .map_err(|_| {
                KnowledgeToolError::StorageError(format!("Invalid promotion status: {value}"))
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{TenantId, UserId};
    use std::str::FromStr;

    fn _test_ctx() -> TenantContext {
        TenantContext {
            tenant_id: TenantId::from_str("test-tenant").unwrap(),
            user_id: UserId::from_str("test-user").unwrap(),
            agent_id: None,
            roles: vec![],
            target_tenant_id: None,
        }
    }

    #[tokio::test]
    async fn test_simple_interpreter_detects_adr() {
        let interpreter = SimpleKnowledgeInterpreter::new();

        let result = interpreter
            .interpret("We should decide to use PostgreSQL for all databases", None)
            .await
            .unwrap();

        assert_eq!(result.kind, mk_core::types::KnowledgeType::Adr);
        assert!(result.confidence >= 0.7);
    }

    #[tokio::test]
    async fn test_simple_interpreter_detects_pattern() {
        let interpreter = SimpleKnowledgeInterpreter::new();

        let result = interpreter
            .interpret("Here is the best practice pattern for error handling", None)
            .await
            .unwrap();

        assert_eq!(result.kind, mk_core::types::KnowledgeType::Pattern);
    }

    #[tokio::test]
    async fn test_simple_interpreter_detects_policy() {
        let interpreter = SimpleKnowledgeInterpreter::new();

        let result = interpreter
            .interpret("This policy states that all code must not use eval()", None)
            .await
            .unwrap();

        assert_eq!(result.kind, mk_core::types::KnowledgeType::Policy);
    }

    #[tokio::test]
    async fn test_simple_interpreter_suggests_layer() {
        let interpreter = SimpleKnowledgeInterpreter::new();

        let result = interpreter
            .interpret("All teams must follow this tenant-wide standard", None)
            .await
            .unwrap();

        assert_eq!(
            result.suggested_layer,
            mk_core::types::KnowledgeLayer::Tenant
        );
    }

    #[tokio::test]
    async fn test_extract_title() {
        let title = extract_title_from_description(
            "Use PostgreSQL for databases. This is the recommended approach.",
        );
        assert_eq!(title, "Use PostgreSQL for databases");
    }

    #[tokio::test]
    async fn test_extract_title_truncates_long() {
        let long_desc = "This is a very long description that should be truncated because it \
                         exceeds the maximum allowed length for a title in our system";
        let title = extract_title_from_description(long_desc);
        assert!(title.len() <= 80);
        assert!(title.ends_with("..."));
    }

    #[tokio::test]
    async fn test_propose_creates_draft() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage.clone(), interpreter);

        let draft = tool
            .propose(
                "We should use GraphQL for all new APIs",
                None,
                None,
                None,
                "user@test.com",
            )
            .await
            .unwrap();

        assert!(!draft.draft_id.is_empty());
        assert_eq!(draft.kind, mk_core::types::KnowledgeType::Adr);
        assert!(draft.content.contains("GraphQL"));
        assert_eq!(draft.status, KnowledgeDraftStatus::Draft);
    }

    #[tokio::test]
    async fn test_propose_with_explicit_type() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage, interpreter);

        let draft = tool
            .propose(
                "Here is how we handle authentication",
                Some("pattern"),
                None,
                None,
                "user@test.com",
            )
            .await
            .unwrap();

        assert_eq!(draft.kind, mk_core::types::KnowledgeType::Pattern);
    }

    #[tokio::test]
    async fn test_propose_with_explicit_layer() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage, interpreter);

        let draft = tool
            .propose(
                "Team-specific coding standards",
                None,
                Some("team"),
                None,
                "user@test.com",
            )
            .await
            .unwrap();

        assert_eq!(draft.layer, mk_core::types::KnowledgeLayer::Team);
    }

    #[tokio::test]
    async fn test_propose_with_custom_title() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage, interpreter);

        let draft = tool
            .propose(
                "We should use GraphQL",
                None,
                None,
                Some("ADR-042: GraphQL for APIs"),
                "user@test.com",
            )
            .await
            .unwrap();

        assert_eq!(draft.title, "ADR-042: GraphQL for APIs");
    }

    #[tokio::test]
    async fn test_propose_stores_draft() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage.clone(), interpreter);

        let draft = tool
            .propose(
                "Use REST for external APIs",
                None,
                None,
                None,
                "user@test.com",
            )
            .await
            .unwrap();

        let stored = storage.get_draft(&draft.draft_id).await.unwrap();
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().draft_id, draft.draft_id);
    }

    #[tokio::test]
    async fn test_propose_invalid_knowledge_type() {
        let result = parse_knowledge_type("invalid");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_propose_invalid_layer() {
        let result = parse_knowledge_layer("invalid");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generate_adr_content() {
        let structure = KnowledgeStructure {
            context: Some("Need to choose a database".to_string()),
            decision: Some("Use PostgreSQL".to_string()),
            consequences: Some("Good performance".to_string()),
            alternatives: Some("MySQL, MongoDB".to_string()),
            pattern_description: None,
            applicability: None,
        };

        let content = generate_draft_content(
            &mk_core::types::KnowledgeType::Adr,
            "Database Selection",
            &structure,
        );

        assert!(content.contains("# Database Selection"));
        assert!(content.contains("## Context"));
        assert!(content.contains("## Decision"));
        assert!(content.contains("Use PostgreSQL"));
    }

    #[tokio::test]
    async fn test_generate_pattern_content() {
        let structure = KnowledgeStructure {
            context: None,
            decision: None,
            consequences: None,
            alternatives: None,
            pattern_description: Some("Circuit breaker for resilience".to_string()),
            applicability: Some("External service calls".to_string()),
        };

        let content = generate_draft_content(
            &mk_core::types::KnowledgeType::Pattern,
            "Circuit Breaker",
            &structure,
        );

        assert!(content.contains("# Pattern: Circuit Breaker"));
        assert!(content.contains("## Description"));
        assert!(content.contains("Circuit breaker for resilience"));
    }

    #[tokio::test]
    async fn test_tool_interface() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage, interpreter);

        assert_eq!(tool.name(), "aeterna_knowledge_propose");
        assert!(tool.description().contains("knowledge"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["description"].is_object());
        assert!(schema["properties"]["knowledgeType"].is_object());
        assert!(schema["properties"]["proposedBy"].is_object());
    }

    #[tokio::test]
    async fn test_tool_call() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let interpreter = Arc::new(SimpleKnowledgeInterpreter::new());
        let tool = KnowledgeProposeTool::new(storage, interpreter);

        let params = json!({
            "description": "We should document that all new APIs must use GraphQL",
            "proposedBy": "user@test.com"
        });

        let result = tool.call(params).await.unwrap();

        assert_eq!(result["success"], true);
        assert!(result["draft"]["draftId"].is_string());
        assert!(
            result["draft"]["content"]
                .as_str()
                .unwrap()
                .contains("GraphQL")
        );
        assert!(result["nextSteps"].is_array());
    }

    #[tokio::test]
    async fn test_in_memory_storage_operations() {
        let storage = InMemoryKnowledgeProposalStorage::new();

        let draft = KnowledgeDraft {
            draft_id: "draft-1".to_string(),
            title: "Test".to_string(),
            description: "Test description".to_string(),
            content: "Test content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };

        storage.store_draft(draft.clone()).await.unwrap();
        let retrieved = storage.get_draft("draft-1").await.unwrap();
        assert!(retrieved.is_some());

        let mut updated = draft.clone();
        updated.status = KnowledgeDraftStatus::Validated;
        storage.update_draft(updated).await.unwrap();

        let retrieved = storage.get_draft("draft-1").await.unwrap().unwrap();
        assert_eq!(retrieved.status, KnowledgeDraftStatus::Validated);
    }

    #[tokio::test]
    async fn test_proposal_storage_operations() {
        let storage = InMemoryKnowledgeProposalStorage::new();

        let proposal = KnowledgeProposal {
            proposal_id: "prop-1".to_string(),
            draft_id: "draft-1".to_string(),
            title: "Test Proposal".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            proposed_by: "user".to_string(),
            proposed_at: chrono::Utc::now(),
            status: KnowledgeProposalStatus::Pending,
            approvers: vec!["approver@test.com".to_string()],
            metadata: std::collections::HashMap::new(),
        };

        storage.store_proposal(proposal).await.unwrap();

        let pending = storage.list_pending(None).await.unwrap();
        assert_eq!(pending.len(), 1);

        let pending_project = storage
            .list_pending(Some(mk_core::types::KnowledgeLayer::Project))
            .await
            .unwrap();
        assert_eq!(pending_project.len(), 1);

        let pending_team = storage
            .list_pending(Some(mk_core::types::KnowledgeLayer::Team))
            .await
            .unwrap();
        assert_eq!(pending_team.len(), 0);
    }

    #[tokio::test]
    async fn test_submit_tool_interface() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage, governance);

        assert_eq!(tool.name(), "aeterna_knowledge_submit");
        assert!(tool.description().contains("approval"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["draftId"].is_object());
        assert!(schema["properties"]["proposedBy"].is_object());
    }

    #[tokio::test]
    async fn test_submit_creates_proposal() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage.clone(), governance);

        let draft = KnowledgeDraft {
            draft_id: "draft-submit".to_string(),
            title: "Test ADR".to_string(),
            description: "Test".to_string(),
            content: "# Test".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user@test.com".to_string(),
            created_at: chrono::Utc::now(),
        };
        storage.store_draft(draft).await.unwrap();

        let proposal = tool
            .submit(
                "draft-submit",
                Some("Important decision"),
                &[],
                "user@test.com",
            )
            .await
            .unwrap();

        assert!(!proposal.proposal_id.is_empty());
        assert_eq!(proposal.draft_id, "draft-submit");
        assert_eq!(proposal.title, "Test ADR");
    }

    #[tokio::test]
    async fn test_submit_marks_draft_as_submitted() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage.clone(), governance);

        let draft = KnowledgeDraft {
            draft_id: "draft-mark".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Team,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };
        storage.store_draft(draft).await.unwrap();

        tool.submit("draft-mark", None, &[], "user@test.com")
            .await
            .unwrap();

        let updated = storage.get_draft("draft-mark").await.unwrap().unwrap();
        assert_eq!(updated.status, KnowledgeDraftStatus::Submitted);
    }

    #[tokio::test]
    async fn test_submit_fails_for_missing_draft() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage, governance);

        let result = tool.submit("nonexistent", None, &[], "user@test.com").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_submit_fails_for_already_submitted() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage.clone(), governance);

        let draft = KnowledgeDraft {
            draft_id: "draft-already".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Submitted,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };
        storage.store_draft(draft).await.unwrap();

        let result = tool
            .submit("draft-already", None, &[], "user@test.com")
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already submitted")
        );
    }

    #[tokio::test]
    async fn test_submit_tool_call() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let governance = Arc::new(SimpleGovernanceIntegration::new());
        let tool = KnowledgeProposalSubmitTool::new(storage.clone(), governance);

        let draft = KnowledgeDraft {
            draft_id: "draft-call".to_string(),
            title: "API Standard".to_string(),
            description: "GraphQL standard".to_string(),
            content: "# API Standard".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };
        storage.store_draft(draft).await.unwrap();

        let params = json!({
            "draftId": "draft-call",
            "justification": "Important for API consistency",
            "proposedBy": "user@test.com"
        });

        let result = tool.call(params).await.unwrap();

        assert_eq!(result["success"], true);
        assert!(result["proposal"]["proposalId"].is_string());
        assert_eq!(result["proposal"]["title"], "API Standard");
    }

    #[tokio::test]
    async fn test_pending_list_tool_interface() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());
        let tool = KnowledgePendingListTool::new(storage);

        assert_eq!(tool.name(), "aeterna_knowledge_pending");
        assert!(tool.description().contains("pending"));
    }

    #[tokio::test]
    async fn test_pending_list_returns_proposals() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());

        let proposal = KnowledgeProposal {
            proposal_id: "prop-list".to_string(),
            draft_id: "draft-list".to_string(),
            title: "Pending ADR".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Team,
            proposed_by: "user@test.com".to_string(),
            proposed_at: chrono::Utc::now(),
            status: KnowledgeProposalStatus::Pending,
            approvers: vec![],
            metadata: std::collections::HashMap::new(),
        };
        storage.store_proposal(proposal).await.unwrap();

        let tool = KnowledgePendingListTool::new(storage);

        let params = json!({});
        let result = tool.call(params).await.unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["count"], 1);
        assert_eq!(result["proposals"][0]["title"], "Pending ADR");
    }

    #[tokio::test]
    async fn test_pending_list_filters_by_layer() {
        let storage = Arc::new(InMemoryKnowledgeProposalStorage::new());

        let team_proposal = KnowledgeProposal {
            proposal_id: "prop-team".to_string(),
            draft_id: "draft-team".to_string(),
            title: "Team ADR".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Team,
            proposed_by: "user".to_string(),
            proposed_at: chrono::Utc::now(),
            status: KnowledgeProposalStatus::Pending,
            approvers: vec![],
            metadata: std::collections::HashMap::new(),
        };

        let project_proposal = KnowledgeProposal {
            proposal_id: "prop-project".to_string(),
            draft_id: "draft-project".to_string(),
            title: "Project ADR".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            proposed_by: "user".to_string(),
            proposed_at: chrono::Utc::now(),
            status: KnowledgeProposalStatus::Pending,
            approvers: vec![],
            metadata: std::collections::HashMap::new(),
        };

        storage.store_proposal(team_proposal).await.unwrap();
        storage.store_proposal(project_proposal).await.unwrap();

        let tool = KnowledgePendingListTool::new(storage);

        let params = json!({ "layer": "team" });
        let result = tool.call(params).await.unwrap();

        assert_eq!(result["count"], 1);
        assert_eq!(result["proposals"][0]["title"], "Team ADR");
    }

    #[tokio::test]
    async fn test_parse_promotion_helpers() {
        assert_eq!(parse_promotion_mode(None).unwrap(), PromotionMode::Partial);
        assert_eq!(
            parse_promotion_mode(Some("full")).unwrap(),
            PromotionMode::Full
        );
        assert_eq!(
            parse_variant_role(Some("specialization")).unwrap(),
            Some(KnowledgeVariantRole::Specialization)
        );
        assert_eq!(
            parse_promotion_status(Some("pendingReview")).unwrap(),
            Some(PromotionRequestStatus::PendingReview)
        );
    }

    #[tokio::test]
    async fn test_simple_governance_integration() {
        let governance = SimpleGovernanceIntegration::new();

        let draft = KnowledgeDraft {
            draft_id: "draft-gov".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };

        let proposal_id = governance
            .submit_for_approval(
                &draft,
                Some("Test justification"),
                &["approver@test.com".to_string()],
            )
            .await
            .unwrap();

        assert!(!proposal_id.is_empty());
    }

    #[tokio::test]
    async fn test_governance_with_custom_config() {
        let governance = SimpleGovernanceIntegration::with_config(3, false);

        let draft = KnowledgeDraft {
            draft_id: "draft-config".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            content: "Content".to_string(),
            kind: mk_core::types::KnowledgeType::Adr,
            layer: mk_core::types::KnowledgeLayer::Project,
            status: KnowledgeDraftStatus::Draft,
            created_by: "user".to_string(),
            created_at: chrono::Utc::now(),
        };

        let proposal_id = governance
            .submit_for_approval(&draft, None, &[])
            .await
            .unwrap();

        assert!(!proposal_id.is_empty());
    }
}
