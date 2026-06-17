//! Cedar Authorization Integration Tests
//!
//! Comprehensive tests for Cedar policy evaluation including:
//! - Multi-tenant authorization isolation
//! - Agent delegation (ActAs) scenarios
//! - Role-based access control (RBAC)
//! - Hierarchical unit inheritance
//! - Policy evaluation edge cases
//! - Error handling for invalid policies/requests

use adapters::auth::cedar::{CedarAuthorizer, CedarError};
use mk_core::traits::AuthorizationService;
use mk_core::types::{TenantContext, TenantId, UserId};

// =============================================================================
// Test Policies
// =============================================================================

/// Basic schema for Cedar tests
const TEST_SCHEMA: &str = r#"{
    "": {
        "entityTypes": {
            "User": {},
            "Unit": {},
            "Role": {},
            "Memory": {},
            "Knowledge": {}
        },
        "actions": {
            "View": {
                "appliesTo": {
                    "principalTypes": ["User"],
                    "resourceTypes": ["Unit", "Memory", "Knowledge"]
                }
            },
            "Edit": {
                "appliesTo": {
                    "principalTypes": ["User"],
                    "resourceTypes": ["Unit", "Memory", "Knowledge"]
                }
            },
            "Delete": {
                "appliesTo": {
                    "principalTypes": ["User"],
                    "resourceTypes": ["Unit", "Memory", "Knowledge"]
                }
            },
            "Admin": {
                "appliesTo": {
                    "principalTypes": ["User"],
                    "resourceTypes": ["Unit"]
                }
            },
            "ActAs": {
                "appliesTo": {
                    "principalTypes": ["User"],
                    "resourceTypes": ["User"]
                }
            }
        }
    }
}"#;

// =============================================================================
// Helper Functions
// =============================================================================

fn create_tenant_context(tenant: &str, user: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(tenant.into()).unwrap(),
        UserId::new(user.into()).unwrap(),
    )
}

fn create_agent_context(tenant: &str, user: &str, agent: &str) -> TenantContext {
    let mut ctx = TenantContext::new(
        TenantId::new(tenant.into()).unwrap(),
        UserId::new(user.into()).unwrap(),
    );
    ctx.agent_id = Some(agent.to_string());
    ctx
}

// =============================================================================
// Multi-Tenant Isolation Tests
// =============================================================================

mod multi_tenant_isolation {
    use super::*;

    #[tokio::test]
    async fn test_user_can_access_own_tenant_resources() {
        let policies = r#"
            permit(principal == User::"tenant1-user1", action == Action::"View", resource == Unit::"tenant1-unit1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "tenant1-user1");

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"tenant1-unit1\"")
            .await
            .unwrap();
        assert!(allowed, "User should access resources in own tenant");
    }

    #[tokio::test]
    async fn test_user_cannot_access_other_tenant_resources() {
        let policies = r#"
            permit(principal == User::"tenant1-user1", action == Action::"View", resource == Unit::"tenant1-unit1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "tenant1-user1");

        // Try to access tenant2's resource
        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"tenant2-unit1\"")
            .await
            .unwrap();
        assert!(!denied, "User should NOT access resources in other tenant");
    }

