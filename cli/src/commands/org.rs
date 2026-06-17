use clap::{Args, Subcommand};
use context::ContextResolver;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum OrgCommand {
    #[command(about = "Create a new organization")]
    Create(OrgCreateArgs),

    #[command(about = "List organizations in your tenant")]
    List(OrgListArgs),

    #[command(about = "Show organization details")]
    Show(OrgShowArgs),

    #[command(about = "Manage organization members")]
    Members(OrgMembersArgs),

    #[command(about = "Set default organization for current context")]
    Use(OrgUseArgs),
}

#[derive(Args)]
pub struct OrgCreateArgs {
    /// Organization name
    pub name: String,

    /// Organization description
    #[arg(short, long)]
    pub description: Option<String>,

    /// Parent tenant/root ID (auto-detected if not provided)
    #[arg(long)]
    pub tenant_root: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - show what would be created
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct OrgListArgs {
    /// Filter by tenant/root
    #[arg(long)]
    pub tenant_root: Option<String>,

    /// Show all organizations you have access to
    #[arg(long)]
    pub all: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// #44.d §6 — cross-tenant scoping (`--all-tenants` / `--tenant <slug>`).
    #[command(flatten)]
    pub scope: super::tenant_scope::TenantScopeArgs,
}

#[derive(Args)]
pub struct OrgShowArgs {
    /// Organization ID (uses current context if not provided)
    pub org_id: Option<String>,

