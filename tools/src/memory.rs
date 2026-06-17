use crate::tools::Tool;
use async_trait::async_trait;
use memory::manager::MemoryManager;
use mk_core::types::TenantContext;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use storage::graph::GraphStore;
use storage::graph_duckdb::DuckDbGraphStore;
use validator::Validate;

pub struct MemoryAddTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemoryAddTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryAddParams {
    pub content: String,
    pub layer: String,
    #[serde(default)]
    pub metadata: serde_json::Map<String, Value>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemorySearchParams {
    pub query: String,
    pub limit: Option<usize>,
    pub threshold: Option<f32>,
    #[serde(default)]
    pub filters: serde_json::Map<String, Value>,
    #[serde(rename = "contextSummary")]
    pub context_summary: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryDeleteParams {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    pub layer: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub enum CloseTarget {
    Session,
    Agent,
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryCloseParams {
    pub id: String,
    pub target: CloseTarget,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for MemoryAddTool {
    fn name(&self) -> &str {
        "memory_add"
    }

    fn description(&self) -> &str {
        "Store a piece of information in memory for future reference."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "layer": { "type": "string" },
                "metadata": { "type": "object" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["content", "layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryAddParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "agent" => mk_core::types::MemoryLayer::Agent,
            "user" => mk_core::types::MemoryLayer::User,
            "session" => mk_core::types::MemoryLayer::Session,
            "project" => mk_core::types::MemoryLayer::Project,
            "team" => mk_core::types::MemoryLayer::Team,
            "org" => mk_core::types::MemoryLayer::Org,
            "tenant" => mk_core::types::MemoryLayer::Tenant,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        let id = self.memory_manager.add(ctx, &p.content, layer).await?;
        Ok(json!({ "success": true, "memoryId": id }))
    }
}

pub struct MemorySearchTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search for memories across layers with optional reflective reasoning."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" },
                "threshold": { "type": "number" },
                "filters": { "type": "object" },
                "contextSummary": { "type": "string", "description": "Optional context for reasoning" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemorySearchParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let limit = p.limit.unwrap_or(10);
        let threshold = p.threshold.unwrap_or(0.0);
        let filters: std::collections::HashMap<String, Value> = p.filters.into_iter().collect();

        let (results, reasoning_trace) = self
            .memory_manager
            .search_text_with_reasoning(
                ctx,
                &p.query,
                limit,
                threshold,
                filters,
                p.context_summary.as_deref(),
            )
            .await?;

        let mut response = json!({
            "success": true,
            "results": results,
            "totalCount": results.len()
        });

        if let Some(trace) = reasoning_trace {
            response["reasoning"] = json!({
                "strategy": trace.strategy.to_string(),
                "refinedQuery": trace.refined_query,
                "thoughtProcess": trace.thought_process,
                "durationMs": trace.duration_ms,
                "timedOut": trace.timed_out
            });
        }

        Ok(response)
    }
}

pub struct MemoryDeleteTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemoryDeleteTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[async_trait]
impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete a memory from a specific layer."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "memoryId": { "type": "string" },
                "layer": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["memoryId", "layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryDeleteParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "agent" => mk_core::types::MemoryLayer::Agent,
            "user" => mk_core::types::MemoryLayer::User,
            "session" => mk_core::types::MemoryLayer::Session,
            "project" => mk_core::types::MemoryLayer::Project,
            "team" => mk_core::types::MemoryLayer::Team,
            "org" => mk_core::types::MemoryLayer::Org,
            "tenant" => mk_core::types::MemoryLayer::Tenant,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        self.memory_manager
            .delete_from_layer(ctx, layer, &p.memory_id)
            .await?;

        Ok(json!({
            "success": true,
            "message": "Memory deleted successfully"
        }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryReasonParams {
    pub query: String,
    #[serde(rename = "contextSummary")]
    pub context_summary: Option<String>,
}

pub struct MemoryReasonTool {
    reasoner: Arc<dyn memory::reasoning::ReflectiveReasoner>,
}

impl MemoryReasonTool {
    pub fn new(reasoner: Arc<dyn memory::reasoning::ReflectiveReasoner>) -> Self {
        Self { reasoner }
    }
}

#[async_trait]
impl Tool for MemoryReasonTool {
    fn name(&self) -> &str {
        "memory_reason"
    }

    fn description(&self) -> &str {
        "Perform reflective reasoning on a query to determine the best retrieval strategy."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "contextSummary": { "type": "string" }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryReasonParams = serde_json::from_value(params)?;
        p.validate()?;

        let trace = self
            .reasoner
            .reason(&p.query, p.context_summary.as_deref())
            .await?;

        Ok(serde_json::to_value(trace)?)
    }
}

pub struct MemoryCloseTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemoryCloseTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[async_trait]
impl Tool for MemoryCloseTool {
    fn name(&self) -> &str {
        "memory_close"
    }

    fn description(&self) -> &str {
        "Close a session or agent memory context."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "target": { "type": "string", "enum": ["session", "agent"] },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["id", "target"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryCloseParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        match p.target {
            CloseTarget::Session => self.memory_manager.close_session(ctx, &p.id).await?,
            CloseTarget::Agent => self.memory_manager.close_agent(ctx, &p.id).await?,
        }

        Ok(json!({
            "success": true,
            "message": format!("{:?} context closed successfully", p.target)
        }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryFeedbackParams {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    pub layer: String,
    #[serde(rename = "rewardType")]
    pub reward_type: String,
    pub score: f32,
    pub reasoning: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct MemoryFeedbackTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemoryFeedbackTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[async_trait]
impl Tool for MemoryFeedbackTool {
    fn name(&self) -> &str {
        "memory_feedback"
    }

    fn description(&self) -> &str {
        "Submit a reward signal for a retrieved memory."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "memoryId": { "type": "string" },
                "layer": { "type": "string" },
                "rewardType": { "type": "string", "enum": ["helpful", "irrelevant", "outdated", "inaccurate", "duplicate"] },
                "score": { "type": "number", "minimum": -1.0, "maximum": 1.0 },
                "reasoning": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["memoryId", "layer", "rewardType", "score"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryFeedbackParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "agent" => mk_core::types::MemoryLayer::Agent,
            "user" => mk_core::types::MemoryLayer::User,
            "session" => mk_core::types::MemoryLayer::Session,
            "project" => mk_core::types::MemoryLayer::Project,
            "team" => mk_core::types::MemoryLayer::Team,
            "org" => mk_core::types::MemoryLayer::Org,
            "tenant" => mk_core::types::MemoryLayer::Tenant,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        let reward_type = match p.reward_type.to_lowercase().as_str() {
            "helpful" => mk_core::types::RewardType::Helpful,
            "irrelevant" => mk_core::types::RewardType::Irrelevant,
            "outdated" => mk_core::types::RewardType::Outdated,
            "inaccurate" => mk_core::types::RewardType::Inaccurate,
            "duplicate" => mk_core::types::RewardType::Duplicate,
            _ => return Err(format!("Unknown reward type: {}", p.reward_type).into()),
        };

        let reward = mk_core::types::RewardSignal {
            reward_type,
            score: p.score,
            reasoning: p.reasoning,
            agent_id: ctx.agent_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        self.memory_manager
            .record_reward(ctx, layer, &p.memory_id, reward)
            .await?;

        Ok(json!({ "success": true, "message": "Reward recorded successfully" }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryOptimizeParams {
    pub layer: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct MemoryOptimizeTool {
    memory_manager: Arc<MemoryManager>,
}

impl MemoryOptimizeTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self { memory_manager }
    }
}

#[async_trait]
impl Tool for MemoryOptimizeTool {
    fn name(&self) -> &str {
        "memory_optimize"
    }

    fn description(&self) -> &str {
        "Manually trigger a pruning/compression cycle for a specific memory layer."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "layer": { "type": "string", "enum": ["agent", "user", "session", "project", "team", "org", "tenant"] },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryOptimizeParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let layer = match p.layer.to_lowercase().as_str() {
            "agent" => mk_core::types::MemoryLayer::Agent,
            "user" => mk_core::types::MemoryLayer::User,
            "session" => mk_core::types::MemoryLayer::Session,
            "project" => mk_core::types::MemoryLayer::Project,
            "team" => mk_core::types::MemoryLayer::Team,
            "org" => mk_core::types::MemoryLayer::Org,
            "tenant" => mk_core::types::MemoryLayer::Tenant,
            _ => return Err(format!("Unknown layer: {}", p.layer).into()),
        };

        self.memory_manager.optimize_layer(ctx, layer).await?;

        Ok(json!({ "success": true, "message": "Memory optimization complete" }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MemoryPromoteParams {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    #[serde(rename = "toLayer")]
    pub to_layer: String,
    pub reason: Option<String>,
    #[serde(default)]
    pub notify: Vec<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionResult {
    pub original_id: String,
    pub promoted_id: Option<String>,
    pub status: PromotionStatus,
    pub from_layer: String,
    pub to_layer: String,
    pub approval_required: bool,
    pub approvers_notified: Vec<String>,
    pub reason: Option<String>,
    pub proposal_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromotionStatus {
    Promoted,
    PendingApproval,
    Blocked,
    Failed,
}

pub trait PromotionGovernance: Send + Sync {
    fn requires_approval(
        &self,
        from_layer: mk_core::types::MemoryLayer,
        to_layer: mk_core::types::MemoryLayer,
    ) -> impl std::future::Future<Output = bool> + Send;

    fn get_approvers(
        &self,
        to_layer: mk_core::types::MemoryLayer,
        ctx: &TenantContext,
    ) -> impl std::future::Future<
        Output = Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>,
    > + Send;

    fn create_promotion_request(
        &self,
        memory_id: &str,
        from_layer: mk_core::types::MemoryLayer,
        to_layer: mk_core::types::MemoryLayer,
        reason: Option<String>,
        requestor: &str,
        approvers: Vec<String>,
    ) -> impl std::future::Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>> + Send;
}

pub struct DefaultPromotionGovernance;

impl DefaultPromotionGovernance {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultPromotionGovernance {
    fn default() -> Self {
        Self::new()
    }
}

impl PromotionGovernance for DefaultPromotionGovernance {
    async fn requires_approval(
        &self,
        from_layer: mk_core::types::MemoryLayer,
        to_layer: mk_core::types::MemoryLayer,
    ) -> bool {
        use mk_core::types::MemoryLayer;
        // GOVERNANCE RULE: Cross-scope promotions (project→team, team→org, org→tenant)
        // require approval; same-scope promotions (session→project, agent→user)
        // auto-approve
        matches!(
            (from_layer, to_layer),
            (MemoryLayer::Project, MemoryLayer::Team)
                | (MemoryLayer::Team, MemoryLayer::Org)
                | (MemoryLayer::Org, MemoryLayer::Tenant)
                | (MemoryLayer::User, MemoryLayer::Team)
        )
    }

    async fn get_approvers(
        &self,
        to_layer: mk_core::types::MemoryLayer,
        ctx: &TenantContext,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        use mk_core::types::MemoryLayer;
        let tenant = ctx.tenant_id.as_str();
        let approvers = match to_layer {
            MemoryLayer::Team => {
                vec![format!("tech-lead@{}.team", tenant)]
            }
            MemoryLayer::Org => {
                vec![format!("architect@{}.org", tenant)]
            }
            MemoryLayer::Tenant => {
                vec![format!("admin@{}.tenant", tenant)]
            }
            _ => vec![],
        };
        Ok(approvers)
    }

    async fn create_promotion_request(
        &self,
        memory_id: &str,
        _from_layer: mk_core::types::MemoryLayer,
        _to_layer: mk_core::types::MemoryLayer,
        _reason: Option<String>,
        _requestor: &str,
        _approvers: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(format!("promo-req-{}-{}", memory_id, uuid::Uuid::new_v4()))
    }
}

pub struct MemoryPromoteTool<G: PromotionGovernance> {
    memory_manager: Arc<MemoryManager>,
    governance: Arc<G>,
}

impl<G: PromotionGovernance> MemoryPromoteTool<G> {
    pub fn new(memory_manager: Arc<MemoryManager>, governance: Arc<G>) -> Self {
        Self {
            memory_manager,
            governance,
        }
    }

    fn parse_layer(layer: &str) -> Result<mk_core::types::MemoryLayer, String> {
        match layer.to_lowercase().as_str() {
            "agent" => Ok(mk_core::types::MemoryLayer::Agent),
            "user" => Ok(mk_core::types::MemoryLayer::User),
            "session" => Ok(mk_core::types::MemoryLayer::Session),
            "project" => Ok(mk_core::types::MemoryLayer::Project),
            "team" => Ok(mk_core::types::MemoryLayer::Team),
            "org" => Ok(mk_core::types::MemoryLayer::Org),
            "tenant" => Ok(mk_core::types::MemoryLayer::Tenant),
            _ => Err(format!("Unknown layer: {}", layer)),
        }
    }

    fn layer_to_string(layer: mk_core::types::MemoryLayer) -> String {
        match layer {
            mk_core::types::MemoryLayer::Agent => "agent".to_string(),
            mk_core::types::MemoryLayer::User => "user".to_string(),
            mk_core::types::MemoryLayer::Session => "session".to_string(),
            mk_core::types::MemoryLayer::Project => "project".to_string(),
            mk_core::types::MemoryLayer::Team => "team".to_string(),
            mk_core::types::MemoryLayer::Org => "org".to_string(),
            mk_core::types::MemoryLayer::Tenant => "tenant".to_string(),
        }
    }

    fn get_layer_hierarchy_level(layer: mk_core::types::MemoryLayer) -> u8 {
        match layer {
            mk_core::types::MemoryLayer::Agent => 0,
            mk_core::types::MemoryLayer::Session => 1,
            mk_core::types::MemoryLayer::User => 2,
            mk_core::types::MemoryLayer::Project => 3,
            mk_core::types::MemoryLayer::Team => 4,
            mk_core::types::MemoryLayer::Org => 5,
            mk_core::types::MemoryLayer::Tenant => 6,
        }
    }

    async fn promote(
        &self,
        ctx: TenantContext,
        memory_id: &str,
        to_layer: mk_core::types::MemoryLayer,
        reason: Option<String>,
        notify: Vec<String>,
    ) -> Result<PromotionResult, Box<dyn std::error::Error + Send + Sync>> {
        // Find the memory in any layer
        let (entry, from_layer) = self.find_memory(&ctx, memory_id).await?;

        // Validate promotion direction (can only promote UP the hierarchy)
        let from_level = Self::get_layer_hierarchy_level(from_layer);
        let to_level = Self::get_layer_hierarchy_level(to_layer);

        if to_level <= from_level {
            return Ok(PromotionResult {
                original_id: memory_id.to_string(),
                promoted_id: None,
                status: PromotionStatus::Blocked,
                from_layer: Self::layer_to_string(from_layer),
                to_layer: Self::layer_to_string(to_layer),
                approval_required: false,
                approvers_notified: vec![],
                reason: Some(format!(
                    "Cannot promote from {} to {} - can only promote to broader scopes",
                    Self::layer_to_string(from_layer),
                    Self::layer_to_string(to_layer)
                )),
                proposal_id: None,
            });
        }

        // Check if approval is required
        let requires_approval = self
            .governance
            .requires_approval(from_layer, to_layer)
            .await;

        if requires_approval {
            // Get approvers
            let mut approvers = self.governance.get_approvers(to_layer, &ctx).await?;

            // Add explicit notify list
            for n in notify {
                if !approvers.contains(&n) {
                    approvers.push(n);
                }
            }

            let requestor = ctx.user_id.as_str();
            let proposal_id = self
                .governance
                .create_promotion_request(
                    memory_id,
                    from_layer,
                    to_layer,
                    reason.clone(),
                    requestor,
                    approvers.clone(),
                )
                .await?;

            return Ok(PromotionResult {
                original_id: memory_id.to_string(),
                promoted_id: None,
                status: PromotionStatus::PendingApproval,
                from_layer: Self::layer_to_string(from_layer),
                to_layer: Self::layer_to_string(to_layer),
                approval_required: true,
                approvers_notified: approvers,
                reason,
                proposal_id: Some(proposal_id),
            });
        }

        // Auto-approve: perform the promotion
        let promoted_id = self
            .perform_promotion(&ctx, &entry, from_layer, to_layer, reason.clone())
            .await?;

        Ok(PromotionResult {
            original_id: memory_id.to_string(),
            promoted_id: Some(promoted_id),
            status: PromotionStatus::Promoted,
            from_layer: Self::layer_to_string(from_layer),
            to_layer: Self::layer_to_string(to_layer),
            approval_required: false,
            approvers_notified: vec![],
            reason,
            proposal_id: None,
        })
    }

    async fn find_memory(
        &self,
        ctx: &TenantContext,
        memory_id: &str,
    ) -> Result<
        (mk_core::types::MemoryEntry, mk_core::types::MemoryLayer),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        use mk_core::types::MemoryLayer;

        // Search layers from most specific to least specific
        let layers = [
            MemoryLayer::Agent,
            MemoryLayer::Session,
            MemoryLayer::User,
            MemoryLayer::Project,
            MemoryLayer::Team,
            MemoryLayer::Org,
            MemoryLayer::Tenant,
        ];

        for layer in layers {
            if let Ok(Some(entry)) = self
                .memory_manager
                .get_from_layer(ctx.clone(), layer, memory_id)
                .await
            {
                return Ok((entry, layer));
            }
        }

        Err(format!("Memory not found: {}", memory_id).into())
    }

    async fn perform_promotion(
        &self,
        ctx: &TenantContext,
        entry: &mk_core::types::MemoryEntry,
        from_layer: mk_core::types::MemoryLayer,
        to_layer: mk_core::types::MemoryLayer,
        reason: Option<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut promoted_entry = entry.clone();
        promoted_entry.id = format!("{}_promoted_{}", entry.id, uuid::Uuid::new_v4());
        promoted_entry.layer = to_layer;

        // Add promotion metadata
        promoted_entry.metadata.insert(
            "original_memory_id".to_string(),
            serde_json::json!(entry.id),
        );
        promoted_entry.metadata.insert(
            "promoted_from_layer".to_string(),
            serde_json::json!(Self::layer_to_string(from_layer)),
        );
        promoted_entry.metadata.insert(
            "promoted_at".to_string(),
            serde_json::json!(chrono::Utc::now().timestamp()),
        );
        if let Some(r) = reason {
            promoted_entry
                .metadata
                .insert("promotion_reason".to_string(), serde_json::json!(r));
        }

        let new_id = self
            .memory_manager
            .add_to_layer(ctx.clone(), to_layer, promoted_entry)
            .await?;

        Ok(new_id)
    }
}

#[async_trait]
impl<G: PromotionGovernance + 'static> Tool for MemoryPromoteTool<G> {
    fn name(&self) -> &str {
        "aeterna_memory_promote"
    }

    fn description(&self) -> &str {
        "Promote a memory to a broader scope with governance approval if required. Cross-scope \
         promotions (e.g., project→team, team→org) require approval from layer leads."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "memoryId": {
                    "type": "string",
                    "description": "ID of the memory to promote"
                },
                "toLayer": {
                    "type": "string",
                    "enum": ["user", "project", "team", "org", "tenant"],
                    "description": "Target layer to promote to (must be broader than current)"
                },
                "reason": {
                    "type": "string",
                    "description": "Reason for promoting this memory to broader scope"
                },
                "notify": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional people to notify about this promotion"
                },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["memoryId", "toLayer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryPromoteParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let to_layer = Self::parse_layer(&p.to_layer)?;

        let result = self
            .promote(ctx, &p.memory_id, to_layer, p.reason, p.notify)
            .await?;

        Ok(serde_json::to_value(result)?)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAutoPromoteParams {
    pub layer: String,
    #[serde(default)]
    pub threshold: Option<f32>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPromoteResult {
    pub layer: String,
    pub evaluated_count: usize,
    pub promoted_count: usize,
    pub promoted_memories: Vec<AutoPromotedMemory>,
    pub threshold_used: f32,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPromotedMemory {
    pub original_id: String,
    pub promoted_id: Option<String>,
    pub score: f32,
    pub from_layer: String,
    pub to_layer: String,
}

pub struct MemoryAutoPromoteTool {
    memory_manager: Arc<MemoryManager>,
    default_threshold: f32,
}

impl MemoryAutoPromoteTool {
    pub fn new(memory_manager: Arc<MemoryManager>) -> Self {
        Self {
            memory_manager,
            default_threshold: 0.7,
        }
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.default_threshold = threshold;
        self
    }

    fn parse_layer(layer: &str) -> Result<mk_core::types::MemoryLayer, String> {
        match layer.to_lowercase().as_str() {
            "agent" => Ok(mk_core::types::MemoryLayer::Agent),
            "user" => Ok(mk_core::types::MemoryLayer::User),
            "session" => Ok(mk_core::types::MemoryLayer::Session),
            "project" => Ok(mk_core::types::MemoryLayer::Project),
            "team" => Ok(mk_core::types::MemoryLayer::Team),
            "org" => Ok(mk_core::types::MemoryLayer::Org),
            "tenant" => Ok(mk_core::types::MemoryLayer::Tenant),
            _ => Err(format!("Unknown layer: {}", layer)),
        }
    }

    fn layer_to_string(layer: mk_core::types::MemoryLayer) -> String {
        match layer {
            mk_core::types::MemoryLayer::Agent => "agent".to_string(),
            mk_core::types::MemoryLayer::User => "user".to_string(),
            mk_core::types::MemoryLayer::Session => "session".to_string(),
            mk_core::types::MemoryLayer::Project => "project".to_string(),
            mk_core::types::MemoryLayer::Team => "team".to_string(),
            mk_core::types::MemoryLayer::Org => "org".to_string(),
            mk_core::types::MemoryLayer::Tenant => "tenant".to_string(),
        }
    }

    fn determine_target_layer(
        current_layer: mk_core::types::MemoryLayer,
    ) -> Option<mk_core::types::MemoryLayer> {
        use mk_core::types::MemoryLayer;
        match current_layer {
            MemoryLayer::Agent => Some(MemoryLayer::User),
            MemoryLayer::Session => Some(MemoryLayer::Project),
            MemoryLayer::User => Some(MemoryLayer::Team),
            MemoryLayer::Project => Some(MemoryLayer::Team),
            MemoryLayer::Team => Some(MemoryLayer::Org),
            MemoryLayer::Org => Some(MemoryLayer::Tenant),
            MemoryLayer::Tenant => None,
        }
    }

    fn calculate_importance_score(entry: &mk_core::types::MemoryEntry) -> f32 {
        let explicit_score = entry
            .metadata
            .get("score")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.0);

        let reward = entry
            .metadata
            .get("reward")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.0);

        let access_count = entry
            .metadata
            .get("access_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as f32;

        let last_accessed = entry
            .metadata
            .get("last_accessed_at")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp()) as f32;

        let now_ts = chrono::Utc::now().timestamp() as f32;
        let days_since_last_access = (now_ts - last_accessed).max(0.0) / 86400.0;
        let recency_score = (1.0f32 - days_since_last_access * 0.1)
            .max(0.0f32)
            .min(1.0f32);

        let frequency_score = (access_count / 10.0).min(1.0);

        let base_score = explicit_score.max(reward);

        (base_score * 0.5) + (frequency_score * 0.3) + (recency_score * 0.2)
    }

    async fn auto_promote(
        &self,
        ctx: TenantContext,
        layer: mk_core::types::MemoryLayer,
        threshold: f32,
        dry_run: bool,
    ) -> Result<AutoPromoteResult, Box<dyn std::error::Error + Send + Sync>> {
        let entries = self
            .memory_manager
            .list_all_from_layer(ctx.clone(), layer)
            .await?;

        let mut promoted_memories = Vec::new();
        let target_layer = Self::determine_target_layer(layer);

        for entry in &entries {
            let score = Self::calculate_importance_score(entry);

            if score >= threshold
                && let Some(target) = target_layer
            {
                let promoted_id = if dry_run {
                    None
                } else {
                    let mut promoted_entry = entry.clone();
                    promoted_entry.id =
                        format!("{}_auto_promoted_{}", entry.id, uuid::Uuid::new_v4());
                    promoted_entry.layer = target;
                    promoted_entry.metadata.insert(
                        "original_memory_id".to_string(),
                        serde_json::json!(entry.id),
                    );
                    promoted_entry.metadata.insert(
                        "auto_promoted_at".to_string(),
                        serde_json::json!(chrono::Utc::now().timestamp()),
                    );
                    promoted_entry
                        .metadata
                        .insert("promotion_score".to_string(), serde_json::json!(score));
                    promoted_entry.metadata.insert(
                        "promoted_from_layer".to_string(),
                        serde_json::json!(Self::layer_to_string(layer)),
                    );

                    let new_id = self
                        .memory_manager
                        .add_to_layer(ctx.clone(), target, promoted_entry)
                        .await?;
                    Some(new_id)
                };

                promoted_memories.push(AutoPromotedMemory {
                    original_id: entry.id.clone(),
                    promoted_id,
                    score,
                    from_layer: Self::layer_to_string(layer),
                    to_layer: Self::layer_to_string(target),
                });
            }
        }

        Ok(AutoPromoteResult {
            layer: Self::layer_to_string(layer),
            evaluated_count: entries.len(),
            promoted_count: promoted_memories.len(),
            promoted_memories,
            threshold_used: threshold,
            dry_run,
        })
    }
}

#[async_trait]
impl Tool for MemoryAutoPromoteTool {
    fn name(&self) -> &str {
        "aeterna_memory_auto_promote"
    }

    fn description(&self) -> &str {
        "Automatically evaluate and promote memories based on reward threshold. Memories with high \
         scores (based on reward, access frequency, recency) are promoted to the next broader \
         layer. Use dry_run=true to preview without making changes."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["agent", "session", "user", "project", "team", "org"],
                    "description": "Source layer to evaluate for auto-promotion"
                },
                "threshold": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "default": 0.7,
                    "description": "Minimum score threshold for promotion (0.0-1.0)"
                },
                "dryRun": {
                    "type": "boolean",
                    "default": false,
                    "description": "Preview promotions without making changes"
                },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["layer"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MemoryAutoPromoteParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let layer = Self::parse_layer(&p.layer)?;
        let threshold = p.threshold.unwrap_or(self.default_threshold);

        let result = self.auto_promote(ctx, layer, threshold, p.dry_run).await?;

        Ok(serde_json::to_value(result)?)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GraphQueryParams {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct GraphQueryTool {
    graph_store: Arc<DuckDbGraphStore>,
}

impl GraphQueryTool {
    pub fn new(graph_store: Arc<DuckDbGraphStore>) -> Self {
        Self { graph_store }
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn name(&self) -> &str {
        "graph_query"
    }

    fn description(&self) -> &str {
        "Search the knowledge graph for nodes matching a query."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query for finding nodes" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10 },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GraphQueryParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let limit = p.limit.unwrap_or(10);

        let nodes = self.graph_store.search_nodes(ctx, &p.query, limit).await?;

        Ok(json!({
            "success": true,
            "results": nodes,
            "totalCount": nodes.len()
        }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GraphNeighborsParams {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub depth: Option<usize>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct GraphNeighborsTool {
    graph_store: Arc<DuckDbGraphStore>,
}

impl GraphNeighborsTool {
    pub fn new(graph_store: Arc<DuckDbGraphStore>) -> Self {
        Self { graph_store }
    }
}

#[async_trait]
impl Tool for GraphNeighborsTool {
    fn name(&self) -> &str {
        "graph_neighbors"
    }

    fn description(&self) -> &str {
        "Find neighboring nodes connected to a given node in the knowledge graph."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "nodeId": { "type": "string", "description": "ID of the node to find neighbors for" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 1, "description": "Traversal depth (hops)" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["nodeId"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GraphNeighborsParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let _depth = p.depth.unwrap_or(1);

        let neighbors = self.graph_store.get_neighbors(ctx, &p.node_id).await?;

        let results: Vec<Value> = neighbors
            .into_iter()
            .map(|(edge, node)| {
                json!({
                    "edge": edge,
                    "node": node
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "nodeId": p.node_id,
            "neighbors": results,
            "count": results.len()
        }))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GraphPathParams {
    #[serde(rename = "sourceId")]
    pub source_id: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
    #[serde(rename = "maxHops")]
    pub max_hops: Option<usize>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

pub struct GraphPathTool {
    graph_store: Arc<DuckDbGraphStore>,
}

impl GraphPathTool {
    pub fn new(graph_store: Arc<DuckDbGraphStore>) -> Self {
        Self { graph_store }
    }
}

#[async_trait]
impl Tool for GraphPathTool {
    fn name(&self) -> &str {
        "graph_path"
    }

    fn description(&self) -> &str {
        "Find the shortest path between two nodes in the knowledge graph."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sourceId": { "type": "string", "description": "ID of the starting node" },
                "targetId": { "type": "string", "description": "ID of the target node" },
                "maxHops": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5, "description": "Maximum path length" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["sourceId", "targetId"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GraphPathParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let max_hops = p.max_hops.unwrap_or(5);

        let path = self
            .graph_store
            .find_path(ctx, &p.source_id, &p.target_id, max_hops)
            .await?;

        let found = !path.is_empty();

        Ok(json!({
            "success": true,
            "found": found,
            "sourceId": p.source_id,
            "targetId": p.target_id,
            "path": path,
            "hops": path.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::manager::MemoryManager;
    use memory::providers::MockProvider;
    use mk_core::types::{MemoryEntry, MemoryLayer, TenantContext, TenantId, UserId};
    use std::collections::HashMap;
    use std::str::FromStr;

    fn test_ctx() -> TenantContext {
        TenantContext {
            tenant_id: TenantId::from_str("test-tenant").unwrap(),
            user_id: UserId::from_str("test-user").unwrap(),
            agent_id: None,
            roles: vec![],
            target_tenant_id: None,
        }
    }

    fn create_test_entry(id: &str, layer: MemoryLayer) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: format!("Test memory content for {}", id),
            embedding: None,
            layer,
            metadata: HashMap::new(),
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
            summaries: HashMap::new(),
            context_vector: None,
            importance_score: None,
        }
    }

    async fn setup_manager_with_providers(layers: &[MemoryLayer]) -> Arc<MemoryManager> {
        let manager = Arc::new(MemoryManager::new());
        for &layer in layers {
            let provider: Arc<
                dyn mk_core::traits::MemoryProviderAdapter<
                        Error = Box<dyn std::error::Error + Send + Sync>,
                    > + Send
                    + Sync,
            > = Arc::new(MockProvider::new());
            manager.register_provider(layer, provider).await;
        }
        manager
    }

    struct TestGovernance {
        requires_approval_for_cross_scope: bool,
    }

    impl TestGovernance {
        fn new(requires_approval: bool) -> Self {
            Self {
                requires_approval_for_cross_scope: requires_approval,
            }
        }
    }

    impl PromotionGovernance for TestGovernance {
        async fn requires_approval(&self, from_layer: MemoryLayer, to_layer: MemoryLayer) -> bool {
            if !self.requires_approval_for_cross_scope {
                return false;
            }
            matches!(
                (from_layer, to_layer),
                (MemoryLayer::Project, MemoryLayer::Team)
                    | (MemoryLayer::Team, MemoryLayer::Org)
                    | (MemoryLayer::Org, MemoryLayer::Tenant)
                    | (MemoryLayer::User, MemoryLayer::Team)
            )
        }

        async fn get_approvers(
            &self,
            to_layer: MemoryLayer,
            _ctx: &TenantContext,
        ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
            let approvers = match to_layer {
                MemoryLayer::Team => vec!["tech-lead@test.team".to_string()],
                MemoryLayer::Org => vec!["architect@test.org".to_string()],
                MemoryLayer::Tenant => vec!["admin@test.tenant".to_string()],
                _ => vec![],
            };
            Ok(approvers)
        }

        async fn create_promotion_request(
            &self,
            memory_id: &str,
            _from_layer: MemoryLayer,
            _to_layer: MemoryLayer,
            _reason: Option<String>,
            _requestor: &str,
            _approvers: Vec<String>,
        ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            Ok(format!("test-proposal-{}", memory_id))
        }
    }

    #[tokio::test]
    async fn test_promote_auto_approves_same_scope() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_session_1", MemoryLayer::Session);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(true));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "mem_session_1",
                MemoryLayer::Project,
                Some("Test promotion".to_string()),
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(result.status, PromotionStatus::Promoted);
        assert!(result.promoted_id.is_some());
        assert!(!result.approval_required);
        assert_eq!(result.from_layer, "session");
        assert_eq!(result.to_layer, "project");
    }

    #[tokio::test]
    async fn test_promote_requires_approval_cross_scope() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Project, MemoryLayer::Team]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_project_1", MemoryLayer::Project);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Project, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(true));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "mem_project_1",
                MemoryLayer::Team,
                Some("Important for team".to_string()),
                vec!["extra-reviewer@test.com".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(result.status, PromotionStatus::PendingApproval);
        assert!(result.promoted_id.is_none());
        assert!(result.approval_required);
        assert!(result.proposal_id.is_some());
        assert!(
            result
                .approvers_notified
                .contains(&"tech-lead@test.team".to_string())
        );
        assert!(
            result
                .approvers_notified
                .contains(&"extra-reviewer@test.com".to_string())
        );
        assert_eq!(result.from_layer, "project");
        assert_eq!(result.to_layer, "team");
    }

    #[tokio::test]
    async fn test_promote_blocked_wrong_direction() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Team, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_team_1", MemoryLayer::Team);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Team, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "mem_team_1",
                MemoryLayer::Project,
                None,
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(result.status, PromotionStatus::Blocked);
        assert!(result.promoted_id.is_none());
        assert!(!result.approval_required);
        assert!(
            result
                .reason
                .unwrap()
                .contains("can only promote to broader scopes")
        );
    }

    #[tokio::test]
    async fn test_promote_blocked_same_level() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_project_2", MemoryLayer::Project);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Project, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "mem_project_2",
                MemoryLayer::Project,
                None,
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(result.status, PromotionStatus::Blocked);
    }

    #[tokio::test]
    async fn test_promote_memory_not_found() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "nonexistent_memory",
                MemoryLayer::Project,
                None,
                vec![],
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Memory not found"));
    }

    #[tokio::test]
    async fn test_tool_interface() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::User, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_user_1", MemoryLayer::User);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        assert_eq!(tool.name(), "aeterna_memory_promote");
        assert!(tool.description().contains("Promote a memory"));

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["memoryId"].is_object());
        assert!(schema["properties"]["toLayer"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("memoryId"))
        );
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("toLayer"))
        );
    }

    #[tokio::test]
    async fn test_tool_call_auto_approve() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Agent, MemoryLayer::User]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_agent_1", MemoryLayer::Agent);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Agent, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let params = json!({
            "memoryId": "mem_agent_1",
            "toLayer": "user",
            "reason": "Useful for user-level context",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result["status"], "promoted");
        assert!(result["promotedId"].as_str().is_some());
        assert_eq!(result["fromLayer"], "agent");
        assert_eq!(result["toLayer"], "user");
    }

    #[tokio::test]
    async fn test_default_governance_requires_approval() {
        let governance = DefaultPromotionGovernance::new();

        assert!(
            governance
                .requires_approval(MemoryLayer::Project, MemoryLayer::Team)
                .await
        );
        assert!(
            governance
                .requires_approval(MemoryLayer::Team, MemoryLayer::Org)
                .await
        );
        assert!(
            governance
                .requires_approval(MemoryLayer::Org, MemoryLayer::Tenant)
                .await
        );
        assert!(
            governance
                .requires_approval(MemoryLayer::User, MemoryLayer::Team)
                .await
        );

        assert!(
            !governance
                .requires_approval(MemoryLayer::Session, MemoryLayer::Project)
                .await
        );
        assert!(
            !governance
                .requires_approval(MemoryLayer::Agent, MemoryLayer::User)
                .await
        );
    }

    #[tokio::test]
    async fn test_default_governance_get_approvers() {
        let governance = DefaultPromotionGovernance::new();
        let ctx = test_ctx();

        let team_approvers = governance
            .get_approvers(MemoryLayer::Team, &ctx)
            .await
            .unwrap();
        assert!(team_approvers[0].contains("tech-lead"));

        let org_approvers = governance
            .get_approvers(MemoryLayer::Org, &ctx)
            .await
            .unwrap();
        assert!(org_approvers[0].contains("architect"));

        let tenant_approvers = governance
            .get_approvers(MemoryLayer::Tenant, &ctx)
            .await
            .unwrap();
        assert!(tenant_approvers[0].contains("admin"));
    }

    #[tokio::test]
    async fn test_promotion_preserves_metadata() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let mut entry = create_test_entry("mem_with_meta", MemoryLayer::Session);
        entry
            .metadata
            .insert("custom_key".to_string(), json!("custom_value"));
        entry.metadata.insert("importance".to_string(), json!(0.95));
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, entry)
            .await
            .unwrap();

        let governance = Arc::new(TestGovernance::new(false));
        let tool = MemoryPromoteTool::new(manager.clone(), governance);

        let result = tool
            .promote(
                ctx.clone(),
                "mem_with_meta",
                MemoryLayer::Project,
                Some("Important memory".to_string()),
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(result.status, PromotionStatus::Promoted);
        let promoted_id = result.promoted_id.unwrap();

        let promoted = manager
            .get_from_layer(ctx.clone(), MemoryLayer::Project, &promoted_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(promoted.metadata.get("custom_key").unwrap(), "custom_value");
        assert_eq!(promoted.metadata.get("importance").unwrap(), &json!(0.95));
        assert_eq!(
            promoted.metadata.get("original_memory_id").unwrap(),
            "mem_with_meta"
        );
        assert_eq!(
            promoted.metadata.get("promoted_from_layer").unwrap(),
            "session"
        );
        assert!(promoted.metadata.contains_key("promoted_at"));
        assert_eq!(
            promoted.metadata.get("promotion_reason").unwrap(),
            "Important memory"
        );
    }

    #[tokio::test]
    async fn test_layer_hierarchy_parsing() {
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("agent").unwrap(),
            MemoryLayer::Agent
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("USER").unwrap(),
            MemoryLayer::User
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("Session").unwrap(),
            MemoryLayer::Session
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("project").unwrap(),
            MemoryLayer::Project
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("TEAM").unwrap(),
            MemoryLayer::Team
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("Org").unwrap(),
            MemoryLayer::Org
        );
        assert_eq!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("tenant").unwrap(),
            MemoryLayer::Tenant
        );

        assert!(MemoryPromoteTool::<DefaultPromotionGovernance>::parse_layer("invalid").is_err());
    }

    #[tokio::test]
    async fn test_hierarchy_level_ordering() {
        assert!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Agent
            ) < MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Session
            )
        );
        assert!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Session
            ) < MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::User
            )
        );
        assert!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Project
            ) < MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Team
            )
        );
        assert!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Team
            ) < MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Org
            )
        );
        assert!(
            MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Org
            ) < MemoryPromoteTool::<DefaultPromotionGovernance>::get_layer_hierarchy_level(
                MemoryLayer::Tenant
            )
        );
    }

    #[tokio::test]
    async fn test_promotion_result_serialization() {
        let result = PromotionResult {
            original_id: "test-mem".to_string(),
            promoted_id: Some("test-mem-promoted-123".to_string()),
            status: PromotionStatus::Promoted,
            from_layer: "session".to_string(),
            to_layer: "project".to_string(),
            approval_required: false,
            approvers_notified: vec![],
            reason: Some("Test reason".to_string()),
            proposal_id: None,
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["originalId"], "test-mem");
        assert_eq!(json["promotedId"], "test-mem-promoted-123");
        assert_eq!(json["status"], "promoted");
        assert_eq!(json["fromLayer"], "session");
        assert_eq!(json["toLayer"], "project");
        assert_eq!(json["approvalRequired"], false);
    }

    #[tokio::test]
    async fn test_memory_add_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryAddTool::new(manager);

        assert_eq!(tool.name(), "memory_add");
        assert!(tool.description().contains("Store"));

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["content"].is_object());
        assert!(schema["properties"]["layer"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("content"))
        );
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("layer"))
        );
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "llm-integration"),
        ignore = "requires LLM/embedding services"
    )]
    async fn test_memory_add_tool_call() {
        let manager = setup_manager_with_providers(&[MemoryLayer::User]).await;
        let tool = MemoryAddTool::new(manager);

        let params = json!({
            "content": "Test memory content",
            "layer": "user",
            "metadata": {"tag": "important"},
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        match result {
            Ok(result) => {
                assert_eq!(result["success"], true);
                assert!(result["memoryId"].as_str().is_some());
            }
            Err(err) => {
                let message = err.to_string();
                assert!(
                    message.contains("Embedding service not configured")
                        || message.contains("LLM service required")
                        || message.contains("Knowledge manager not configured"),
                    "unexpected configuration error: {message}"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_memory_add_invalid_layer() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryAddTool::new(manager);

        let params = json!({
            "content": "Test memory content",
            "layer": "invalid_layer",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown layer"));
    }

    #[tokio::test]
    async fn test_memory_delete_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryDeleteTool::new(manager);

        assert_eq!(tool.name(), "memory_delete");
        assert!(tool.description().contains("Delete"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["memoryId"].is_object());
        assert!(schema["properties"]["layer"].is_object());
    }

    #[tokio::test]
    async fn test_memory_delete_tool_call() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_to_delete", MemoryLayer::Project);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Project, entry)
            .await
            .unwrap();

        let tool = MemoryDeleteTool::new(manager.clone());
        let params = json!({
            "memoryId": "mem_to_delete",
            "layer": "project",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result["success"], true);

        let retrieved = manager
            .get_from_layer(ctx, MemoryLayer::Project, "mem_to_delete")
            .await
            .unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_memory_close_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryCloseTool::new(manager);

        assert_eq!(tool.name(), "memory_close");
        assert!(tool.description().contains("Close"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["id"].is_object());
        assert!(schema["properties"]["target"].is_object());
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "llm-integration"),
        ignore = "requires LLM service configuration"
    )]
    async fn test_memory_close_session() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryCloseTool::new(manager);

        let params = json!({
            "id": "session-123",
            "target": "session",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("LLM service required")
                || message.contains("Embedding service not configured")
                || message.contains("Knowledge manager not configured"),
            "unexpected configuration error: {message}"
        );
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "llm-integration"),
        ignore = "requires LLM service configuration"
    )]
    async fn test_memory_close_agent() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Agent]).await;
        let tool = MemoryCloseTool::new(manager);

        let params = json!({
            "id": "agent-456",
            "target": "agent",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("LLM service required")
                || message.contains("Embedding service not configured")
                || message.contains("Knowledge manager not configured"),
            "unexpected configuration error: {message}"
        );
    }

    #[tokio::test]
    async fn test_memory_feedback_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryFeedbackTool::new(manager);

        assert_eq!(tool.name(), "memory_feedback");
        assert!(tool.description().contains("reward"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["memoryId"].is_object());
        assert!(schema["properties"]["rewardType"].is_object());
        assert!(schema["properties"]["score"].is_object());
    }

    #[tokio::test]
    async fn test_memory_feedback_helpful() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_feedback", MemoryLayer::Session);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, entry)
            .await
            .unwrap();

        let tool = MemoryFeedbackTool::new(manager);
        let params = json!({
            "memoryId": "mem_feedback",
            "layer": "session",
            "rewardType": "helpful",
            "score": 0.9,
            "reasoning": "Very useful information",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result["success"], true);
    }

    #[tokio::test]
    async fn test_memory_feedback_irrelevant() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry("mem_irrelevant", MemoryLayer::Project);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Project, entry)
            .await
            .unwrap();

        let tool = MemoryFeedbackTool::new(manager);
        let params = json!({
            "memoryId": "mem_irrelevant",
            "layer": "project",
            "rewardType": "irrelevant",
            "score": -0.5,
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result["success"], true);
    }

    #[tokio::test]
    async fn test_memory_feedback_invalid_reward_type() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryFeedbackTool::new(manager);

        let params = json!({
            "memoryId": "mem_invalid",
            "layer": "session",
            "rewardType": "invalid_type",
            "score": 0.5,
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown reward type")
        );
    }

    #[tokio::test]
    async fn test_memory_optimize_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryOptimizeTool::new(manager);

        assert_eq!(tool.name(), "memory_optimize");
        assert!(tool.description().contains("pruning"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["layer"].is_object());
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "llm-integration"),
        ignore = "requires LLM service configuration"
    )]
    async fn test_memory_optimize_tool_call() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryOptimizeTool::new(manager);

        let params = json!({
            "layer": "session",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("LLM service required")
                || message.contains("Embedding service not configured")
                || message.contains("Knowledge manager not configured"),
            "unexpected configuration error: {message}"
        );
    }

    #[tokio::test]
    async fn test_memory_optimize_invalid_layer() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryOptimizeTool::new(manager);

        let params = json!({
            "layer": "invalid",
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_memory_search_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemorySearchTool::new(manager);

        assert_eq!(tool.name(), "memory_search");
        assert!(tool.description().contains("Search"));

        let schema = tool.input_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["limit"].is_object());
        assert!(schema["properties"]["threshold"].is_object());
    }

    fn create_test_entry_with_score(id: &str, layer: MemoryLayer, score: f32) -> MemoryEntry {
        let mut entry = create_test_entry(id, layer);
        entry.metadata.insert("score".to_string(), json!(score));
        entry.metadata.insert("access_count".to_string(), json!(5));
        entry.metadata.insert(
            "last_accessed_at".to_string(),
            json!(chrono::Utc::now().timestamp()),
        );
        entry
    }

    #[tokio::test]
    async fn test_auto_promote_above_threshold() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let high_score_entry = create_test_entry_with_score("mem_high", MemoryLayer::Session, 0.9);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, high_score_entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Session, 0.7, false)
            .await
            .unwrap();

        assert_eq!(result.layer, "session");
        assert_eq!(result.evaluated_count, 1);
        assert_eq!(result.promoted_count, 1);
        assert!(!result.dry_run);
        assert_eq!(result.promoted_memories.len(), 1);
        assert_eq!(result.promoted_memories[0].original_id, "mem_high");
        assert!(result.promoted_memories[0].promoted_id.is_some());
        assert_eq!(result.promoted_memories[0].from_layer, "session");
        assert_eq!(result.promoted_memories[0].to_layer, "project");
    }

    #[tokio::test]
    async fn test_auto_promote_below_threshold() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let low_score_entry = create_test_entry_with_score("mem_low", MemoryLayer::Session, 0.3);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, low_score_entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Session, 0.7, false)
            .await
            .unwrap();

        assert_eq!(result.evaluated_count, 1);
        assert_eq!(result.promoted_count, 0);
        assert!(result.promoted_memories.is_empty());
    }

    #[tokio::test]
    async fn test_auto_promote_dry_run() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let high_score_entry = create_test_entry_with_score("mem_dry", MemoryLayer::Session, 0.9);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, high_score_entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Session, 0.7, true)
            .await
            .unwrap();

        assert!(result.dry_run);
        assert_eq!(result.promoted_count, 1);
        assert!(result.promoted_memories[0].promoted_id.is_none());

        let project_entries = manager
            .list_all_from_layer(ctx.clone(), MemoryLayer::Project)
            .await
            .unwrap();
        assert!(project_entries.is_empty());
    }

    #[tokio::test]
    async fn test_auto_promote_multiple_memories() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Agent, MemoryLayer::User]).await;
        let ctx = test_ctx();

        let entry1 = create_test_entry_with_score("mem_1", MemoryLayer::Agent, 0.8);
        let entry2 = create_test_entry_with_score("mem_2", MemoryLayer::Agent, 0.9);
        let entry3 = create_test_entry_with_score("mem_3", MemoryLayer::Agent, 0.4);

        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Agent, entry1)
            .await
            .unwrap();
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Agent, entry2)
            .await
            .unwrap();
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Agent, entry3)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Agent, 0.7, false)
            .await
            .unwrap();

        assert_eq!(result.evaluated_count, 3);
        assert_eq!(result.promoted_count, 2);
    }

    #[tokio::test]
    async fn test_auto_promote_tool_interface() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Session]).await;
        let tool = MemoryAutoPromoteTool::new(manager);

        assert_eq!(tool.name(), "aeterna_memory_auto_promote");
        assert!(tool.description().contains("Automatically"));

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["layer"].is_object());
        assert!(schema["properties"]["threshold"].is_object());
        assert!(schema["properties"]["dryRun"].is_object());
    }

    #[tokio::test]
    async fn test_auto_promote_tool_call() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry_with_score("mem_call", MemoryLayer::Session, 0.85);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager);
        let params = json!({
            "layer": "session",
            "threshold": 0.5,
            "dryRun": false,
            "tenantContext": {
                "tenant_id": "test-tenant",
                "user_id": "test-user"
            }
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result["layer"], "session");
        assert_eq!(result["evaluatedCount"], 1);
        assert_eq!(result["promotedCount"], 1);
        assert_eq!(result["dryRun"], false);
    }

    #[tokio::test]
    async fn test_auto_promote_with_reward_metadata() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Project, MemoryLayer::Team]).await;
        let ctx = test_ctx();

        let mut entry = create_test_entry("mem_reward", MemoryLayer::Project);
        entry.metadata.insert("reward".to_string(), json!(0.95));
        entry.metadata.insert("access_count".to_string(), json!(20));

        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Project, entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Project, 0.6, false)
            .await
            .unwrap();

        assert_eq!(result.promoted_count, 1);
        assert!(result.promoted_memories[0].score >= 0.6);
    }

    #[tokio::test]
    async fn test_auto_promote_preserves_promotion_metadata() {
        let manager =
            setup_manager_with_providers(&[MemoryLayer::Session, MemoryLayer::Project]).await;
        let ctx = test_ctx();

        let entry = create_test_entry_with_score("mem_meta", MemoryLayer::Session, 0.9);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Session, entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager.clone());
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Session, 0.7, false)
            .await
            .unwrap();

        let promoted_id = result.promoted_memories[0].promoted_id.as_ref().unwrap();
        let promoted = manager
            .get_from_layer(ctx.clone(), MemoryLayer::Project, promoted_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            promoted.metadata.get("original_memory_id").unwrap(),
            "mem_meta"
        );
        assert!(promoted.metadata.contains_key("auto_promoted_at"));
        assert!(promoted.metadata.contains_key("promotion_score"));
        assert_eq!(
            promoted.metadata.get("promoted_from_layer").unwrap(),
            "session"
        );
    }

    #[tokio::test]
    async fn test_auto_promote_no_target_for_tenant() {
        let manager = setup_manager_with_providers(&[MemoryLayer::Tenant]).await;
        let ctx = test_ctx();

        let entry = create_test_entry_with_score("mem_tenant", MemoryLayer::Tenant, 0.95);
        manager
            .add_to_layer(ctx.clone(), MemoryLayer::Tenant, entry)
            .await
            .unwrap();

        let tool = MemoryAutoPromoteTool::new(manager);
        let result = tool
            .auto_promote(ctx.clone(), MemoryLayer::Tenant, 0.7, false)
            .await
            .unwrap();

        assert_eq!(result.evaluated_count, 1);
        assert_eq!(result.promoted_count, 0);
    }

    #[tokio::test]
    async fn test_calculate_importance_score() {
        let mut entry = create_test_entry("score_test", MemoryLayer::Session);

        entry.metadata.insert("score".to_string(), json!(0.8));
        entry.metadata.insert("access_count".to_string(), json!(10));
        entry.metadata.insert(
            "last_accessed_at".to_string(),
            json!(chrono::Utc::now().timestamp()),
        );

        let score = MemoryAutoPromoteTool::calculate_importance_score(&entry);
        assert!(score > 0.5);
        assert!(score <= 1.0);
    }

    #[tokio::test]
    async fn test_determine_target_layer() {
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Agent),
            Some(MemoryLayer::User)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Session),
            Some(MemoryLayer::Project)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::User),
            Some(MemoryLayer::Team)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Project),
            Some(MemoryLayer::Team)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Team),
            Some(MemoryLayer::Org)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Org),
            Some(MemoryLayer::Tenant)
        );
        assert_eq!(
            MemoryAutoPromoteTool::determine_target_layer(MemoryLayer::Tenant),
            None
        );
    }

    #[tokio::test]
    async fn test_auto_promote_result_serialization() {
        let result = AutoPromoteResult {
            layer: "session".to_string(),
            evaluated_count: 10,
            promoted_count: 3,
            promoted_memories: vec![AutoPromotedMemory {
                original_id: "mem_1".to_string(),
                promoted_id: Some("mem_1_promoted".to_string()),
                score: 0.85,
                from_layer: "session".to_string(),
                to_layer: "project".to_string(),
            }],
            threshold_used: 0.7,
            dry_run: false,
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["layer"], "session");
        assert_eq!(json["evaluatedCount"], 10);
        assert_eq!(json["promotedCount"], 3);
        assert!((json["thresholdUsed"].as_f64().unwrap() - 0.7).abs() < 0.01);
        assert_eq!(json["dryRun"], false);
        assert!(json["promotedMemories"].is_array());
    }
}
