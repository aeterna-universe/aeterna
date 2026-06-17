use clap::{Args, Subcommand};
use serde_json::json;

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum PermissionsCommand {
    #[command(about = "Show the role-to-permission matrix from the active Cedar RBAC policy")]
    Matrix(PermissionsMatrixArgs),

    #[command(about = "Show effective permissions for a user or role at a resource")]
    Effective(PermissionsEffectiveArgs),
}

#[derive(Args)]
pub struct PermissionsMatrixArgs {
    /// Filter output to a single role (admin, architect, techlead, developer, agent, platform_admin)
    #[arg(short, long)]
    pub role: Option<String>,

    /// Target a specific tenant (PlatformAdmin only — for cross-tenant operations)
    #[arg(long)]
    pub target_tenant: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct PermissionsEffectiveArgs {
    /// User ID to inspect (required)
    #[arg(long)]
    pub user_id: String,

    /// Cedar resource expression to evaluate against (defaults to tenant resource)
    #[arg(long)]
    pub resource: Option<String>,

    /// Comma-separated list of actions to check (defaults to all known actions)
    #[arg(long)]
    pub actions: Option<String>,

    /// Evaluate as if the user holds this role (skips live role lookup)
    #[arg(long)]
    pub role: Option<String>,

    /// Target a specific tenant (PlatformAdmin only — for cross-tenant operations)
    #[arg(long)]
    pub target_tenant: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cmd: PermissionsCommand) -> anyhow::Result<()> {
    match cmd {
        PermissionsCommand::Matrix(args) => run_matrix(args).await,
        PermissionsCommand::Effective(args) => run_effective(args).await,
    }
}

// ---------------------------------------------------------------------------
// Live client helper
// ---------------------------------------------------------------------------

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    get_live_client_for(None).await
}