    /// Show full details including policies and teams
    #[arg(short, long)]
    pub verbose: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct OrgMembersArgs {
    /// Organization ID (uses current context if not provided)
    #[arg(long)]
    pub org: Option<String>,

    /// Add member by user ID
    #[arg(long)]
    pub add: Option<String>,

    /// Remove member by user ID
    #[arg(long)]
    pub remove: Option<String>,

    /// Set role for a member (with --add or --set-role)
    #[arg(long, value_name = "USER_ID")]
    pub set_role: Option<String>,

    /// Role to assign (developer, techlead, architect, admin)
    #[arg(long)]
    pub role: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct OrgUseArgs {
    /// Organization ID to set as default
    pub org_id: String,
}

pub async fn run(cmd: OrgCommand) -> anyhow::Result<()> {
    match cmd {
        OrgCommand::Create(args) => run_create(args).await,
        OrgCommand::List(args) => run_list(args).await,
        OrgCommand::Show(args) => run_show(args).await,
        OrgCommand::Members(args) => run_members(args).await,
        OrgCommand::Use(args) => run_use(args).await,
    }
}

fn org_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("This command requires a live control-plane backend")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
        .suggest("aeterna admin health")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    crate::backend::connect()
        .await
        .ok()
        .map(|(client, _)| client)
}

fn normalize_org_role(role: &str, suggestion: &str) -> anyhow::Result<String> {
    let role = role.to_lowercase();
    let valid_roles = ["developer", "techlead", "architect", "admin"];
    if valid_roles.contains(&role.as_str()) {
        Ok(role)
    } else {
        ux_error::UxError::new(format!("Invalid role: '{role}'"))
            .why("Role determines user permissions within the organization")
            .fix("Use one of: developer, techlead, architect, admin")
            .suggest(suggestion)
            .display();
        anyhow::bail!("Invalid role")
    }
}

fn resolve_current_org(explicit_org: Option<String>) -> anyhow::Result<String> {
    if let Some(org_id) = explicit_org {
        return Ok(org_id);
    }

    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;
    if let Some(org) = resolved.org_id.as_ref() {
        Ok(org.value.clone())
    } else {
        ux_error::UxError::new("No organization specified")
            .why("This command needs an organization ID or an active org context")
            .fix("Pass an org ID explicitly")
            .suggest("aeterna org show <org-id>")
            .display();
        anyhow::bail!("No organization specified")
    }
}

fn set_context_value(key: &str, value: &str) -> anyhow::Result<()> {
    let aeterna_dir = Path::new(".aeterna");
    let context_file = aeterna_dir.join("context.toml");

    let mut config = if context_file.exists() {
        let content = fs::read_to_string(&context_file)?;
        toml::from_str::<toml::Value>(&content)
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    if let Some(table) = config.as_table_mut() {
        table.insert(key.to_string(), toml::Value::String(value.to_string()));
    }

    fs::create_dir_all(aeterna_dir)?;
    fs::write(&context_file, toml::to_string_pretty(&config)?)?;
    Ok(())
}

fn get_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(|v| v.as_str()) {
            return Some(found);
        }
    }
    None
}

async fn run_create(args: OrgCreateArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let tenant_root_id = args
        .tenant_root
        .clone()
        .unwrap_or_else(|| resolved.tenant_id.value.clone());

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "org_create",
                "org": {
                    "name": args.name,
                    "description": args.description,
                    "tenantRoot": tenant_root_id,
                },
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                },
                "nextSteps": [
                    "Review organization configuration",
                    "Run without --dry-run to create",
                    "Add members with 'aeterna org members --add <user>'",
                    "Create teams with 'aeterna team create <name> --org <org-id>'"
                ]
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Organization Create (Dry Run)");
            println!();
            println!("  Name:        {}", args.name);
            if let Some(ref desc) = args.description {
                println!("  Description: {desc}");
            }
            println!("  Tenant Root:     {tenant_root_id}");
            println!();
            output::info("Dry run mode - organization not created.");
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let body = json!({
            "name": args.name,
            "description": args.description,
            "tenantRoot": tenant_root_id,
        });
        let result = client.org_create(&body).await.inspect_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string()})
                    )
                    .unwrap()
                );
            } else {
                ux_error::UxError::new(e.to_string())
                    .fix("Run: aeterna auth login")
                    .display();
            }
        })?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header("Organization Created");
            println!();
            println!("  ID:      {}", get_str(&result, &["id"]).unwrap_or("?"));
            println!("  Name:    {}", get_str(&result, &["name"]).unwrap_or("?"));
            println!(
                "  Tenant Root: {}",
                get_str(&result, &["parentId", "parent_id"]).unwrap_or("?")
            );
            if let Some(description) = result
                .get("metadata")
                .and_then(|v| v.get("description"))
                .and_then(|v| v.as_str())
            {
                println!("  Description: {description}");
            }
            println!();
            output::hint("Add members with 'aeterna org members --add <user-id> --role <role>'");
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "org_create"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: org_create");
    }

    org_server_required(
        "org_create",
        "Cannot create organization: server not connected",
    )
}

async fn run_list(args: OrgListArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client().await {
        let scope = args.scope.to_query_param();
        let result = client
            .org_list(args.tenant_root.as_deref(), args.all, scope.as_deref())
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string()})
                        )
                        .unwrap()
                    );
                } else {
                    ux_error::UxError::new(e.to_string())
                        .fix("Run: aeterna auth login")
                        .display();
                }
            })?;

        if args.json {
            // JSON mode emits the server payload verbatim — envelope or
            // array. Automation callers need the exact server shape so
            // they can distinguish scope=all / scope=tenant themselves.
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            let payload = super::tenant_scope::ListPayload::from_json(&result);
            output::header("Organizations");
            if let Some(banner) = &payload.banner {
                println!("  {banner}");
            }
            println!();
            if payload.items.is_empty() {
                println!("  (no organizations found)");
            } else {
                for org in &payload.items {
                    let id = get_str(org, &["id"]).unwrap_or("?");
                    let name = get_str(org, &["name"]).unwrap_or("?");
                    let tenant_root = get_str(org, &["parentId", "parent_id"]).unwrap_or("-");
                    // In cross-tenant views decorate with the tenant slug
                    // so operators can see WHICH tenant each row belongs
                    // to — otherwise the listing is ambiguous (two orgs
                    // can legitimately share a name across tenants).
                    if args.scope.is_cross_tenant() {
                        let tenant = get_str(org, &["tenantSlug"]).unwrap_or("-");
                        println!("  [{tenant:<16}] {id:<24} {name:<32} {tenant_root}");
                    } else {
                        println!("  {id:<24} {name:<32} {tenant_root}");
                    }
                }
            }
            println!();
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "org_list"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: org_list");
    }

    org_server_required(
        "org_list",
        "Cannot list organizations: server not connected",
    )
}