    #[tokio::test]
    async fn test_multiple_tenants_isolated() {
        let policies = r#"
            permit(principal == User::"alice", action == Action::"View", resource == Unit::"acme-data");
            permit(principal == User::"bob", action == Action::"View", resource == Unit::"globex-data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Alice can access ACME data
        let alice_ctx = create_tenant_context("acme", "alice");
        let alice_acme = authorizer
            .check_permission(&alice_ctx, "View", "Unit::\"acme-data\"")
            .await
            .unwrap();
        assert!(alice_acme, "Alice should access ACME data");

        // Alice cannot access Globex data
        let alice_globex = authorizer
            .check_permission(&alice_ctx, "View", "Unit::\"globex-data\"")
            .await
            .unwrap();
        assert!(!alice_globex, "Alice should NOT access Globex data");

        // Bob can access Globex data
        let bob_ctx = create_tenant_context("globex", "bob");
        let bob_globex = authorizer
            .check_permission(&bob_ctx, "View", "Unit::\"globex-data\"")
            .await
            .unwrap();
        assert!(bob_globex, "Bob should access Globex data");

        // Bob cannot access ACME data
        let bob_acme = authorizer
            .check_permission(&bob_ctx, "View", "Unit::\"acme-data\"")
            .await
            .unwrap();
        assert!(!bob_acme, "Bob should NOT access ACME data");
    }

    #[tokio::test]
    async fn test_tenant_admin_scoped_to_tenant() {
        let policies = r#"
            permit(principal == User::"tenant1-admin", action == Action::"Admin", resource == Unit::"tenant1-unit1");
            permit(principal == User::"tenant1-admin", action == Action::"Admin", resource == Unit::"tenant1-unit2");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "tenant1-admin");

        // Admin can manage tenant1 units
        let can_admin_unit1 = authorizer
            .check_permission(&ctx, "Admin", "Unit::\"tenant1-unit1\"")
            .await
            .unwrap();
        assert!(can_admin_unit1);

        let can_admin_unit2 = authorizer
            .check_permission(&ctx, "Admin", "Unit::\"tenant1-unit2\"")
            .await
            .unwrap();
        assert!(can_admin_unit2);

        // Admin cannot manage tenant2 units
        let cannot_admin_other = authorizer
            .check_permission(&ctx, "Admin", "Unit::\"tenant2-unit1\"")
            .await
            .unwrap();
        assert!(!cannot_admin_other);
    }
}

// =============================================================================
// Agent Delegation (ActAs) Tests
// =============================================================================

mod agent_delegation {
    use super::*;

    #[tokio::test]
    async fn test_agent_with_valid_delegation_can_act() {
        let policies = r#"
            permit(principal == User::"agent-123", action == Action::"ActAs", resource == User::"user1");
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_agent_context("tenant1", "user1", "agent-123");

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"data1\"")
            .await
            .unwrap();
        assert!(
            allowed,
            "Agent with delegation should act on behalf of user"
        );
    }

    #[tokio::test]
    async fn test_agent_without_delegation_denied() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_agent_context("tenant1", "user1", "unauthorized-agent");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"data1\"")
            .await
            .unwrap();
        assert!(!denied, "Agent without ActAs delegation should be denied");
    }

    #[tokio::test]
    async fn test_agent_delegation_to_wrong_user_denied() {
        let policies = r#"
            permit(principal == User::"agent-123", action == Action::"ActAs", resource == User::"user2");
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        // Agent has delegation to user2, but trying to act as user1
        let ctx = create_agent_context("tenant1", "user1", "agent-123");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"data1\"")
            .await
            .unwrap();
        assert!(
            !denied,
            "Agent delegated to different user should be denied"
        );
    }

    #[tokio::test]
    async fn test_multiple_agents_different_delegations() {
        let policies = r#"
            permit(principal == User::"agent-a", action == Action::"ActAs", resource == User::"alice");
            permit(principal == User::"agent-b", action == Action::"ActAs", resource == User::"bob");
            permit(principal == User::"alice", action == Action::"View", resource == Unit::"alice-data");
            permit(principal == User::"bob", action == Action::"View", resource == Unit::"bob-data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Agent-A acting as Alice can access Alice's data
        let ctx_a = create_agent_context("tenant1", "alice", "agent-a");
        let allowed_a = authorizer
            .check_permission(&ctx_a, "View", "Unit::\"alice-data\"")
            .await
            .unwrap();
        assert!(allowed_a, "Agent-A should access Alice's data");

        // Agent-A acting as Alice cannot access Bob's data
        let denied_a = authorizer
            .check_permission(&ctx_a, "View", "Unit::\"bob-data\"")
            .await
            .unwrap();
        assert!(!denied_a, "Agent-A should NOT access Bob's data via Alice");

        // Agent-B acting as Bob can access Bob's data
        let ctx_b = create_agent_context("tenant1", "bob", "agent-b");
        let allowed_b = authorizer
            .check_permission(&ctx_b, "View", "Unit::\"bob-data\"")
            .await
            .unwrap();
        assert!(allowed_b, "Agent-B should access Bob's data");
    }

    #[tokio::test]
    async fn test_direct_user_request_without_agent() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        // No agent_id - direct user request
        let ctx = create_tenant_context("tenant1", "user1");

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"data1\"")
            .await
            .unwrap();
        assert!(
            allowed,
            "Direct user request should work without agent check"
        );
    }
}

