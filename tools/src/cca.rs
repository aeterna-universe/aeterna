use crate::tools::Tool;
use async_trait::async_trait;
use knowledge::context_architect::{ContextAssembler, SummarySource};
use knowledge::hindsight::HindsightQuery;
use knowledge::meta_agent::MetaAgentLoopState;
use knowledge::note_taking::{TrajectoryCapture, TrajectoryEvent};
use mk_core::types::{ErrorSignature, HindsightNote, MemoryLayer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use validator::Validate;

pub struct ContextAssembleTool {
    assembler: Arc<ContextAssembler>,
    sources_provider: Arc<dyn SummarySourceProvider + Send + Sync>,
}

pub trait SummarySourceProvider: Send + Sync {
    fn get_sources(&self, layers: &[MemoryLayer]) -> Vec<SummarySource>;
}

pub struct DefaultSummarySourceProvider;

impl SummarySourceProvider for DefaultSummarySourceProvider {
    fn get_sources(&self, _layers: &[MemoryLayer]) -> Vec<SummarySource> {
        Vec::new()
    }
}

impl ContextAssembleTool {
    pub fn new(
        assembler: Arc<ContextAssembler>,
        sources_provider: Arc<dyn SummarySourceProvider + Send + Sync>,
    ) -> Self {
        Self {
            assembler,
            sources_provider,
        }
    }

    pub fn with_default_provider(assembler: Arc<ContextAssembler>) -> Self {
        Self::new(assembler, Arc::new(DefaultSummarySourceProvider))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct ContextAssembleParams {
    #[serde(rename = "tenantContext")]
    pub tenant_context: mk_core::types::TenantContext,

    #[serde(default)]
    pub query: Option<String>,

    #[serde(rename = "tokenBudget", default = "default_token_budget")]
    pub token_budget: u32,

    #[serde(default)]
    pub layers: Vec<String>,
}

fn default_token_budget() -> u32 {
    4000
}

fn parse_layer(s: &str) -> Option<MemoryLayer> {
    match s.to_lowercase().as_str() {
        "agent" => Some(MemoryLayer::Agent),
        "user" => Some(MemoryLayer::User),
        "session" => Some(MemoryLayer::Session),
        "project" => Some(MemoryLayer::Project),
        "team" => Some(MemoryLayer::Team),
        "org" => Some(MemoryLayer::Org),
        "tenant" => Some(MemoryLayer::Tenant),
        _ => None,
    }
}

#[async_trait]
impl Tool for ContextAssembleTool {
    fn name(&self) -> &str {
        "context_assemble"
    }

    fn description(&self) -> &str {
        "Assemble hierarchical context from memory layers using CCA Context Architect."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenantContext": { "$ref": "#/definitions/TenantContext" },
                "query": { "type": "string" },
                "tokenBudget": { "type": "integer", "minimum": 100, "maximum": 32000 },
                "layers": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["tenantContext"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: ContextAssembleParams = serde_json::from_value(params)?;
        p.validate()?;

        let layers: Vec<MemoryLayer> = p.layers.iter().filter_map(|s| parse_layer(s)).collect();
        let sources = self.sources_provider.get_sources(&layers);
        let context = self
            .assembler
            .assemble_context(None, &sources, Some(p.token_budget));

        Ok(json!({
            "success": true,
            "context": {
                "totalTokens": context.total_tokens,
                "tokenBudget": context.token_budget,
                "layersIncluded": context.layers_included.iter().map(|l| format!("{:?}", l)).collect::<Vec<_>>(),
                "isWithinBudget": context.is_within_budget(),
                "entryCount": context.entries.len(),
                "content": context.content()
            }
        }))
    }
}

pub struct NoteCaptureTool {
    capture: Arc<RwLock<TrajectoryCapture>>,
}

impl NoteCaptureTool {
    pub fn new(capture: Arc<RwLock<TrajectoryCapture>>) -> Self {
        Self { capture }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct NoteCaptureParams {
    #[serde(rename = "tenantContext")]
    pub tenant_context: mk_core::types::TenantContext,

    pub description: String,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(rename = "toolName", default = "default_tool_name")]
    pub tool_name: String,

    #[serde(default)]
    pub success: bool,
}

fn default_tool_name() -> String {
    "manual_capture".to_string()
}

#[async_trait]
impl Tool for NoteCaptureTool {
    fn name(&self) -> &str {
        "note_capture"
    }

    fn description(&self) -> &str {
        "Capture a trajectory event for note distillation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenantContext": { "$ref": "#/definitions/TenantContext" },
                "description": { "type": "string" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "toolName": { "type": "string" },
                "success": { "type": "boolean" }
            },
            "required": ["tenantContext", "description"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: NoteCaptureParams = serde_json::from_value(params)?;
        p.validate()?;

        let event = TrajectoryEvent::new(&p.tool_name, &p.description, "", p.success, 0)
            .with_metadata(json!({ "tags": p.tags }));

        let mut capture = self.capture.write().map_err(|e| e.to_string())?;
        capture.capture(event);
        let event_count = capture.len();

        Ok(json!({
            "success": true,
            "message": format!("Trajectory event captured: {}", p.description),
            "eventCount": event_count
        }))
    }
}

pub struct HindsightQueryTool {
    query_engine: Arc<HindsightQuery>,
    notes_provider: Arc<dyn HindsightNotesProvider + Send + Sync>,
}

pub trait HindsightNotesProvider: Send + Sync {
    fn get_notes(&self) -> Vec<HindsightNote>;
}

pub struct DefaultHindsightNotesProvider;

impl HindsightNotesProvider for DefaultHindsightNotesProvider {
    fn get_notes(&self) -> Vec<HindsightNote> {
        Vec::new()
    }
}

impl HindsightQueryTool {
    pub fn new(
        query_engine: Arc<HindsightQuery>,
        notes_provider: Arc<dyn HindsightNotesProvider + Send + Sync>,
    ) -> Self {
        Self {
            query_engine,
            notes_provider,
        }
    }

    pub fn with_default_provider(query_engine: Arc<HindsightQuery>) -> Self {
        Self::new(query_engine, Arc::new(DefaultHindsightNotesProvider))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct HindsightQueryParams {
    #[serde(rename = "tenantContext")]
    pub tenant_context: mk_core::types::TenantContext,

    #[serde(rename = "errorType")]
    pub error_type: String,

    #[serde(rename = "messagePattern")]
    pub message_pattern: String,

    #[serde(rename = "contextPatterns", default)]
    pub context_patterns: Vec<String>,
}

#[async_trait]
impl Tool for HindsightQueryTool {
    fn name(&self) -> &str {
        "hindsight_query"
    }

    fn description(&self) -> &str {
        "Query hindsight learning for error patterns and resolutions."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenantContext": { "$ref": "#/definitions/TenantContext" },
                "errorType": { "type": "string" },
                "messagePattern": { "type": "string" },
                "contextPatterns": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["tenantContext", "errorType", "messagePattern"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: HindsightQueryParams = serde_json::from_value(params)?;
        p.validate()?;

        let error_sig = ErrorSignature {
            error_type: p.error_type.clone(),
            message_pattern: p.message_pattern.clone(),
            stack_patterns: vec![],
            context_patterns: p.context_patterns.clone(),
            embedding: None,
        };

        let notes = self.notes_provider.get_notes();
        let matches = self.query_engine.query_hindsight(&error_sig, &notes);

        let results: Vec<Value> = matches
            .iter()
            .map(|m| {
                json!({
                    "noteId": m.note_id,
                    "score": m.score,
                    "content": m.note.content,
                    "resolution": m.best_resolution.as_ref().map(|r| json!({
                        "description": r.description,
                        "successRate": r.success_rate,
                        "applicationCount": r.application_count
                    }))
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "matchCount": results.len(),
            "matches": results
        }))
    }
}

pub struct MetaLoopStatusTool {
    state_provider: Arc<dyn MetaLoopStateProvider + Send + Sync>,
}

pub trait MetaLoopStateProvider: Send + Sync {
    fn get_state(&self, loop_id: Option<&str>) -> Option<MetaAgentLoopState>;
    fn active_loop_count(&self) -> usize;
}

pub struct DefaultMetaLoopStateProvider;

impl MetaLoopStateProvider for DefaultMetaLoopStateProvider {
    fn get_state(&self, _loop_id: Option<&str>) -> Option<MetaAgentLoopState> {
        None
    }

    fn active_loop_count(&self) -> usize {
        0
    }
}

impl MetaLoopStatusTool {
    pub fn new(state_provider: Arc<dyn MetaLoopStateProvider + Send + Sync>) -> Self {
        Self { state_provider }
    }

    pub fn with_default_provider() -> Self {
        Self::new(Arc::new(DefaultMetaLoopStateProvider))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct MetaLoopStatusParams {
    #[serde(rename = "tenantContext")]
    pub tenant_context: mk_core::types::TenantContext,

    #[serde(rename = "loopId", default)]
    pub loop_id: Option<String>,

    #[serde(rename = "includeDetails", default)]
    pub include_details: bool,
}

#[async_trait]
impl Tool for MetaLoopStatusTool {
    fn name(&self) -> &str {
        "meta_loop_status"
    }

    fn description(&self) -> &str {
        "Get status of meta-agent build-test-improve loops."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenantContext": { "$ref": "#/definitions/TenantContext" },
                "loopId": { "type": "string" },
                "includeDetails": { "type": "boolean" }
            },
            "required": ["tenantContext"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: MetaLoopStatusParams = serde_json::from_value(params)?;
        p.validate()?;

        let active_count = self.state_provider.active_loop_count();
        let state = self.state_provider.get_state(p.loop_id.as_deref());

        let status = if active_count > 0 { "running" } else { "idle" };

        let details = if p.include_details {
            state.as_ref().map(|s| {
                json!({
                    "iterations": s.iterations,
                    "lastBuild": s.last_build.as_ref().map(|b| json!({
                        "output": b.output,
                        "notes": b.notes,
                        "tokensUsed": b.tokens_used
                    })),
                    "lastTest": s.last_test.as_ref().map(|t| json!({
                        "status": format!("{:?}", t.status),
                        "output": t.output,
                        "durationMs": t.duration_ms
                    })),
                    "lastImprove": s.last_improve.as_ref().map(|i| json!({
                        "action": format!("{:?}", i.action)
                    }))
                })
            })
        } else {
            None
        };

        Ok(json!({
            "success": true,
            "status": status,
            "activeLoops": active_count,
            "loopState": state.map(|s| json!({
                "iterations": s.iterations,
                "hasLastBuild": s.last_build.is_some(),
                "hasLastTest": s.last_test.is_some()
            })),
            "details": details
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge::context_architect::AssemblerConfig;
    use knowledge::hindsight::HindsightQueryConfig;
    use knowledge::note_taking::TrajectoryConfig;

    #[tokio::test]
    async fn test_context_assemble_tool() {
        let assembler = Arc::new(ContextAssembler::new(AssemblerConfig::default()));
        let tool = ContextAssembleTool::with_default_provider(assembler);

        let result = tool
            .call(json!({
                "tenantContext": {
                    "tenant_id": "test-tenant",
                    "user_id": "test-user"
                },
                "tokenBudget": 2000
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert!(result["context"]["totalTokens"].is_number());
    }

    #[tokio::test]
    async fn test_note_capture_tool() {
        let capture = Arc::new(RwLock::new(TrajectoryCapture::new(
            TrajectoryConfig::default(),
        )));
        let tool = NoteCaptureTool::new(capture.clone());

        let result = tool
            .call(json!({
                "tenantContext": {
                    "tenant_id": "test-tenant",
                    "user_id": "test-user"
                },
                "description": "Test capture",
                "tags": ["test"],
                "success": true
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["eventCount"], 1);
    }

    #[tokio::test]
    async fn test_hindsight_query_tool() {
        let query_engine = Arc::new(HindsightQuery::new(HindsightQueryConfig::default()));
        let tool = HindsightQueryTool::with_default_provider(query_engine);

        let result = tool
            .call(json!({
                "tenantContext": {
                    "tenant_id": "test-tenant",
                    "user_id": "test-user"
                },
                "errorType": "TypeError",
                "messagePattern": "cannot read property"
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["matchCount"], 0);
    }

    #[tokio::test]
    async fn test_meta_loop_status_tool() {
        let tool = MetaLoopStatusTool::with_default_provider();

        let result = tool
            .call(json!({
                "tenantContext": {
                    "tenant_id": "test-tenant",
                    "user_id": "test-user"
                }
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["status"], "idle");
        assert_eq!(result["activeLoops"], 0);
    }
}