async fn run_show(args: OrgShowArgs) -> anyhow::Result<()> {
    let org_id = resolve_current_org(args.org_id.clone())?;

    if let Some(client) = get_live_client().await {
        let result = client.org_show(&org_id).await.inspect_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string()})
                    )
                    .unwrap()
                );
            } else {
                ux_error::UxError::new(e.to_string())
                    .fix("Run: aeterna auth login")
                    .display();
            }
        })?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header(&format!("Organization: {org_id}"));
            println!();
            println!("  ID:       {}", get_str(&result, &["id"]).unwrap_or("?"));
            println!("  Name:     {}", get_str(&result, &["name"]).unwrap_or("?"));
            println!(
                "  Tenant Root:  {}",
                get_str(&result, &["parentId", "parent_id"]).unwrap_or("?")
            );
            println!(
                "  Type:     {}",
                get_str(&result, &["unitType", "unit_type"]).unwrap_or("organization")
            );
            if let Some(description) = result
                .get("metadata")
                .and_then(|v| v.get("description"))
                .and_then(|v| v.as_str())
            {
                println!("  Description: {description}");
            }
            if args.verbose {
                if let Some(source_owner) = get_str(&result, &["sourceOwner", "source_owner"]) {
                    println!("  Source:   {source_owner}");
                }
                if let Some(metadata) = result.get("metadata") {
                    println!("  Metadata: {}", serde_json::to_string_pretty(metadata)?);
                }
            }
            println!();
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "org_show",
                "orgId": org_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: org_show");
    }

    org_server_required(
        "org_show",
        &format!("Cannot show organization '{org_id}': server not connected"),
    )
}

