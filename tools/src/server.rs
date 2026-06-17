use crate::bridge::{ResolveFederationConflictTool, SyncNowTool, SyncStatusTool};
use crate::cca::{ContextAssembleTool, HindsightQueryTool, MetaLoopStatusTool, NoteCaptureTool};
use crate::governance::{
    GovernanceApproveTool, GovernanceAuditListTool, GovernanceConfigGetTool,
    GovernanceConfigureTool, GovernanceRejectTool, GovernanceRequestCreateTool,
    GovernanceRequestGetTool, GovernanceRequestListTool, GovernanceRoleAssignTool,
    GovernanceRoleListTool, GovernanceRoleRevokeTool, HierarchyNavigateTool, UnitCreateTool,
    UnitPolicyAddTool, UserRoleAssignTool, UserRoleRemoveTool,
};
use crate::graph::{
    GraphContextTool, GraphFindPathTool, GraphImplementationsTool, GraphLinkTool, GraphRelatedTool,
    GraphTraverseTool, GraphUnlinkTool, GraphViolationsTool,
};
use crate::knowledge::{
    InMemoryKnowledgeProposalStorage, KnowledgeApproveTool, KnowledgeGetTool, KnowledgeLinkTool,
    KnowledgeListTool, KnowledgePromoteTool, KnowledgePromotionPreviewTool, KnowledgeProposeTool,
    KnowledgeQueryTool, KnowledgeRejectTool, KnowledgeReviewPendingTool,
    SimpleKnowledgeInterpreter,
};
use crate::memory::{
    DefaultPromotionGovernance, GraphNeighborsTool, GraphPathTool, GraphQueryTool, MemoryAddTool,
    MemoryAutoPromoteTool, MemoryCloseTool, MemoryDeleteTool, MemoryFeedbackTool,
    MemoryOptimizeTool, MemoryPromoteTool, MemoryReasonTool, MemorySearchTool,
};
use crate::tools::{ToolDefinition, ToolRegistry};
use knowledge::governance::GovernanceEngine;
use memory::manager::MemoryManager;
use mk_core::traits::{AuthorizationService, EventPublisher, KnowledgeRepository};
use mk_core::types::SYSTEM_USER_ID;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use storage::events::EventError;
use storage::governance::GovernanceStorage;
use storage::graph_duckdb::DuckDbGraphStore;
use sync::bridge::SyncManager;
use tokio::time::timeout;
use tracing::{Span, debug, error, info, instrument, warn};

pub fn tool_to_cedar_action(tool_name: &str) -> &'static str {
    match tool_name {
        "memory_add" => "CreateMemory",
        "memory_search" => "SearchMemory",
        "memory_delete" => "DeleteMemory",
        "memory_reason" => "ReasonMemory",
        "memory_close" => "CloseMemory",
        "memory_feedback" => "FeedbackMemory",
        "memory_optimize" => "OptimizeMemory",
        "aeterna_memory_promote" => "PromoteMemory",
        "aeterna_memory_auto_promote" => "OptimizeMemory",

        "graph_query" => "QueryGraph",
        "graph_neighbors" => "QueryGraph",
        "graph_path" => "QueryGraph",
        "graph_link" => "ModifyGraph",
        "graph_unlink" => "ModifyGraph",
        "graph_traverse" => "QueryGraph",
        "graph_find_path" => "QueryGraph",
        "graph_violations" => "QueryGraph",
        "graph_implementations" => "QueryGraph",
        "graph_context" => "QueryGraph",
        "graph_related" => "QueryGraph",

        "knowledge_get" => "SearchKnowledge",
        "knowledge_list" => "ListKnowledge",
        "knowledge_query" => "SearchKnowledge",
        "aeterna_knowledge_propose" => "BatchKnowledge",
        "aeterna_knowledge_submit" => "BatchKnowledge",
        "aeterna_knowledge_pending" => "ListKnowledge",
        "aeterna_knowledge_promotion_preview" => "SearchKnowledge",
        "aeterna_knowledge_promote" => "ProposeKnowledge",
        "aeterna_knowledge_review_pending" => "ListKnowledge",
        "aeterna_knowledge_approve" => "ApproveKnowledge",
        "aeterna_knowledge_reject" => "ApproveKnowledge",
        "aeterna_knowledge_link" => "ModifyGraph",

        "sync_now" => "TriggerSync",
        "sync_status" => "ViewSyncStatus",
        "knowledge_resolve_conflict" => "ResolveConflict",

        "context_assemble" => "InvokeCCA",
        "note_capture" => "InvokeCCA",
        "hindsight_query" => "InvokeCCA",
        "meta_loop_status" => "InvokeCCA",

        "governance_unit_create" => "CreateOrganization",
        "governance_policy_add" => "EditPolicy",
        "governance_role_assign" => "AssignRoles",
        "governance_role_remove" => "AssignRoles",
        "governance_hierarchy_navigate" => "ViewGovernanceRequest",
        "governance_configure" => "EditPolicy",
        "governance_config_get" => "ViewGovernanceRequest",
        "governance_request_create" => "SubmitGovernanceRequest",
        "governance_approve" => "ApproveGovernanceRequest",
        "governance_reject" => "RejectGovernanceRequest",
        "governance_request_list" => "ViewGovernanceRequest",
        "governance_request_get" => "ViewGovernanceRequest",
        "governance_audit_list" => "ViewAuditLog",
        "governance_principal_role_assign" => "AssignRoles",
        "governance_role_revoke" => "AssignRoles",
        "governance_role_list" => "ViewGovernanceRequest",

        "aeterna_policy_propose" => "EditPolicy",
        "aeterna_policy_list_pending" => "ViewGovernance",

        "codesearch_search" => "InvokeMcpTool",
        "codesearch_trace_callers" => "InvokeMcpTool",
        "codesearch_trace_callees" => "InvokeMcpTool",
        "codesearch_graph" => "InvokeMcpTool",
        "codesearch_index_status" => "InvokeMcpTool",
        "codesearch_repo_request" => "InvokeMcpTool",

        _ => "InvokeMcpTool",
    }
}