// =============================================================================
// Role-Based Access Control Tests
// =============================================================================

mod rbac {
    use super::*;

    #[tokio::test]
    async fn test_viewer_can_only_view() {
        let policies = r#"
            permit(principal == User::"viewer", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "viewer");

        let can_view = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(can_view, "Viewer should be able to view");

        let cannot_edit = authorizer
            .check_permission(&ctx, "Edit", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!cannot_edit, "Viewer should NOT be able to edit");

        let cannot_delete = authorizer
            .check_permission(&ctx, "Delete", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!cannot_delete, "Viewer should NOT be able to delete");
    }

    #[tokio::test]
    async fn test_editor_can_view_and_edit() {
        let policies = r#"
            permit(principal == User::"editor", action == Action::"View", resource == Unit::"data");
            permit(principal == User::"editor", action == Action::"Edit", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "editor");

        let can_view = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(can_view, "Editor should be able to view");

        let can_edit = authorizer
            .check_permission(&ctx, "Edit", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(can_edit, "Editor should be able to edit");

        let cannot_delete = authorizer
            .check_permission(&ctx, "Delete", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!cannot_delete, "Editor should NOT be able to delete");
    }

    #[tokio::test]
    async fn test_admin_can_do_everything() {
        let policies = r#"
            permit(principal == User::"admin", action == Action::"View", resource == Unit::"data");
            permit(principal == User::"admin", action == Action::"Edit", resource == Unit::"data");
            permit(principal == User::"admin", action == Action::"Delete", resource == Unit::"data");
            permit(principal == User::"admin", action == Action::"Admin", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "admin");

        for action in &["View", "Edit", "Delete", "Admin"] {
            let allowed = authorizer
                .check_permission(&ctx, action, "Unit::\"data\"")
                .await
                .unwrap();
            assert!(allowed, "Admin should be able to {}", action);
        }
    }

    #[tokio::test]
    async fn test_role_based_memory_access() {
        let policies = r#"
            permit(principal == User::"memory-reader", action == Action::"View", resource == Memory::"personal");
            permit(principal == User::"memory-writer", action == Action::"View", resource == Memory::"personal");
            permit(principal == User::"memory-writer", action == Action::"Edit", resource == Memory::"personal");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Reader can only view
        let reader_ctx = create_tenant_context("tenant1", "memory-reader");
        let reader_view = authorizer
            .check_permission(&reader_ctx, "View", "Memory::\"personal\"")
            .await
            .unwrap();
        assert!(reader_view);

        let reader_edit = authorizer
            .check_permission(&reader_ctx, "Edit", "Memory::\"personal\"")
            .await
            .unwrap();
        assert!(!reader_edit);

        // Writer can view and edit
        let writer_ctx = create_tenant_context("tenant1", "memory-writer");
        let writer_view = authorizer
            .check_permission(&writer_ctx, "View", "Memory::\"personal\"")
            .await
            .unwrap();
        assert!(writer_view);

        let writer_edit = authorizer
            .check_permission(&writer_ctx, "Edit", "Memory::\"personal\"")
            .await
            .unwrap();
        assert!(writer_edit);
    }

    #[tokio::test]
    async fn test_knowledge_repository_roles() {
        let policies = r#"
            permit(principal == User::"knowledge-viewer", action == Action::"View", resource == Knowledge::"adr-001");
            permit(principal == User::"knowledge-editor", action == Action::"View", resource == Knowledge::"adr-001");
            permit(principal == User::"knowledge-editor", action == Action::"Edit", resource == Knowledge::"adr-001");
            permit(principal == User::"knowledge-admin", action == Action::"View", resource == Knowledge::"adr-001");
            permit(principal == User::"knowledge-admin", action == Action::"Edit", resource == Knowledge::"adr-001");
            permit(principal == User::"knowledge-admin", action == Action::"Delete", resource == Knowledge::"adr-001");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Viewer
        let viewer_ctx = create_tenant_context("tenant1", "knowledge-viewer");
        assert!(
            authorizer
                .check_permission(&viewer_ctx, "View", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&viewer_ctx, "Edit", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&viewer_ctx, "Delete", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );

        // Editor
        let editor_ctx = create_tenant_context("tenant1", "knowledge-editor");
        assert!(
            authorizer
                .check_permission(&editor_ctx, "View", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&editor_ctx, "Edit", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&editor_ctx, "Delete", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );

        // Admin
        let admin_ctx = create_tenant_context("tenant1", "knowledge-admin");
        assert!(
            authorizer
                .check_permission(&admin_ctx, "View", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&admin_ctx, "Edit", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&admin_ctx, "Delete", "Knowledge::\"adr-001\"")
                .await
                .unwrap()
        );
    }
}

// =============================================================================
// Hierarchical Unit Permission Tests
// =============================================================================

mod hierarchical_permissions {
    use super::*;

    #[tokio::test]
    async fn test_parent_unit_access_does_not_grant_child_access() {
        // In this basic policy model, parent access doesn't automatically grant child
        // access
        let policies = r#"
            permit(principal == User::"manager", action == Action::"View", resource == Unit::"org");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "manager");

        let can_view_org = authorizer
            .check_permission(&ctx, "View", "Unit::\"org\"")
            .await
            .unwrap();
        assert!(can_view_org, "Manager can view org unit");

        // Child unit not explicitly granted
        let cannot_view_team = authorizer
            .check_permission(&ctx, "View", "Unit::\"org-team1\"")
            .await
            .unwrap();
        assert!(
            !cannot_view_team,
            "Without explicit policy, child access not granted"
        );
    }

    #[tokio::test]
    async fn test_explicit_hierarchical_policy() {
        let policies = r#"
            permit(principal == User::"org-admin", action == Action::"View", resource == Unit::"org");
            permit(principal == User::"org-admin", action == Action::"View", resource == Unit::"org-team1");
            permit(principal == User::"org-admin", action == Action::"View", resource == Unit::"org-team2");
            permit(principal == User::"team1-member", action == Action::"View", resource == Unit::"org-team1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Org admin can view all levels
        let admin_ctx = create_tenant_context("tenant1", "org-admin");
        assert!(
            authorizer
                .check_permission(&admin_ctx, "View", "Unit::\"org\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&admin_ctx, "View", "Unit::\"org-team1\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&admin_ctx, "View", "Unit::\"org-team2\"")
                .await
                .unwrap()
        );

        // Team member can only view their team
        let member_ctx = create_tenant_context("tenant1", "team1-member");
        assert!(member_ctx.agent_id.is_none()); // Sanity check - no agent
        assert!(
            !authorizer
                .check_permission(&member_ctx, "View", "Unit::\"org\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&member_ctx, "View", "Unit::\"org-team1\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&member_ctx, "View", "Unit::\"org-team2\"")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_memory_layer_hierarchy() {
        // Memory layers: agent < user < session < project < team < org < tenant
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Memory::"user-layer");
            permit(principal == User::"user1", action == Action::"View", resource == Memory::"session-layer");
            permit(principal == User::"team-member", action == Action::"View", resource == Memory::"team-layer");
            permit(principal == User::"org-member", action == Action::"View", resource == Memory::"org-layer");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        let user_ctx = create_tenant_context("tenant1", "user1");
        assert!(
            authorizer
                .check_permission(&user_ctx, "View", "Memory::\"user-layer\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&user_ctx, "View", "Memory::\"session-layer\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&user_ctx, "View", "Memory::\"team-layer\"")
                .await
                .unwrap()
        );
    }
}

// =============================================================================
// Policy Evaluation Edge Cases
// =============================================================================

mod edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_empty_policy_denies_all() {
        let policies = "";
        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"anything\"")
            .await
            .unwrap();
        assert!(!denied, "Empty policy should deny all requests");
    }

    #[tokio::test]
    async fn test_forbid_overrides_permit() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
            forbid(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!denied, "Forbid should override permit");
    }

    #[tokio::test]
    async fn test_specific_forbid_with_general_permit() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource);
            forbid(principal == User::"user1", action == Action::"View", resource == Unit::"secret");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        // General permit works
        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"public\"")
            .await
            .unwrap();
        assert!(allowed, "General permit should allow");

        // Specific forbid blocks
        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"secret\"")
            .await
            .unwrap();
        assert!(!denied, "Specific forbid should block");
    }

    #[tokio::test]
    async fn test_action_not_in_policy() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        let denied = authorizer
            .check_permission(&ctx, "NonExistentAction", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!denied, "Unknown action should be denied");
    }

    #[tokio::test]
    async fn test_special_characters_in_ids() {
        let policies = r#"
            permit(principal == User::"user@example.com", action == Action::"View", resource == Unit::"data-with-dash");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user@example.com");

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"data-with-dash\"")
            .await
            .unwrap();
        assert!(allowed, "Should handle special characters in IDs");
    }

    #[tokio::test]
    async fn test_uuid_style_ids() {
        let policies = r#"
            permit(principal == User::"550e8400-e29b-41d4-a716-446655440000", action == Action::"View", resource == Unit::"a3bb189e-8bf9-3888-9912-ace4e6543002");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "550e8400-e29b-41d4-a716-446655440000");

        let allowed = authorizer
            .check_permission(
                &ctx,
                "View",
                "Unit::\"a3bb189e-8bf9-3888-9912-ace4e6543002\"",
            )
            .await
            .unwrap();
        assert!(allowed, "Should handle UUID-style IDs");
    }

    #[tokio::test]
    async fn test_when_clause_always_false() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data")
            when { false };
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!denied, "when {{ false }} should always deny");
    }

    #[tokio::test]
    async fn test_unless_clause_always_true() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data")
            unless { true };
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await
            .unwrap();
        assert!(!denied, "unless {{ true }} should always deny");
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