async fn get_live_client_for(
    target_tenant: Option<&str>,
) -> Option<crate::client::AeternaClient> {
    let resolved = crate::profile::load_resolved(None, None);
    if let Ok(ref r) = resolved {
        let client = crate::client::AeternaClient::from_profile(r).await.ok()?;
        if let Some(tenant) = target_tenant {
            Some(client.with_target_tenant(tenant))
        } else {
            Some(client)
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Fallback helper
// ---------------------------------------------------------------------------

fn perm_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("Permission inspection requires a live control-plane backend to read the active Cedar policy bundle")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
        .suggest("aeterna admin health")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

// ---------------------------------------------------------------------------
// run_matrix
// ---------------------------------------------------------------------------

async fn run_matrix(args: PermissionsMatrixArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client_for(args.target_tenant.as_deref()).await {
        let result = client.permissions_matrix().await.map_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string(), "operation": "permissions_matrix"})
                    )
                    .unwrap()
                );
            } else {
                ux_error::UxError::new(&e.to_string())
                    .fix("Ensure you have Admin or PlatformAdmin role")
                    .display();
            }
            e
        })?;

        // Optionally filter to a single role
        let filtered = if let Some(ref role_filter) = args.role {
            if let Some(matrix) = result.get("matrix").and_then(|m| m.as_object()) {
                if let Some(perms) = matrix.get(role_filter) {
                    json!({
                        "success": true,
                        "role": role_filter,
                        "permissions": perms,
                    })
                } else {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": format!("Role '{}' not found in matrix", role_filter)})
                            )?
                        );
                        anyhow::bail!("Role '{}' not found in matrix", role_filter);
                    } else {
                        ux_error::UxError::new(&format!(
                            "Role '{}' not found in permission matrix",
                            role_filter
                        ))
                        .fix("Use one of: admin, architect, techlead, developer, agent, platform_admin")
                        .display();
                        anyhow::bail!("Unknown role: {}", role_filter);
                    }
                }
            } else {
                result.clone()
            }
        } else {
            result.clone()
        };

        if args.json {
            println!("{}", serde_json::to_string_pretty(&filtered)?);
        } else {
            if let Some(ref t) = args.target_tenant {
                output::info(&format!("Targeting tenant: {t}"));
                println!();
            }
            output::header("Role-to-Permission Matrix");
            println!();

            // Print table: role | permissions
            if let Some(matrix) = filtered.get("matrix").and_then(|m| m.as_object()) {
                for (role, perms) in matrix {
                    let perm_list: Vec<&str> = perms
                        .as_array()
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();
                    println!("  \x1b[1m{role}\x1b[0m");
                    for p in &perm_list {
                        println!("    ✓  {p}");
                    }
                    println!();
                }
            } else if let Some(perms) = filtered.get("permissions") {
                // Single-role filtered result
                let role = filtered
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                println!("  \x1b[1m{role}\x1b[0m");
                if let Some(arr) = perms.as_array() {
                    for p in arr {
                        println!("    ✓  {}", p.as_str().unwrap_or("?"));
                    }
                }
                println!();
            } else {
                println!("{filtered}");
            }

            output::hint("Use --role <name> to filter to a single role");
            output::hint("Use --json for machine-readable output");
        }
        return Ok(());
    }

    // Fallback: no server
    if args.json {
        let output = json!({
            "operation": "permissions_matrix",
            "success": false,
            "error": "server_not_connected",
            "message": "Permission matrix requires a live Aeterna server connection."
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        anyhow::bail!("Aeterna server not connected for operation: permissions_matrix");
    }

    perm_server_required(
        "permissions_matrix",
        "Cannot show permission matrix: server not connected",
    )
}

// ---------------------------------------------------------------------------
// run_effective
// ---------------------------------------------------------------------------

async fn run_effective(args: PermissionsEffectiveArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client_for(args.target_tenant.as_deref()).await {
        let result = client
            .permissions_effective(
                &args.user_id,
                args.resource.as_deref(),
                args.actions.as_deref(),
                args.role.as_deref(),
            )
            .await
            .map_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "permissions_effective", "userId": args.user_id})
                        )
                        .unwrap()
                    );
                } else {
                    ux_error::UxError::new(&e.to_string())
                        .fix("Ensure you have Admin or PlatformAdmin role")
                        .fix("Verify the user ID exists: aeterna user show <id>")
                        .display();
                }
                e
            })?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            let user_id = result
                .get("userId")
                .and_then(|v| v.as_str())
                .unwrap_or(&args.user_id);
            let resource = result
                .get("resource")
                .and_then(|v| v.as_str())
                .unwrap_or("(default)");

            if let Some(ref t) = args.target_tenant {
                output::info(&format!("Targeting tenant: {t}"));
                println!();
            }
            output::header("Effective Permissions");
            println!();
            println!("  User:     {user_id}");
            println!("  Resource: {resource}");
            if let Some(ref role) = args.role {
                println!("  Role:     {role} (evaluated as)");
            }
            println!();

            if let Some(granted) = result.get("granted").and_then(|v| v.as_array()) {
                if granted.is_empty() {
                    println!("  GRANTED   (none)");
                } else {
                    println!("  GRANTED");
                    for p in granted {
                        println!("    ✓  {}", p.as_str().unwrap_or("?"));
                    }
                }
            }
            println!();

            if let Some(denied) = result.get("denied").and_then(|v| v.as_array()) {
                if !denied.is_empty() {
                    println!("  DENIED");
                    for p in denied {
                        println!("    ✗  {}", p.as_str().unwrap_or("?"));
                    }
                    println!();
                }
            }

            output::hint("Use --role <name> to evaluate as a specific role");
            output::hint("Use --actions a,b,c to check a subset of actions");
        }
        return Ok(());
    }

    // Fallback: no server
    if args.json {
        let output = json!({
            "operation": "permissions_effective",
            "userId": args.user_id,
            "role": args.role,
            "resource": args.resource,
            "success": false,
            "error": "server_not_connected",
            "message": "Effective permission inspection requires a live Aeterna server connection."
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        anyhow::bail!("Aeterna server not connected for operation: permissions_effective");
    }

    perm_server_required(
        "permissions_effective",
        &format!(
            "Cannot inspect permissions for user '{}': server not connected",
            args.user_id
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_matrix_args_defaults() {
        let args = PermissionsMatrixArgs {
            role: None,
            target_tenant: None,
            json: false,
        };
        assert!(args.role.is_none());
        assert!(!args.json);
    }

    #[test]
    fn test_permissions_matrix_args_with_role() {
        let args = PermissionsMatrixArgs {
            role: Some("admin".to_string()),
            target_tenant: None,
            json: true,
        };
        assert_eq!(args.role.as_deref(), Some("admin"));
        assert!(args.json);
    }

    #[test]
    fn test_permissions_matrix_args_all_roles() {
        let roles = ["admin", "architect", "techlead", "developer", "agent", "platform_admin"];
        for role_name in roles {
            let args = PermissionsMatrixArgs {
                role: Some(role_name.to_string()),
                target_tenant: None,
                json: false,
            };
            assert_eq!(args.role.as_deref(), Some(role_name));
        }
    }

    #[test]
    fn test_permissions_effective_args_minimal() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_abc".to_string(),
            resource: None,
            actions: None,
            role: None,
            target_tenant: None,
            json: false,
        };
        assert_eq!(args.user_id, "user_abc");
        assert!(args.resource.is_none());
        assert!(args.role.is_none());
    }

    #[test]
    fn test_permissions_effective_args_full() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_abc".to_string(),
            resource: Some("Aeterna::Tenant::\"acme\"".to_string()),
            actions: Some("ViewMemory,WriteMemory,ManageUsers".to_string()),
            role: Some("admin".to_string()),
            target_tenant: None,
            json: true,
        };
        assert_eq!(args.user_id, "user_abc");
        assert!(args.resource.is_some());
        assert_eq!(args.role.as_deref(), Some("admin"));
        assert!(args.json);
    }

    #[test]
    fn test_permissions_effective_args_role_override() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_xyz".to_string(),
            resource: None,
            actions: None,
            role: Some("developer".to_string()),
            target_tenant: None,
            json: false,
        };
        assert_eq!(args.role.as_deref(), Some("developer"));
    }

    #[test]
    fn test_permissions_effective_args_actions_csv() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_xyz".to_string(),
            resource: None,
            actions: Some("ViewMemory,ManageUsers,AssignRoles".to_string()),
            role: None,
            target_tenant: None,
            json: false,
        };
        let actions = args.actions.as_deref().unwrap();
        let parts: Vec<&str> = actions.split(',').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.contains(&"ViewMemory"));
    }

    #[test]
    fn test_permissions_effective_args_platform_admin_role() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_admin".to_string(),
            resource: Some("Aeterna::Tenant::\"acme\"".to_string()),
            actions: None,
            role: Some("platform_admin".to_string()),
            target_tenant: None,
            json: false,
        };
        assert_eq!(args.role.as_deref(), Some("platform_admin"));
    }
    #[test]
    fn test_permissions_matrix_args_target_tenant() {
        let args = PermissionsMatrixArgs {
            role: None,
            target_tenant: Some("tenant-acme".to_string()),
            json: false,
        };
        assert_eq!(args.target_tenant.as_deref(), Some("tenant-acme"));
    }

    #[test]
    fn test_permissions_effective_args_target_tenant() {
        let args = PermissionsEffectiveArgs {
            user_id: "user_abc".to_string(),
            resource: None,
            actions: None,
            role: None,
            target_tenant: Some("tenant-beta".to_string()),
            json: false,
        };
        assert_eq!(args.target_tenant.as_deref(), Some("tenant-beta"));
    }

}