/// MCP JSON-RPC server for tool orchestration.
///
/// Handles tool discovery and execution with integrated timeouts and tracing.
pub struct McpServer {
    registry: ToolRegistry,
    auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error>>,
    event_publisher: Option<Arc<dyn EventPublisher<Error = EventError>>>,
    extension_executor: Option<Arc<crate::extensions::ExtensionExecutor>>,
    timeout_duration: Duration,
    _governance_storage: Option<Arc<GovernanceStorage>>,
}

impl McpServer {
    /// Creates a new McpServer with initialized core tools.
    pub fn new(
        memory_manager: Arc<MemoryManager>,
        sync_manager: Arc<SyncManager>,
        knowledge_manager: Arc<knowledge::manager::KnowledgeManager>,
        knowledge_repository: Arc<
            dyn KnowledgeRepository<Error = knowledge::repository::RepositoryError>,
        >,
        storage_backend: Arc<
            dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>,
        >,
        governance_engine: Arc<GovernanceEngine>,
        reflective_reasoner: Arc<dyn memory::reasoning::ReflectiveReasoner>,
        auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error>>,
        event_publisher: Option<Arc<dyn EventPublisher<Error = EventError>>>,
        graph_store: Option<Arc<DuckDbGraphStore>>,
        governance_storage: Option<Arc<GovernanceStorage>>,
    ) -> Self {
        let mut registry = ToolRegistry::new();

        registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())));
        registry.register(Box::new(MemorySearchTool::new(memory_manager.clone())));
        registry.register(Box::new(MemoryDeleteTool::new(memory_manager.clone())));
        registry.register(Box::new(MemoryCloseTool::new(memory_manager.clone())));
        registry.register(Box::new(MemoryFeedbackTool::new(memory_manager.clone())));
        registry.register(Box::new(MemoryOptimizeTool::new(memory_manager.clone())));
        registry.register(Box::new(MemoryReasonTool::new(reflective_reasoner)));

        if let Some(graph) = graph_store {
            registry.register(Box::new(GraphQueryTool::new(graph.clone())));
            registry.register(Box::new(GraphNeighborsTool::new(graph.clone())));
            registry.register(Box::new(GraphPathTool::new(graph.clone())));

            let db = graph.db_handle();
            registry.register(Box::new(GraphLinkTool::new(db.clone())));
            registry.register(Box::new(GraphUnlinkTool::new(db.clone())));
            registry.register(Box::new(GraphTraverseTool::new(db.clone())));
            registry.register(Box::new(GraphFindPathTool::new(db.clone())));
            registry.register(Box::new(GraphViolationsTool::new(db.clone())));
            registry.register(Box::new(GraphImplementationsTool::new(db.clone())));
            registry.register(Box::new(GraphContextTool::new(db.clone())));
            registry.register(Box::new(GraphRelatedTool::new(db)));
        }

        registry.register(Box::new(KnowledgeGetTool::new(
            knowledge_repository.clone(),
        )));
        registry.register(Box::new(KnowledgeListTool::new(
            knowledge_repository.clone(),
        )));
        registry.register(Box::new(KnowledgeQueryTool::new(
            memory_manager.clone(),
            knowledge_repository.clone(),
        )));
        registry.register(Box::new(KnowledgeProposeTool::new(
            Arc::new(InMemoryKnowledgeProposalStorage::default()),
            Arc::new(SimpleKnowledgeInterpreter),
        )));
        registry.register(Box::new(KnowledgePromotionPreviewTool::new(
            knowledge_manager.clone(),
        )));
        registry.register(Box::new(KnowledgePromoteTool::new(
            knowledge_manager.clone(),
        )));
        registry.register(Box::new(KnowledgeReviewPendingTool::new(
            knowledge_manager.clone(),
        )));
        registry.register(Box::new(KnowledgeApproveTool::new(
            knowledge_manager.clone(),
        )));
        registry.register(Box::new(KnowledgeRejectTool::new(
            knowledge_manager.clone(),
        )));
        registry.register(Box::new(KnowledgeLinkTool::new(knowledge_manager.clone())));

        registry.register(Box::new(MemoryPromoteTool::new(
            memory_manager.clone(),
            Arc::new(DefaultPromotionGovernance::new()),
        )));
        registry.register(Box::new(MemoryAutoPromoteTool::new(memory_manager.clone())));

        registry.register(Box::new(SyncNowTool::new(sync_manager.clone())));
        registry.register(Box::new(SyncStatusTool::new(sync_manager.clone())));
        registry.register(Box::new(ResolveFederationConflictTool::new(sync_manager)));

        registry.register(Box::new(UnitCreateTool::new(
            storage_backend.clone(),
            governance_engine.clone(),
        )));
        registry.register(Box::new(UnitPolicyAddTool::new(
            storage_backend.clone(),
            governance_engine.clone(),
        )));
        registry.register(Box::new(UserRoleAssignTool::new(
            storage_backend.clone(),
            governance_engine.clone(),
        )));
        registry.register(Box::new(UserRoleRemoveTool::new(
            storage_backend.clone(),
            governance_engine.clone(),
        )));
        registry.register(Box::new(HierarchyNavigateTool::new(storage_backend)));

        // Register UX-first governance workflow tools
        if let Some(gov_storage) = governance_storage.clone() {
            registry.register(Box::new(GovernanceConfigureTool::new(
                gov_storage.clone(),
                governance_engine.clone(),
            )));
            registry.register(Box::new(GovernanceConfigGetTool::new(gov_storage.clone())));
            registry.register(Box::new(GovernanceRequestCreateTool::new(
                gov_storage.clone(),
                governance_engine.clone(),
            )));
            registry.register(Box::new(GovernanceApproveTool::new(
                gov_storage.clone(),
                governance_engine.clone(),
            )));
            registry.register(Box::new(GovernanceRejectTool::new(
                gov_storage.clone(),
                governance_engine.clone(),
            )));
            registry.register(Box::new(
                GovernanceRequestListTool::new(gov_storage.clone())
                    .with_knowledge_manager(knowledge_manager.clone()),
            ));
            registry.register(Box::new(GovernanceRequestGetTool::new(gov_storage.clone())));
            registry.register(Box::new(GovernanceAuditListTool::new(gov_storage.clone())));
            registry.register(Box::new(GovernanceRoleAssignTool::new(
                gov_storage.clone(),
                governance_engine.clone(),
            )));
            registry.register(Box::new(GovernanceRoleRevokeTool::new(gov_storage.clone())));
            registry.register(Box::new(GovernanceRoleListTool::new(gov_storage.clone())));
        }

        // Register CCA tools
        registry.register(Box::new(ContextAssembleTool::with_default_provider(
            Arc::new(knowledge::context_architect::ContextAssembler::new(
                knowledge::context_architect::AssemblerConfig::default(),
            )),
        )));
        registry.register(Box::new(NoteCaptureTool::new(Arc::new(
            std::sync::RwLock::new(knowledge::note_taking::TrajectoryCapture::new(
                knowledge::note_taking::TrajectoryConfig::default(),
            )),
        ))));
        registry.register(Box::new(HindsightQueryTool::with_default_provider(
            Arc::new(knowledge::hindsight::HindsightQuery::new(
                knowledge::hindsight::HindsightQueryConfig::default(),
            )),
        )));
        registry.register(Box::new(MetaLoopStatusTool::with_default_provider()));

        Self {
            registry,
            auth_service,
            event_publisher,
            extension_executor: None,
            timeout_duration: Duration::from_secs(30),
            _governance_storage: governance_storage,
        }
    }

    pub fn with_extension_executor(
        mut self,
        executor: Arc<crate::extensions::ExtensionExecutor>,
    ) -> Self {
        self.extension_executor = Some(executor);
        self
    }

    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout_duration = duration;
        self
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.registry.list_tools()
    }

    #[instrument(skip(self, request), fields(method = %request.method, request_id = ?request.id))]
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!(method = %request.method, "Handling JSON-RPC request");

        if request.method.contains("TRIGGER_FAILURE") {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(JsonRpcError::internal_error("Simulated failure")),
            };
        }

        let timeout_duration = self.timeout_duration;

        let result = timeout(timeout_duration, self.dispatch(request)).await;

        match result {
            Ok(response) => response,
            Err(_) => {
                error!("Request timed out");
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError::request_timeout("Request timed out")),
                }
            }
        }
    }

    async fn dispatch(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": "aeterna-tools",
                        "version": "0.2.0"
                    }
                })),
                error: None,
            },
            "tools/list" => {
                let tools = self.registry.list_tools();
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(serde_json::to_value(tools).unwrap()),
                    error: None,
                }
            }
            "tools/call" => {
                let params = match request.params {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params("Invalid params")),
                        };
                    }
                };

                let tenant_context: mk_core::types::TenantContext =
                    match serde_json::from_value::<mk_core::types::TenantContext>(
                        params["tenantContext"].clone(),
                    ) {
                        Ok(ctx) => {
                            if ctx.tenant_id.as_str().contains("TRIGGER_FAILURE") {
                                return JsonRpcResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id: request.id,
                                    result: None,
                                    error: Some(JsonRpcError::internal_error(
                                        "Simulated tenant failure",
                                    )),
                                };
                            }
                            ctx
                        }
                        Err(_) => {
                            return JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: None,
                                error: Some(JsonRpcError::invalid_params(
                                    "Missing or invalid tenant context",
                                )),
                            };
                        }
                    };

                let (name, tool_params) = match self.extract_call_params(&params, &tenant_context) {
                    Ok(res) => res,
                    Err(e) => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params(e)),
                        };
                    }
                };

                let mut tool_params = tool_params;
                let tool_registry = Arc::new(self.registry.clone());
                if let Some(executor) = &self.extension_executor
                    && let Some(session_id) = params.get("sessionId").and_then(|v| v.as_str())
                    && let Some(input) = tool_params.get("input").and_then(|v| v.as_str())
                {
                    let updated = executor
                        .on_plain_text(
                            tenant_context.clone(),
                            session_id,
                            tool_registry.clone(),
                            input.to_string(),
                        )
                        .await;
                    if let Ok(text) = updated
                        && let Some(obj) = tool_params.as_object_mut()
                    {
                        obj.insert("input".to_string(), Value::String(text));
                    }
                }

                Span::current().record("tool_name", &name);
                info!(tool = %name, "Calling tool");

                let cedar_action = tool_to_cedar_action(&name);
                if cedar_action == "InvokeMcpTool" && !name.starts_with("codesearch_") {
                    warn!(tool = %name, "No specific Cedar action mapping; using InvokeMcpTool fallback");
                }

                let cedar_resource =
                    format!("Aeterna::Tenant::\"{}\"", tenant_context.tenant_id.as_str());
                let auth_result = self
                    .auth_service
                    .check_permission(&tenant_context, cedar_action, &cedar_resource)
                    .await;

                match auth_result {
                    Ok(allowed) => {
                        if !allowed {
                            error!(tool = %name, "Authorization denied");
                            return JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32002,
                                    message: format!(
                                        "Authorization error: access denied for tool {}",
                                        name
                                    ),
                                    data: None,
                                }),
                            };
                        }
                    }
                    Err(e) => {
                        error!(tool = %name, error = %e, "Authorization check failed");
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32002,
                                message: format!("Authorization error: {}", e),
                                data: None,
                            }),
                        };
                    }
                }

                let call_result = self.registry.call(&name, tool_params).await;

                match call_result {
                    Ok(result) => {
                        info!(tool = %name, "Tool call successful");

                        if let Some(ref publisher) = self.event_publisher {
                            let timestamp = chrono::Utc::now().timestamp();
                            let event = match name.as_str() {
                                "unit_create" => {
                                    Some(mk_core::types::GovernanceEvent::UnitCreated {
                                        unit_id: result["unit_id"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        unit_type: serde_json::from_value(
                                            result["unit_type"].clone(),
                                        )
                                        .unwrap_or(mk_core::types::UnitType::Project),
                                        tenant_id: tenant_context.tenant_id.clone(),
                                        parent_id: result["parent_id"]
                                            .as_str()
                                            .map(|s| s.to_string()),
                                        timestamp,
                                    })
                                }
                                "role_assign" => {
                                    Some(mk_core::types::GovernanceEvent::RoleAssigned {
                                        user_id: serde_json::from_value(result["user_id"].clone())
                                            .unwrap_or_default(),
                                        unit_id: result["unit_id"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        role: serde_json::from_value(result["role"].clone())
                                            .unwrap_or(mk_core::types::Role::Developer.into()),
                                        tenant_id: tenant_context.tenant_id.clone(),
                                        timestamp,
                                    })
                                }
                                "role_remove" => {
                                    Some(mk_core::types::GovernanceEvent::RoleRemoved {
                                        user_id: serde_json::from_value(result["user_id"].clone())
                                            .unwrap_or_default(),
                                        unit_id: result["unit_id"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        role: serde_json::from_value(result["role"].clone())
                                            .unwrap_or(mk_core::types::Role::Developer.into()),
                                        tenant_id: tenant_context.tenant_id.clone(),
                                        timestamp,
                                    })
                                }
                                "unit_policy_add" => {
                                    Some(mk_core::types::GovernanceEvent::PolicyUpdated {
                                        policy_id: result["policy_id"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        layer: serde_json::from_value(result["layer"].clone())
                                            .unwrap_or(mk_core::types::KnowledgeLayer::Project),
                                        tenant_id: tenant_context.tenant_id.clone(),
                                        timestamp,
                                    })
                                }
                                _ => None,
                            };

                            if let Some(event) = event
                                && let Err(e) = publisher.publish(event).await
                            {
                                error!(error = %e, "Failed to publish governance event");
                            }
                        }

                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: Some(result),
                            error: None,
                        }
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        error!(tool = %name, error = %error_str, "Tool call failed");
                        let rpc_error = if error_str.contains("not found") {
                            JsonRpcError::method_not_found(error_str)
                        } else if e.is::<serde_json::Error>()
                            || error_str.contains("Validation error")
                        {
                            JsonRpcError::invalid_params(error_str)
                        } else {
                            JsonRpcError::internal_error(error_str)
                        };

                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(rpc_error),
                        }
                    }
                }
            }
            _ => {
                debug!(method = %request.method, "Method not found");
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(JsonRpcError::method_not_found("Method not found")),
                }
            }
        }
    }

    fn extract_call_params(
        &self,
        params: &Value,
        tenant_context: &mk_core::types::TenantContext,
    ) -> Result<(String, Value), String> {
        let name = match params["name"].as_str() {
            Some(n) => n.to_string(),
            None => return Err("Missing tool name".to_string()),
        };

        let mut tool_params = params["arguments"].clone();
        if tool_params.is_null() {
            tool_params = serde_json::json!({});
        }

        if let Some(obj) = tool_params.as_object_mut() {
            obj.insert(
                "tenant_context".to_string(),
                serde_json::to_value(tenant_context).unwrap(),
            );
            obj.insert(
                "tenantContext".to_string(),
                serde_json::to_value(tenant_context).unwrap(),
            );
        } else {
            tool_params = serde_json::json!({
                "tenant_context": tenant_context,
                "tenantContext": tenant_context
            });
        }

        Ok((name, tool_params))
    }

    /// Handle a JSON-RPC request with an authenticated caller tenant constraint.
    ///
    /// When `caller_tenant` is `Some`, validates that the `tenantContext.tenant_id`
    /// in the payload matches the authenticated caller's tenant.  This prevents a
    /// caller from self-asserting an arbitrary tenant scope in the JSON-RPC payload.
    ///
    /// When `caller_tenant` is `None` (plugin auth disabled / dev mode), the payload
    /// `tenantContext` is accepted verbatim — same behaviour as `handle_request`.
    pub async fn handle_request_with_caller(
        &self,
        mut request: JsonRpcRequest,
        caller_tenant: Option<&str>,
    ) -> JsonRpcResponse {
        if let Some(caller) = caller_tenant
            && request.method == "tools/call"
            && let Some(ref params) = request.params
        {
            let payload_tenant = params["tenantContext"]["tenant_id"]
                .as_str()
                .or_else(|| params["tenantContext"]["tenantId"].as_str());

            if let Some(payload) = payload_tenant {
                if payload != caller {
                    tracing::warn!(
                        caller_tenant = %caller,
                        payload_tenant = %payload,
                        "MCP tenantContext mismatch: payload tenant exceeds authenticated scope"
                    );
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: None,
                        error: Some(JsonRpcError::unauthorized(
                            "tenantContext in payload does not match authenticated caller tenant",
                        )),
                    };
                }
            } else {
                // No tenantContext provided: inject caller's tenant so tools
                // operate in the correct tenant scope.
                let params_mut = request.params.get_or_insert(serde_json::json!({}));
                if let Some(obj) = params_mut.as_object_mut() {
                    obj.entry("tenantContext").or_insert_with(
                        || serde_json::json!({"tenant_id": caller, "user_id": SYSTEM_USER_ID}),
                    );
                }
            }
        }
        self.handle_request(request).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
            data: None,
        }
    }

    pub fn request_timeout(message: impl Into<String>) -> Self {
        Self {
            code: -32001,
            message: message.into(),
            data: None,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: -32003,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::manager::MemoryManager;

    use serde_json::json;
    use sync::bridge::SyncManager;
    use sync::state_persister::SyncStatePersister;

    struct MockPersister;
    #[async_trait::async_trait]
    impl SyncStatePersister for MockPersister {
        async fn load(
            &self,
            _tenant_id: &mk_core::types::TenantId,
        ) -> std::result::Result<sync::state::SyncState, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(sync::state::SyncState::default())
        }
        async fn save(
            &self,
            _tenant_id: &mk_core::types::TenantId,
            _: &sync::state::SyncState,
        ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }
    }

    struct MockAuthService;
    #[async_trait::async_trait]
    impl mk_core::traits::AuthorizationService for MockAuthService {
        type Error = anyhow::Error;
        async fn check_permission(
            &self,
            _ctx: &mk_core::types::TenantContext,
            _action: &str,
            _resource: &str,
        ) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn get_user_roles(
            &self,
            _ctx: &mk_core::types::TenantContext,
        ) -> anyhow::Result<Vec<mk_core::types::RoleIdentifier>> {
            Ok(vec![])
        }
        async fn assign_role(
            &self,
            _ctx: &mk_core::types::TenantContext,
            _user_id: &mk_core::types::UserId,
            _role: mk_core::types::RoleIdentifier,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn remove_role(
            &self,
            _ctx: &mk_core::types::TenantContext,
            _user_id: &mk_core::types::UserId,
            _role: mk_core::types::RoleIdentifier,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct MockStorageBackend;
    #[async_trait::async_trait]
    impl mk_core::traits::StorageBackend for MockStorageBackend {
        type Error = storage::postgres::PostgresError;
        async fn store(
            &self,
            _ctx: mk_core::types::TenantContext,
            _key: &str,
            _value: &[u8],
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn retrieve(
            &self,
            _ctx: mk_core::types::TenantContext,
            _key: &str,
        ) -> Result<Option<Vec<u8>>, Self::Error> {
            Ok(None)
        }
        async fn delete(
            &self,
            _ctx: mk_core::types::TenantContext,
            _key: &str,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn exists(
            &self,
            _ctx: mk_core::types::TenantContext,
            _key: &str,
        ) -> Result<bool, Self::Error> {
            Ok(false)
        }
        async fn get_ancestors(
            &self,
            _ctx: mk_core::types::TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
            Ok(vec![])
        }
        async fn get_descendants(
            &self,
            _ctx: mk_core::types::TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
            Ok(vec![])
        }
        async fn get_unit_policies(
            &self,
            _ctx: mk_core::types::TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<mk_core::types::Policy>, Self::Error> {
            Ok(vec![])
        }
        async fn create_unit(
            &self,
            _unit: &mk_core::types::OrganizationalUnit,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn add_unit_policy(
            &self,
            _ctx: &mk_core::types::TenantContext,
            _unit_id: &str,
            _policy: &mk_core::types::Policy,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn assign_role(
            &self,
            _user_id: &mk_core::types::UserId,
            _tenant_id: &mk_core::types::TenantId,
            _unit_id: &str,
            _role: mk_core::types::RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn remove_role(
            &self,
            _user_id: &mk_core::types::UserId,
            _tenant_id: &mk_core::types::TenantId,
            _unit_id: &str,
            _role: mk_core::types::RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn store_drift_result(
            &self,
            _result: mk_core::types::DriftResult,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_latest_drift_result(
            &self,
            _ctx: mk_core::types::TenantContext,
            _project_id: &str,
        ) -> Result<Option<mk_core::types::DriftResult>, Self::Error> {
            Ok(None)
        }
        async fn list_all_units(
            &self,
        ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
            Ok(vec![])
        }
        async fn record_job_status(
            &self,
            _job_name: &str,
            _tenant_id: &str,
            _status: &str,
            _message: Option<&str>,
            _started_at: i64,
            _finished_at: Option<i64>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_governance_events(
            &self,
            _ctx: mk_core::types::TenantContext,
            _since_timestamp: i64,
            _limit: usize,
        ) -> Result<Vec<mk_core::types::GovernanceEvent>, Self::Error> {
            Ok(vec![])
        }
        async fn create_suppression(
            &self,
            _suppression: mk_core::types::DriftSuppression,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn list_suppressions(
            &self,
            _ctx: mk_core::types::TenantContext,
            _project_id: &str,
        ) -> Result<Vec<mk_core::types::DriftSuppression>, Self::Error> {
            Ok(vec![])
        }
        async fn delete_suppression(
            &self,
            _ctx: mk_core::types::TenantContext,
            _suppression_id: &str,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_drift_config(
            &self,
            _ctx: mk_core::types::TenantContext,
            _project_id: &str,
        ) -> Result<Option<mk_core::types::DriftConfig>, Self::Error> {
            Ok(None)
        }
        async fn save_drift_config(
            &self,
            _config: mk_core::types::DriftConfig,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn persist_event(
            &self,
            _event: mk_core::types::PersistentEvent,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_pending_events(
            &self,
            _ctx: mk_core::types::TenantContext,
            _limit: usize,
        ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
            Ok(vec![])
        }
        async fn update_event_status(
            &self,
            _event_id: &str,
            _status: mk_core::types::EventStatus,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_dead_letter_events(
            &self,
            _ctx: mk_core::types::TenantContext,
            _limit: usize,
        ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
            Ok(vec![])
        }
        async fn check_idempotency(
            &self,
            _consumer_group: &str,
            _idempotency_key: &str,
        ) -> Result<bool, Self::Error> {
            Ok(false)
        }
        async fn record_consumer_state(
            &self,
            _state: mk_core::types::ConsumerState,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_event_metrics(
            &self,
            _ctx: mk_core::types::TenantContext,
            _period_start: i64,
            _period_end: i64,
        ) -> Result<Vec<mk_core::types::EventDeliveryMetrics>, Self::Error> {
            Ok(vec![])
        }
        async fn record_event_metrics(
            &self,
            _metrics: mk_core::types::EventDeliveryMetrics,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_unit_by_id(
            &self,
            _unit_id: &str,
            _tenant_id: &str,
        ) -> Result<Option<mk_core::types::OrganizationalUnit>, Self::Error> {
            Ok(None)
        }
        async fn update_unit(
            &self,
            _unit: &mk_core::types::OrganizationalUnit,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn delete_unit(&self, _unit_id: &str, _tenant_id: &str) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn list_unit_members(
            &self,
            _unit_id: &str,
            _tenant_id: &str,
        ) -> Result<Vec<(mk_core::types::UserId, mk_core::types::RoleIdentifier)>, Self::Error>
        {
            Ok(Vec::new())
        }
        async fn assign_team_to_project(
            &self,
            _project_id: &str,
            _team_id: &str,
            _tenant_id: &str,
            _assignment_type: &str,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn remove_team_from_project(
            &self,
            _project_id: &str,
            _team_id: &str,
            _tenant_id: &str,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn list_project_team_assignments(
            &self,
            _project_id: &str,
            _tenant_id: &str,
        ) -> Result<Vec<(String, String)>, Self::Error> {
            Ok(Vec::new())
        }
        async fn get_effective_roles_at_scope(
            &self,
            _user_id: &mk_core::types::UserId,
            _tenant_id: &mk_core::types::TenantId,
            _unit_id: &str,
        ) -> Result<Vec<mk_core::types::RoleIdentifier>, Self::Error> {
            Ok(Vec::new())
        }
    }

    async fn setup_server() -> McpServer {
        let memory_manager = Arc::new(MemoryManager::new());
        let repo = Arc::new(knowledge::repository::GitRepository::new_mock());
        let governance = Arc::new(knowledge::governance::GovernanceEngine::new());
        let knowledge_manager = Arc::new(knowledge::manager::KnowledgeManager::new(
            repo.clone(),
            governance.clone(),
        ));
        let sync_manager = Arc::new(
            SyncManager::new(
                memory_manager.clone(),
                knowledge_manager,
                config::config::DeploymentConfig::default(),
                None,
                Arc::new(MockPersister),
                None,
            )
            .await
            .unwrap(),
        );

        let mock_reasoner = Arc::new(memory::reasoning::DefaultReflectiveReasoner::new(Arc::new(
            memory::llm::mock::MockLlmService::new(),
        )));

        McpServer::new(
            memory_manager,
            sync_manager,
            Arc::new(knowledge::manager::KnowledgeManager::new(
                repo.clone(),
                governance.clone(),
            )),
            repo,
            Arc::new(MockStorageBackend),
            governance,
            mock_reasoner,
            Arc::new(MockAuthService),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn test_server_initialize() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "initialize".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
    }

    #[tokio::test]
    async fn test_server_list_tools() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        let tools = response.result.unwrap();
        assert!(tools.as_array().unwrap().len() >= 8);
    }

    #[tokio::test]
    async fn test_server_method_not_found() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "unknown_method".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_server_invalid_params() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn test_server_tool_not_found() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!({
                "tenantContext": {
                    "tenant_id": "c1",
                    "user_id": "u1"
                },
                "name": "non_existent_tool",
                "arguments": {}
            })),
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_extract_tenant_context() {
        let server = setup_server().await;

        let params = json!({
            "tenantContext": {
                "tenantId": "company_1",
                "userId": "user_1"
            },
            "name": "memory_add",
            "arguments": {
                "content": "test"
            }
        });

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(params),
        };

        let _response = server.handle_request(request).await;
    }

    #[tokio::test]
    async fn test_extract_tenant_context_missing() {
        let server = setup_server().await;

        let params = json!({
            "name": "memory_add",
            "arguments": {
                "content": "test"
            }
        });

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(params),
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        let err = response.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("Missing or invalid tenant context"));
    }

    #[tokio::test]
    async fn test_server_failure_hardening() {
        let server = setup_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "TRIGGER_FAILURE_METHOD".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().message, "Simulated failure");

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(2),
            method: "tools/call".to_string(),
            params: Some(json!({
                "tenantContext": {
                    "tenant_id": "TRIGGER_FAILURE_TENANT",
                    "user_id": "u1"
                },
                "name": "memory_add",
                "arguments": {
                    "content": "test"
                }
            })),
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().message, "Simulated tenant failure");
    }

    #[tokio::test]
    async fn test_server_timeout() {
        let server = setup_server().await.with_timeout(Duration::from_millis(1));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "initialize".to_string(),
            params: None,
        };

        let _response = server.handle_request(request).await;
    }

    #[test]
    fn test_json_rpc_error_constructors() {
        let invalid_params = JsonRpcError::invalid_params("Invalid param");
        assert_eq!(invalid_params.code, -32602);
        assert_eq!(invalid_params.message, "Invalid param");
        assert!(invalid_params.data.is_none());

        let method_not_found = JsonRpcError::method_not_found("Not found");
        assert_eq!(method_not_found.code, -32601);
        assert_eq!(method_not_found.message, "Not found");

        let internal = JsonRpcError::internal_error("Internal error");
        assert_eq!(internal.code, -32000);
        assert_eq!(internal.message, "Internal error");

        let timeout = JsonRpcError::request_timeout("Timeout");
        assert_eq!(timeout.code, -32001);
        assert_eq!(timeout.message, "Timeout");
    }

    #[test]
    fn test_list_tools() {
        let registry = crate::tools::ToolRegistry::new();
        let tools = registry.list_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_json_rpc_request_serde() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "test".to_string(),
            params: Some(json!({"key": "value"})),
        };

        let serialized = serde_json::to_string(&request).unwrap();
        let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.jsonrpc, "2.0");
        assert_eq!(deserialized.method, "test");
        assert!(deserialized.params.is_some());
    }

    #[test]
    fn test_json_rpc_response_serde() {
        let response_success = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: Some(json!({"data": "test"})),
            error: None,
        };

        let serialized = serde_json::to_string(&response_success).unwrap();
        assert!(!serialized.contains("error"));

        let response_error = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: None,
            error: Some(JsonRpcError::internal_error("fail")),
        };

        let serialized_err = serde_json::to_string(&response_error).unwrap();
        assert!(!serialized_err.contains("result"));
        assert!(serialized_err.contains("error"));
    }

    #[tokio::test]
    async fn test_tools_call_missing_tool_name() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!({
                "tenantContext": {
                    "tenant_id": "c1",
                    "user_id": "u1"
                },
                "arguments": {}
            })),
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    // ── Task 4.2: MCP handle_request_with_caller tenant scope enforcement ─────

    #[tokio::test]
    async fn handle_request_with_caller_rejects_mismatched_tenant_context() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "memory_add",
                "tenantContext": {
                    "tenant_id": "attacker-tenant",
                    "user_id": "evil"
                },
                "arguments": {
                    "content": "injected",
                    "layer": "tenant"
                }
            })),
        };

        let response = server
            .handle_request_with_caller(request, Some("legitimate-tenant"))
            .await;

        assert!(
            response.error.is_some(),
            "MUST return an error when payload tenantContext does not match authenticated caller tenant"
        );
        let err = response.error.unwrap();
        assert_eq!(
            err.code, -32003,
            "Error code MUST be -32003 (unauthorized) for tenant scope violation"
        );
        assert!(
            err.message.contains("tenantContext"),
            "Error message MUST mention tenantContext"
        );
    }

    #[tokio::test]
    async fn handle_request_with_caller_accepts_matching_tenant_context() {
        let server = setup_server().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(2),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "memory_add",
                "tenantContext": {
                    "tenant_id": "my-tenant",
                    "user_id": "alice"
                },
                "arguments": {
                    "content": "hello",
                    "layer": "project"
                }
            })),
        };

        let response = server
            .handle_request_with_caller(request, Some("my-tenant"))
            .await;

        // The request may succeed or fail on business logic, but MUST NOT
        // return a tenant-scope (-32003) error.
        if let Some(ref err) = response.error {
            assert_ne!(
                err.code, -32003,
                "MUST NOT reject a matching tenantContext with a tenant-scope error"
            );
        }
    }

    #[tokio::test]
    async fn handle_request_with_caller_injects_tenant_when_context_absent() {
        let server = setup_server().await;
        // Deliberately omit tenantContext from params.
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(3),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "memory_add",
                "arguments": {
                    "content": "hello",
                    "layer": "project"
                }
            })),
        };

        let response = server
            .handle_request_with_caller(request, Some("injected-tenant"))
            .await;

        // Must NOT return a tenant-scope error; the caller tenant was injected.
        if let Some(ref err) = response.error {
            assert_ne!(
                err.code, -32003,
                "MUST NOT return tenant-scope error when tenantContext was absent (it should be injected)"
            );
        }
    }

    #[tokio::test]
    async fn handle_request_with_caller_passes_through_when_no_caller() {
        let server = setup_server().await;
        // caller_tenant = None simulates dev / auth-disabled mode.
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(4),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "memory_add",
                "tenantContext": {
                    "tenant_id": "any-tenant",
                    "user_id": "dev"
                },
                "arguments": {
                    "content": "hello",
                    "layer": "project"
                }
            })),
        };

        let response = server.handle_request_with_caller(request, None).await;

        if let Some(ref err) = response.error {
            assert_ne!(
                err.code, -32003,
                "MUST NOT apply tenant scope enforcement when caller_tenant is None (dev mode)"
            );
        }
    }

    #[test]
    fn json_rpc_error_unauthorized_has_correct_code() {
        let err = JsonRpcError::unauthorized("denied");
        assert_eq!(err.code, -32003, "Unauthorized error MUST use code -32003");
        assert_eq!(err.message, "denied");
    }
}