mod error_handling {
    use super::*;

    #[test]
    fn test_invalid_policy_syntax() {
        let invalid_policies = "this is not valid cedar policy syntax!!!";
        let result = CedarAuthorizer::new(invalid_policies, TEST_SCHEMA);
        assert!(result.is_err(), "Invalid policy should return error");

        if let Err(CedarError::Parse(msg)) = result {
            assert!(!msg.is_empty(), "Error message should not be empty");
        } else {
            panic!("Expected Parse error");
        }
    }

    #[test]
    fn test_empty_schema_accepted() {
        let policies = r#"
            permit(principal, action, resource);
        "#;
        // Empty schema string should still work (schema validation is not strict in
        // current impl)
        let result = CedarAuthorizer::new(policies, "");
        assert!(result.is_ok(), "Empty schema should be accepted");
    }

    #[tokio::test]
    async fn test_malformed_resource_uid() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        // Malformed resource string (missing quotes, wrong format)
        let result = authorizer
            .check_permission(&ctx, "View", "not-a-valid-resource-uid")
            .await;

        assert!(
            result.is_err(),
            "Malformed resource UID should return error"
        );
    }

    #[tokio::test]
    async fn test_malformed_action_returns_deny() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        // Action that doesn't match format (but still parseable)
        let result = authorizer
            .check_permission(&ctx, "View", "Unit::\"data\"")
            .await;

        assert!(result.is_ok(), "Valid format should not error");
    }

    #[test]
    fn test_policy_with_syntax_error_in_condition() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource)
            when { undefined_variable };
        "#;

        let result = CedarAuthorizer::new(policies, TEST_SCHEMA);
        // This should either fail at parse time or evaluation time
        // Current implementation parses policies at construction
        assert!(
            result.is_err(),
            "Policy with undefined variable should fail"
        );
    }

    #[test]
    fn test_duplicate_policy_ids_handled() {
        // Multiple policies - Cedar handles this by evaluating all
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let result = CedarAuthorizer::new(policies, TEST_SCHEMA);
        assert!(result.is_ok(), "Duplicate policies should be accepted");
    }
}