async fn run_members(args: OrgMembersArgs) -> anyhow::Result<()> {
    let org_id = resolve_current_org(args.org.clone())?;

    if let Some(ref user_to_add) = args.add {
        let role = normalize_org_role(
            args.role.as_deref().unwrap_or("developer"),
            &format!("aeterna org members --add {user_to_add} --role developer"),
        )?;

        if let Some(client) = get_live_client().await {
            let result = client
                .org_member_add(&org_id, &json!({"userId": user_to_add, "role": role}))
                .await
                .inspect_err(|e| {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": e.to_string()})
                            )
                            .unwrap()
                        );
                    } else {
                        ux_error::UxError::new(e.to_string())
                            .fix("Run: aeterna auth login")
                            .display();
                    }
                })?;

            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                output::header("Organization Member Added");
                println!();
                println!("  Organization: {org_id}");
                println!(
                    "  User:         {}",
                    get_str(&result, &["userId", "user_id"]).unwrap_or(user_to_add)
                );
                println!(
                    "  Role:         {}",
                    get_str(&result, &["role"]).unwrap_or(role.as_str())
                );
                println!();
            }
            return Ok(());
        }

        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "org_member_add",
                    "orgId": org_id,
                    "userId": user_to_add
                }))?
            );
            anyhow::bail!("Aeterna server not connected for operation: org_member_add");
        }

        return org_server_required(
            "org_member_add",
            &format!("Cannot add member to organization '{org_id}': server not connected"),
        );
    }

    if let Some(ref user_to_remove) = args.remove {
        if let Some(client) = get_live_client().await {
            let result = client
                .org_member_remove(&org_id, user_to_remove)
                .await
                .inspect_err(|e| {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": e.to_string()})
                            )
                            .unwrap()
                        );
                    } else {
                        ux_error::UxError::new(e.to_string())
                            .fix("Run: aeterna auth login")
                            .display();
                    }
                })?;

            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                output::header("Organization Member Removed");
                println!();
                println!("  Organization: {org_id}");
                println!("  User:         {user_to_remove}");
                println!();
            }
            return Ok(());
        }

        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "org_member_remove",
                    "orgId": org_id,
                    "userId": user_to_remove
                }))?
            );
            anyhow::bail!("Aeterna server not connected for operation: org_member_remove");
        }

        return org_server_required(
            "org_member_remove",
            &format!("Cannot remove member from organization '{org_id}': server not connected"),
        );
    }

    if let Some(ref user_id) = args.set_role {
        let role_value = args.role.clone().ok_or_else(|| {
            ux_error::UxError::new("Missing --role for --set-role")
                .why("Must specify which role to assign")
                .fix("Add --role with the desired role")
                .suggest(format!(
                    "aeterna org members --set-role {user_id} --role techlead"
                ))
                .display();
            anyhow::anyhow!("Missing role")
        })?;
        let role = normalize_org_role(
            &role_value,
            &format!("aeterna org members --set-role {user_id} --role techlead"),
        )?;

        if let Some(client) = get_live_client().await {
            let result = client
                .org_member_set_role(&org_id, user_id, &json!({"role": role}))
                .await
                .inspect_err(|e| {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": e.to_string()})
                            )
                            .unwrap()
                        );
                    } else {
                        ux_error::UxError::new(e.to_string())
                            .fix("Run: aeterna auth login")
                            .display();
                    }
                })?;

            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                output::header("Organization Member Role Updated");
                println!();
                println!("  Organization: {org_id}");
                println!(
                    "  User:         {}",
                    get_str(&result, &["userId", "user_id"]).unwrap_or(user_id)
                );
                println!(
                    "  New Role:     {}",
                    get_str(&result, &["role"]).unwrap_or(role.as_str())
                );
                println!();
            }
            return Ok(());
        }

        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "org_member_set_role",
                    "orgId": org_id,
                    "userId": user_id
                }))?
            );
            anyhow::bail!("Aeterna server not connected for operation: org_member_set_role");
        }

        return org_server_required(
            "org_member_set_role",
            &format!("Cannot update member role in organization '{org_id}': server not connected"),
        );
    }

    if let Some(client) = get_live_client().await {
        let result = client.org_members_list(&org_id).await.inspect_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string()})
                    )
                    .unwrap()
                );
            } else {
                ux_error::UxError::new(e.to_string())
                    .fix("Run: aeterna auth login")
                    .display();
            }
        })?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header(&format!("Members of: {org_id}"));
            println!();
            if let Some(members) = result.as_array() {
                if members.is_empty() {
                    println!("  (no members found)");
                } else {
                    for member in members {
                        let user = get_str(member, &["userId", "user_id"]).unwrap_or("?");
                        let role = get_str(member, &["role"]).unwrap_or("?");
                        println!("  {user:<36} {role}");
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            println!();
            output::hint("Add member: aeterna org members --add <user> --role <role>");
            output::hint("Change role: aeterna org members --set-role <user> --role <role>");
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "org_members_list",
                "orgId": org_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: org_members_list");
    }

    org_server_required(
        "org_members_list",
        &format!("Cannot list members for organization '{org_id}': server not connected"),
    )
}

async fn run_use(args: OrgUseArgs) -> anyhow::Result<()> {
    set_context_value("org_id", &args.org_id)?;

    output::header("Set Default Organization");
    println!();
    println!("  Setting default org: {}", args.org_id);
    println!();
    println!("  ✓ Updated .aeterna/context.toml");
    println!("  org_id = \"{}\"", args.org_id);

    Ok(())
}