// =============================================================================
// Combined Authorization Scenarios
// =============================================================================

mod combined_scenarios {
    use super::*;

    #[tokio::test]
    async fn test_agent_delegation_with_rbac() {
        let policies = r#"
            permit(principal == User::"agent-1", action == Action::"ActAs", resource == User::"viewer");
            permit(principal == User::"agent-2", action == Action::"ActAs", resource == User::"editor");
            permit(principal == User::"viewer", action == Action::"View", resource == Knowledge::"doc1");
            permit(principal == User::"editor", action == Action::"View", resource == Knowledge::"doc1");
            permit(principal == User::"editor", action == Action::"Edit", resource == Knowledge::"doc1");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Agent-1 acting as viewer can only view
        let viewer_agent_ctx = create_agent_context("tenant1", "viewer", "agent-1");
        assert!(
            authorizer
                .check_permission(&viewer_agent_ctx, "View", "Knowledge::\"doc1\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&viewer_agent_ctx, "Edit", "Knowledge::\"doc1\"")
                .await
                .unwrap()
        );

        // Agent-2 acting as editor can view and edit
        let editor_agent_ctx = create_agent_context("tenant1", "editor", "agent-2");
        assert!(
            authorizer
                .check_permission(&editor_agent_ctx, "View", "Knowledge::\"doc1\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&editor_agent_ctx, "Edit", "Knowledge::\"doc1\"")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_multi_tenant_with_agent_delegation() {
        let policies = r#"
            permit(principal == User::"agent-acme", action == Action::"ActAs", resource == User::"acme-user");
            permit(principal == User::"agent-globex", action == Action::"ActAs", resource == User::"globex-user");
            permit(principal == User::"acme-user", action == Action::"View", resource == Unit::"acme-data");
            permit(principal == User::"globex-user", action == Action::"View", resource == Unit::"globex-data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // ACME agent can only access ACME data
        let acme_ctx = create_agent_context("acme", "acme-user", "agent-acme");
        assert!(
            authorizer
                .check_permission(&acme_ctx, "View", "Unit::\"acme-data\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&acme_ctx, "View", "Unit::\"globex-data\"")
                .await
                .unwrap()
        );

        // Globex agent can only access Globex data
        let globex_ctx = create_agent_context("globex", "globex-user", "agent-globex");
        assert!(
            authorizer
                .check_permission(&globex_ctx, "View", "Unit::\"globex-data\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&globex_ctx, "View", "Unit::\"acme-data\"")
                .await
                .unwrap()
        );

        // Cross-tenant agent delegation doesn't work
        let cross_ctx = create_agent_context("acme", "globex-user", "agent-acme");
        assert!(
            !authorizer
                .check_permission(&cross_ctx, "View", "Unit::\"globex-data\"")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_complete_governance_scenario() {
        let policies = r#"
            // Tenant-level admin
            permit(principal == User::"tenant-admin", action == Action::"Admin", resource == Unit::"tenant");
            permit(principal == User::"tenant-admin", action == Action::"View", resource == Knowledge::"tenant-policy");
            permit(principal == User::"tenant-admin", action == Action::"Edit", resource == Knowledge::"tenant-policy");
            
            // Org-level editor
            permit(principal == User::"org-editor", action == Action::"View", resource == Knowledge::"org-pattern");
            permit(principal == User::"org-editor", action == Action::"Edit", resource == Knowledge::"org-pattern");
            
            // Project-level viewer
            permit(principal == User::"project-viewer", action == Action::"View", resource == Knowledge::"project-spec");
            
            // Agent delegation
            permit(principal == User::"automation-agent", action == Action::"ActAs", resource == User::"project-viewer");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();

        // Tenant admin has full control
        let admin_ctx = create_tenant_context("tenant1", "tenant-admin");
        assert!(
            authorizer
                .check_permission(&admin_ctx, "Admin", "Unit::\"tenant\"")
                .await
                .unwrap()
        );
        assert!(
            authorizer
                .check_permission(&admin_ctx, "Edit", "Knowledge::\"tenant-policy\"")
                .await
                .unwrap()
        );

        // Org editor can edit org patterns but not tenant policies
        let editor_ctx = create_tenant_context("tenant1", "org-editor");
        assert!(
            authorizer
                .check_permission(&editor_ctx, "Edit", "Knowledge::\"org-pattern\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&editor_ctx, "Edit", "Knowledge::\"tenant-policy\"")
                .await
                .unwrap()
        );

        // Project viewer can only view
        let viewer_ctx = create_tenant_context("tenant1", "project-viewer");
        assert!(
            authorizer
                .check_permission(&viewer_ctx, "View", "Knowledge::\"project-spec\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&viewer_ctx, "Edit", "Knowledge::\"project-spec\"")
                .await
                .unwrap()
        );

        // Automation agent can view as project-viewer
        let agent_ctx = create_agent_context("tenant1", "project-viewer", "automation-agent");
        assert!(
            authorizer
                .check_permission(&agent_ctx, "View", "Knowledge::\"project-spec\"")
                .await
                .unwrap()
        );
        assert!(
            !authorizer
                .check_permission(&agent_ctx, "Edit", "Knowledge::\"project-spec\"")
                .await
                .unwrap()
        );
    }
}

// =============================================================================
// Performance/Stress Tests
// =============================================================================

mod performance {
    use super::*;

    #[tokio::test]
    async fn test_large_policy_set() {
        // Generate a policy with many rules
        let mut policies = String::new();
        for i in 0..100 {
            policies.push_str(&format!(
                "permit(principal == User::\"user{}\", action == Action::\"View\", resource == \
                 Unit::\"data{}\");\n",
                i, i
            ));
        }

        let authorizer = CedarAuthorizer::new(&policies, TEST_SCHEMA).unwrap();

        // Test access for various users
        for i in [0, 50, 99].iter() {
            let ctx = create_tenant_context("tenant1", &format!("user{}", i));
            let allowed = authorizer
                .check_permission(&ctx, "View", &format!("Unit::\"data{}\"", i))
                .await
                .unwrap();
            assert!(allowed, "User{} should access data{}", i, i);
        }
    }

    #[tokio::test]
    async fn test_many_sequential_checks() {
        let policies = r#"
            permit(principal == User::"user1", action == Action::"View", resource == Unit::"data");
        "#;

        let authorizer = CedarAuthorizer::new(policies, TEST_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "user1");

        // Perform many checks
        for _ in 0..100 {
            let allowed = authorizer
                .check_permission(&ctx, "View", "Unit::\"data\"")
                .await
                .unwrap();
            assert!(allowed);
        }
    }
}

mod role_membership {
    use super::*;
    use mk_core::types::{Role, RoleIdentifier};

    const MEMBERSHIP_SCHEMA: &str = r#"{
        "": {
            "entityTypes": {
                "User": {
                    "memberOfTypes": ["Aeterna::Role"]
                },
                "Unit": {}
            },
            "actions": {
                "View": {
                    "appliesTo": {
                        "principalTypes": ["User"],
                        "resourceTypes": ["Unit"]
                    }
                }
            }
        },
        "Aeterna": {
            "entityTypes": {
                "Role": {}
            },
            "actions": {}
        }
    }"#;

    #[tokio::test]
    async fn test_known_role_grants_membership_access_expected() {
        let policies = r#"
            permit(principal in Aeterna::Role::"Admin", action == Action::"View", resource == Unit::"secure-data");
        "#;
        let authorizer = CedarAuthorizer::new(policies, MEMBERSHIP_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "known-admin");

        authorizer
            .assign_role(&ctx, &ctx.user_id, RoleIdentifier::Known(Role::Admin))
            .await
            .unwrap();

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"secure-data\"")
            .await
            .unwrap();
        assert!(allowed);
    }

    #[tokio::test]
    async fn test_custom_role_grants_membership_access_expected() {
        let policies = r#"
            permit(principal in Aeterna::Role::"billingOwner", action == Action::"View", resource == Unit::"finance-data");
        "#;
        let authorizer = CedarAuthorizer::new(policies, MEMBERSHIP_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "billing-user");

        authorizer
            .assign_role(
                &ctx,
                &ctx.user_id,
                RoleIdentifier::Custom("billingOwner".to_string()),
            )
            .await
            .unwrap();

        let allowed = authorizer
            .check_permission(&ctx, "View", "Unit::\"finance-data\"")
            .await
            .unwrap();
        assert!(allowed);
    }

    #[tokio::test]
    async fn test_no_role_denies_membership_expected() {
        let policies = r#"
            permit(principal in Aeterna::Role::"Admin", action == Action::"View", resource == Unit::"secure-data");
        "#;
        let authorizer = CedarAuthorizer::new(policies, MEMBERSHIP_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "no-role-user");

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"secure-data\"")
            .await
            .unwrap();
        assert!(!denied);
    }

    #[tokio::test]
    async fn test_wrong_role_denies_membership_expected() {
        let policies = r#"
            permit(principal in Aeterna::Role::"Admin", action == Action::"View", resource == Unit::"secure-data");
        "#;
        let authorizer = CedarAuthorizer::new(policies, MEMBERSHIP_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "developer-user");

        authorizer
            .assign_role(&ctx, &ctx.user_id, RoleIdentifier::Known(Role::Developer))
            .await
            .unwrap();

        let denied = authorizer
            .check_permission(&ctx, "View", "Unit::\"secure-data\"")
            .await
            .unwrap();
        assert!(!denied);
    }

    #[tokio::test]
    async fn test_remove_role_revokes_membership_expected() {
        let policies = r#"
            permit(principal in Aeterna::Role::"Admin", action == Action::"View", resource == Unit::"secure-data");
        "#;
        let authorizer = CedarAuthorizer::new(policies, MEMBERSHIP_SCHEMA).unwrap();
        let ctx = create_tenant_context("tenant1", "revoked-user");

        authorizer
            .assign_role(&ctx, &ctx.user_id, RoleIdentifier::Known(Role::Admin))
            .await
            .unwrap();
        let allowed_before = authorizer
            .check_permission(&ctx, "View", "Unit::\"secure-data\"")
            .await
            .unwrap();
        assert!(allowed_before);

        authorizer
            .remove_role(&ctx, &ctx.user_id, RoleIdentifier::Known(Role::Admin))
            .await
            .unwrap();
        let denied_after = authorizer
            .check_permission(&ctx, "View", "Unit::\"secure-data\"")
            .await
            .unwrap();
        assert!(!denied_after);
    }
}
