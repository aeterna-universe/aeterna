use clap::{Args, Subcommand};
use context::ContextResolver;
use serde_json::json;

use crate::output;
use crate::ux_error;

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    crate::backend::connect()
        .await
        .ok()
        .map(|(client, _)| client)
}

fn user_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("This command requires a live control-plane backend")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
        .suggest("aeterna admin health")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

#[derive(Subcommand)]
pub enum UserCommand {
    #[command(about = "Register current user or show registration status")]
    Register(UserRegisterArgs),

    #[command(about = "List users in your organization/team")]
    List(UserListArgs),

    #[command(about = "Show user details")]
    Show(UserShowArgs),

    #[command(about = "Manage user roles")]
    Roles(UserRolesArgs),

    #[command(about = "Show current user profile")]
    Whoami(UserWhoamiArgs),

    #[command(about = "Invite a user to join organization/team")]
    Invite(UserInviteArgs),
}

#[derive(Args)]
pub struct UserRegisterArgs {
    #[arg(long)]
    pub email: Option<String>,

    #[arg(long)]
    pub name: Option<String>,

    #[arg(long)]
    pub org: Option<String>,

    #[arg(long)]
    pub team: Option<String>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct UserListArgs {
    #[arg(long)]
    pub org: Option<String>,

    #[arg(long)]
    pub team: Option<String>,

    #[arg(long)]
    pub role: Option<String>,

    #[arg(long)]
    pub all: bool,

    #[arg(long)]
    pub json: bool,

    /// #44.d §6 — cross-tenant scoping (`--all-tenants` / `--tenant <slug>`).
    #[command(flatten)]
    pub scope: super::tenant_scope::TenantScopeArgs,
}

#[derive(Args)]
pub struct UserShowArgs {
    pub user_id: Option<String>,

    #[arg(short, long)]
    pub verbose: bool,

    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct UserRolesArgs {
    #[arg(long)]
    pub user: Option<String>,

    #[arg(short, long)]
    pub list: bool,

    #[arg(long)]
    pub grant: Option<String>,

    #[arg(long)]
    pub revoke: Option<String>,

    #[arg(long)]
    pub scope: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct UserWhoamiArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct UserInviteArgs {
    pub email: String,

    #[arg(short, long)]
    pub org: Option<String>,

    #[arg(short, long)]
    pub team: Option<String>,

    #[arg(short, long, default_value = "developer")]
    pub role: String,

    #[arg(short, long)]
    pub message: Option<String>,

    #[arg(short, long)]
    pub yes: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(cmd: UserCommand) -> anyhow::Result<()> {
    match cmd {
        UserCommand::Register(args) => run_register(args).await,
        UserCommand::List(args) => run_list(args).await,
        UserCommand::Show(args) => run_show(args).await,
        UserCommand::Roles(args) => run_roles(args).await,
        UserCommand::Whoami(args) => run_whoami(args).await,
        UserCommand::Invite(args) => run_invite(args).await,
    }
}

async fn run_register(args: UserRegisterArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let email = args
        .email
        .clone()
        .unwrap_or_else(|| resolved.user_id.value.clone());

    let display_name = args.name.clone().unwrap_or_else(|| {
        email
            .split('@')
            .next()
            .unwrap_or("User")
            .replace('.', " ")
            .split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    });

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "user_register",
                "user": {
                    "email": email,
                    "name": display_name,
                    "org": args.org,
                    "team": args.team,
                },
                "context": {
                    "tenantId": resolved.tenant_id.value,
                },
                "nextSteps": [
                    "Review registration details",
                    "Run without --dry-run to register",
                    "Admin approval may be required"
                ]
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("User Registration (Dry Run)");
            println!();
            println!("  Email: {email}");
            println!("  Name:  {display_name}");
            if let Some(ref org) = args.org {
                println!("  Org:   {org}");
            }
            if let Some(ref team) = args.team {
                println!("  Team:  {team}");
            }
            println!();

            output::header("What Would Happen");
            println!("  1. Create user account for '{email}'");
            println!("  2. Set display name to '{display_name}'");
            if args.org.is_some() || args.team.is_some() {
                println!("  3. Request membership in specified org/team");
                println!("  4. Wait for admin approval (if required)");
            } else {
                println!("  3. Grant access to tenant-wide resources");
            }
            println!();

            output::header("Next Steps");
            println!("  1. Run without --dry-run to register");
            println!("  2. Complete any required verification");
            println!("  3. Use 'aeterna user whoami' to verify registration");
            println!();

            output::info("Dry run mode - user not registered.");
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let body = json!({
            "email": email,
            "name": display_name,
            "org": args.org,
            "team": args.team,
        });
        let result = client.user_register(&body).await.inspect_err(|e| {
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
            output::header("User Registered");
            println!();
            println!("  ID:     {}", result["id"].as_str().unwrap_or("?"));
            println!("  Email:  {}", result["email"].as_str().unwrap_or("?"));
            println!("  Name:   {}", result["name"].as_str().unwrap_or("?"));
            println!("  Status: {}", result["status"].as_str().unwrap_or("?"));
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
                "operation": "user_register"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: user_register");
    }

    user_server_required(
        "user_register",
        "Cannot register user: server not connected",
    )
}

async fn run_list(args: UserListArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client().await {
        let scope = args.scope.to_query_param();
        let result = client
            .user_list(
                args.org.as_deref(),
                args.team.as_deref(),
                args.role.as_deref(),
                args.all,
                scope.as_deref(),
            )
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
            // JSON mode: emit the server envelope verbatim.
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            let payload = super::tenant_scope::ListPayload::from_json(&result);
            output::header("Users");
            if let Some(banner) = &payload.banner {
                println!("  {banner}");
            }
            println!();
            if payload.items.is_empty() {
                println!("  (no users found)");
            } else {
                for user in &payload.items {
                    if args.scope.is_cross_tenant() {
                        // Prefix each row with the tenant slug — cross-tenant
                        // views can return two users with the same email
                        // living in different tenants, and the tenant is
                        // the disambiguator.
                        let tenant = user
                            .get("tenantSlug")
                            .and_then(|v| v.as_str())
                            .unwrap_or("-");
                        println!(
                            "  [{:<16}] {:<36} {:<24} {}",
                            tenant,
                            user["email"].as_str().unwrap_or("?"),
                            user["name"].as_str().unwrap_or("?"),
                            user["status"].as_str().unwrap_or("?")
                        );
                    } else {
                        println!(
                            "  {:<36} {:<24} {}",
                            user["email"].as_str().unwrap_or("?"),
                            user["name"].as_str().unwrap_or("?"),
                            user["status"].as_str().unwrap_or("?")
                        );
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
                "operation": "user_list"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: user_list");
    }

    user_server_required("user_list", "Cannot list users: server not connected")
}

async fn run_show(args: UserShowArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let user_id = args
        .user_id
        .clone()
        .unwrap_or_else(|| resolved.user_id.value.clone());

    if let Some(client) = get_live_client().await {
        let result = client.user_show(&user_id).await.inspect_err(|e| {
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
            output::header(&format!("User: {user_id}"));
            println!();
            println!("  ID:     {}", result["id"].as_str().unwrap_or("?"));
            println!("  Email:  {}", result["email"].as_str().unwrap_or("?"));
            println!("  Name:   {}", result["name"].as_str().unwrap_or("?"));
            println!("  Status: {}", result["status"].as_str().unwrap_or("?"));
            if args.verbose
                && let Some(roles) = result["roles"].as_array()
            {
                println!();
                output::subheader("Roles");
                for role in roles {
                    println!(
                        "  {}  {}",
                        role["scope"].as_str().unwrap_or("?"),
                        role["role"].as_str().unwrap_or("?")
                    );
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
                "operation": "user_show",
                "userId": user_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: user_show");
    }

    user_server_required(
        "user_show",
        &format!("Cannot show user '{user_id}': server not connected"),
    )
}

async fn run_roles(args: UserRolesArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let user_id = args
        .user
        .clone()
        .unwrap_or_else(|| resolved.user_id.value.clone());

    if let Some(ref role_to_grant) = args.grant {
        let valid_roles = ["developer", "techlead", "architect", "admin"];
        if !valid_roles.contains(&role_to_grant.to_lowercase().as_str()) {
            let err = ux_error::UxError::new(format!("Invalid role: '{role_to_grant}'"))
                .why("Role must be one of the predefined governance roles")
                .fix("Use one of: developer, techlead, architect, admin")
                .suggest(format!(
                    "aeterna user roles --user {user_id} --grant developer"
                ));
            err.display();
            return Err(anyhow::anyhow!("Invalid role"));
        }

        let scope = args.scope.clone().unwrap_or_else(|| "tenant".to_string());

        if let Some(client) = get_live_client().await {
            let result = client
                .user_role_grant(&user_id, &json!({"role": role_to_grant, "scope": scope}))
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
                output::header("Role Granted");
                println!();
                println!("  User:  {}", result["userId"].as_str().unwrap_or(&user_id));
                println!(
                    "  Role:  {}",
                    result["role"].as_str().unwrap_or(role_to_grant)
                );
                println!("  Scope: {}", result["scope"].as_str().unwrap_or(&scope));
                println!();
            }
        } else if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "user_role_grant",
                    "userId": user_id,
                    "role": role_to_grant,
                    "scope": scope
                }))?
            );
            anyhow::bail!("Aeterna server not connected for operation: user_role_grant");
        } else {
            user_server_required(
                "user_role_grant",
                &format!("Cannot grant role to user '{user_id}': server not connected"),
            )?;
        }
        return Ok(());
    }

    if let Some(ref role_to_revoke) = args.revoke {
        let scope = args.scope.clone().unwrap_or_else(|| "tenant".to_string());

        if let Some(client) = get_live_client().await {
            let result = client
                .user_role_revoke(&user_id, role_to_revoke)
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
                output::header("Role Revoked");
                println!();
                println!("  User:  {}", result["userId"].as_str().unwrap_or(&user_id));
                println!(
                    "  Role:  {}",
                    result["role"].as_str().unwrap_or(role_to_revoke)
                );
                println!();
            }
        } else if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "user_role_revoke",
                    "userId": user_id,
                    "role": role_to_revoke,
                    "scope": scope
                }))?
            );
            anyhow::bail!("Aeterna server not connected for operation: user_role_revoke");
        } else {
            user_server_required(
                "user_role_revoke",
                &format!("Cannot revoke role from user '{user_id}': server not connected"),
            )?;
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let result = client.user_roles_list(&user_id).await.inspect_err(|e| {
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
            output::header(&format!("Roles for: {user_id}"));
            println!();
            if let Some(roles) = result.as_array() {
                if roles.is_empty() {
                    println!("  (no roles found)");
                } else {
                    for role in roles {
                        println!(
                            "  {:<24} {}",
                            role["scope"].as_str().unwrap_or("?"),
                            role["role"].as_str().unwrap_or("?")
                        );
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
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
                "operation": "user_roles_list",
                "userId": user_id,
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: user_roles_list");
    }

    user_server_required(
        "user_roles_list",
        &format!("Cannot list roles for user '{user_id}': server not connected"),
    )
}

async fn run_whoami(args: UserWhoamiArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    if args.json {
        let output = json!({
            "operation": "user_whoami",
            "user": {
                "id": resolved.user_id.value,
                "tenant": resolved.tenant_id.value,
                "org": resolved.org_id.as_ref().map(|o| &o.value),
                "team": resolved.team_id.as_ref().map(|t| &t.value),
                "project": resolved.project_id.as_ref().map(|p| &p.value),
            },
            "source": {
                "userId": format!("{:?}", resolved.user_id.source),
                "tenantId": format!("{:?}", resolved.tenant_id.source),
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("Current User");
        println!();
        println!(
            "  User ID:    {}  (from {})",
            resolved.user_id.value, resolved.user_id.source
        );
        println!(
            "  Tenant:     {}  (from {})",
            resolved.tenant_id.value, resolved.tenant_id.source
        );

        if let Some(ref org) = resolved.org_id {
            println!("  Org:        {}  (from {})", org.value, org.source);
        }
        if let Some(ref team) = resolved.team_id {
            println!("  Team:       {}  (from {})", team.value, team.source);
        }
        if let Some(ref project) = resolved.project_id {
            println!("  Project:    {}  (from {})", project.value, project.source);
        }
        println!();

        output::header("Context Sources");
        println!("  git     - Detected from git remote/user.email");
        println!("  env     - Environment variables (AETERNA_*)");
        println!(
            "  config  - profile config files (.aeterna/config.toml, ~/.config/aeterna/config.toml)"
        );
        println!("  default - Built-in defaults");
        println!();

        output::info("Use 'aeterna context set' to override any value.");
    }

    Ok(())
}

async fn run_invite(args: UserInviteArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    if !args.email.contains('@') || !args.email.contains('.') {
        let err = ux_error::UxError::new(format!("Invalid email address: '{}'", args.email))
            .why("Email must be a valid email address format")
            .fix("Provide a properly formatted email address")
            .suggest("aeterna user invite alice@example.com");
        err.display();
        return Err(anyhow::anyhow!("Invalid email"));
    }

    let valid_roles = ["developer", "techlead", "architect", "admin"];
    let role_lower = args.role.to_lowercase();
    if !valid_roles.contains(&role_lower.as_str()) {
        let err = ux_error::UxError::new(format!("Invalid role: '{}'", args.role))
            .why("Role must be one of the predefined governance roles")
            .fix("Use one of: developer, techlead, architect, admin")
            .suggest(format!(
                "aeterna user invite {} --role developer",
                args.email
            ));
        err.display();
        return Err(anyhow::anyhow!("Invalid role"));
    }

    let target_org = args
        .org
        .clone()
        .or_else(|| resolved.org_id.as_ref().map(|o| o.value.clone()));
    let target_team = args
        .team
        .clone()
        .or_else(|| resolved.team_id.as_ref().map(|t| t.value.clone()));

    if target_org.is_none() {
        let err = ux_error::UxError::new("No organization specified for invitation")
            .why("Users must be invited to a specific organization or team")
            .fix("Specify --org or set your context to an organization")
            .suggest("aeterna user invite alice@example.com --org platform-eng");
        err.display();
        return Err(anyhow::anyhow!("No organization specified"));
    }

    let org_name = target_org.as_ref().unwrap();

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "user_invite",
                "invitation": {
                    "email": args.email,
                    "org": org_name,
                    "team": target_team,
                    "role": role_lower,
                    "message": args.message,
                },
                "invitedBy": resolved.user_id.value,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                },
                "nextSteps": [
                    "Review invitation details",
                    "Run without --dry-run to send invitation",
                    "Invitee will receive email with join link"
                ]
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("User Invitation (Dry Run)");
            println!();
            println!("  Inviting: {}", args.email);
            println!("  To Org:   {org_name}");
            if let Some(ref team) = target_team {
                println!("  To Team:  {team}");
            }
            println!("  Role:     {role_lower}");
            if let Some(ref msg) = args.message {
                println!("  Message:  \"{msg}\"");
            }
            println!();

            output::header("What Would Happen");
            println!("  1. Create pending invitation record");
            println!("  2. Generate unique invitation link");
            println!("  3. Send invitation email to '{}'", args.email);
            println!("  4. Record audit event");
            println!();

            output::header("Invitation Email Preview");
            println!("  Subject: You've been invited to join {org_name} on Aeterna");
            println!();
            println!("  Body:");
            println!("  ----");
            println!(
                "  {} has invited you to join {}.",
                resolved.user_id.value, org_name
            );
            if let Some(ref team) = target_team {
                println!("  You will be added to the '{team}' team.");
            }
            println!("  Your initial role will be: {role_lower}");
            if let Some(ref msg) = args.message {
                println!();
                println!("  Personal message:");
                println!("  \"{msg}\"");
            }
            println!();
            println!("  Click here to accept: https://aeterna.example.com/invite/abc123...");
            println!("  ----");
            println!();

            output::header("After Acceptance");
            println!("  - User account created for '{}'", args.email);
            println!("  - Membership granted to '{org_name}'");
            if let Some(ref team) = target_team {
                println!("  - Added to team '{team}'");
            }
            println!("  - Role '{role_lower}' assigned at appropriate scope");
            println!();

            output::info("Dry run mode - invitation not sent.");
        }
        return Ok(());
    }

    if !args.yes {
        println!();
        output::header("Confirm Invitation");
        println!();
        println!("  Email:    {}", args.email);
        println!("  Org:      {org_name}");
        if let Some(ref team) = target_team {
            println!("  Team:     {team}");
        }
        println!("  Role:     {role_lower}");
        println!();

        output::warn("This will send an invitation email to the user.");
        println!();
        println!("  Use --yes to skip this prompt.");
        println!("  Use --dry-run to preview without sending.");
        println!();

        output::info("Confirmation required. Use --yes to proceed.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let body = json!({
            "email": args.email,
            "org": target_org,
            "team": target_team,
            "role": role_lower,
            "message": args.message,
        });
        let result = client.user_invite(&body).await.inspect_err(|e| {
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
            output::header("Invitation Sent");
            println!();
            println!(
                "  Email:  {}",
                result["invitation"]["email"]
                    .as_str()
                    .unwrap_or(&args.email)
            );
            println!(
                "  Scope:  {}",
                result["invitation"]["scope"].as_str().unwrap_or("?")
            );
            println!(
                "  Role:   {}",
                result["invitation"]["role"].as_str().unwrap_or(&role_lower)
            );
            println!(
                "  Status: {}",
                result["invitation"]["status"].as_str().unwrap_or("pending")
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
                "operation": "user_invite",
                "email": args.email
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: user_invite");
    }

    user_server_required(
        "user_invite",
        &format!("Cannot invite user '{}': server not connected", args.email),
    )
}

fn generate_invitation_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{now:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_register_args_defaults() {
        let args = UserRegisterArgs {
            email: None,
            name: None,
            org: None,
            team: None,
            json: false,
            dry_run: false,
        };
        assert!(args.email.is_none());
        assert!(args.name.is_none());
        assert!(args.org.is_none());
        assert!(args.team.is_none());
        assert!(!args.json);
        assert!(!args.dry_run);
    }

    #[test]
    fn test_user_register_args_with_all_options() {
        let args = UserRegisterArgs {
            email: Some("alice@example.com".to_string()),
            name: Some("Alice Smith".to_string()),
            org: Some("platform-eng".to_string()),
            team: Some("api-team".to_string()),
            json: true,
            dry_run: true,
        };
        assert_eq!(args.email, Some("alice@example.com".to_string()));
        assert_eq!(args.name, Some("Alice Smith".to_string()));
        assert_eq!(args.org, Some("platform-eng".to_string()));
        assert_eq!(args.team, Some("api-team".to_string()));
        assert!(args.json);
        assert!(args.dry_run);
    }

    #[test]
    fn test_user_list_args_defaults() {
        let args = UserListArgs {
            org: None,
            team: None,
            role: None,
            all: false,
            json: false,
            scope: Default::default(),
        };
        assert!(args.org.is_none());
        assert!(args.team.is_none());
        assert!(args.role.is_none());
        assert!(!args.all);
        assert!(!args.json);
    }

    #[test]
    fn test_user_list_args_with_filters() {
        let args = UserListArgs {
            org: Some("engineering".to_string()),
            team: Some("backend".to_string()),
            role: Some("developer".to_string()),
            all: true,
            json: true,
            scope: Default::default(),
        };
        assert_eq!(args.org, Some("engineering".to_string()));
        assert_eq!(args.team, Some("backend".to_string()));
        assert_eq!(args.role, Some("developer".to_string()));
        assert!(args.all);
    }

    #[test]
    fn test_user_show_args_defaults() {
        let args = UserShowArgs {
            user_id: None,
            verbose: false,
            json: false,
        };
        assert!(args.user_id.is_none());
        assert!(!args.verbose);
        assert!(!args.json);
    }

    #[test]
    fn test_user_show_args_with_user_id() {
        let args = UserShowArgs {
            user_id: Some("alice@example.com".to_string()),
            verbose: true,
            json: true,
        };
        assert_eq!(args.user_id, Some("alice@example.com".to_string()));
        assert!(args.verbose);
    }

    #[test]
    fn test_user_roles_args_list_mode() {
        let args = UserRolesArgs {
            user: None,
            list: true,
            grant: None,
            revoke: None,
            scope: None,
            json: false,
        };
        assert!(args.list);
        assert!(args.grant.is_none());
        assert!(args.revoke.is_none());
    }

    #[test]
    fn test_user_roles_args_grant_mode() {
        let args = UserRolesArgs {
            user: Some("bob@example.com".to_string()),
            list: false,
            grant: Some("techlead".to_string()),
            revoke: None,
            scope: Some("api-team".to_string()),
            json: true,
        };
        assert_eq!(args.user, Some("bob@example.com".to_string()));
        assert_eq!(args.grant, Some("techlead".to_string()));
        assert_eq!(args.scope, Some("api-team".to_string()));
    }

    #[test]
    fn test_user_roles_args_revoke_mode() {
        let args = UserRolesArgs {
            user: Some("bob@example.com".to_string()),
            list: false,
            grant: None,
            revoke: Some("admin".to_string()),
            scope: Some("tenant".to_string()),
            json: false,
        };
        assert_eq!(args.revoke, Some("admin".to_string()));
    }

    #[test]
    fn test_user_whoami_args() {
        let args = UserWhoamiArgs { json: false };
        assert!(!args.json);

        let args_json = UserWhoamiArgs { json: true };
        assert!(args_json.json);
    }

    #[test]
    fn test_user_invite_args_minimal() {
        let args = UserInviteArgs {
            email: "newuser@example.com".to_string(),
            org: None,
            team: None,
            role: "developer".to_string(),
            message: None,
            yes: false,
            json: false,
            dry_run: false,
        };
        assert_eq!(args.email, "newuser@example.com");
        assert_eq!(args.role, "developer");
        assert!(args.org.is_none());
        assert!(!args.yes);
    }

    #[test]
    fn test_user_invite_args_full() {
        let args = UserInviteArgs {
            email: "carol@example.com".to_string(),
            org: Some("product-eng".to_string()),
            team: Some("mobile".to_string()),
            role: "techlead".to_string(),
            message: Some("Welcome to the team!".to_string()),
            yes: true,
            json: true,
            dry_run: true,
        };
        assert_eq!(args.email, "carol@example.com");
        assert_eq!(args.org, Some("product-eng".to_string()));
        assert_eq!(args.team, Some("mobile".to_string()));
        assert_eq!(args.role, "techlead");
        assert_eq!(args.message, Some("Welcome to the team!".to_string()));
        assert!(args.yes);
        assert!(args.dry_run);
    }

    #[test]
    fn test_generate_invitation_id() {
        let id1 = generate_invitation_id();
        let id2 = generate_invitation_id();

        assert!(!id1.is_empty());
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(id2.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_invitation_id_format() {
        let id = generate_invitation_id();
        assert!(id.len() >= 8);
    }

    #[test]
    fn test_email_validation_valid() {
        let valid_emails = [
            "user@example.com",
            "alice.smith@example.org",
            "bob+tag@domain.co.uk",
        ];
        for email in valid_emails {
            assert!(
                email.contains('@') && email.contains('.'),
                "Email should be valid: {email}"
            );
        }
    }

    #[test]
    fn test_email_validation_invalid() {
        let invalid_emails = ["userexample.com", "user@example", "@example.com", "user@"];
        for email in invalid_emails {
            let is_valid = email.contains('@') && email.contains('.');
            if email == "user@example" || email == "user@" {
                assert!(
                    !is_valid || !email.split('@').next_back().unwrap_or("").contains('.'),
                    "Email should be invalid: {email}"
                );
            }
        }
    }

    #[test]
    fn test_role_validation_valid_roles() {
        let valid_roles = ["developer", "techlead", "architect", "admin"];
        for role in valid_roles {
            assert!(valid_roles.contains(&role.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_role_validation_invalid_roles() {
        let valid_roles = ["developer", "techlead", "architect", "admin"];
        let invalid_roles = ["superuser", "root", "manager", "viewer"];
        for role in invalid_roles {
            assert!(!valid_roles.contains(&role.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_role_validation_case_insensitive() {
        let valid_roles = ["developer", "techlead", "architect", "admin"];
        let mixed_case = ["Developer", "TECHLEAD", "Architect", "ADMIN"];
        for role in mixed_case {
            assert!(valid_roles.contains(&role.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_user_register_args_email_only() {
        let args = UserRegisterArgs {
            email: Some("test@example.com".to_string()),
            name: None,
            org: None,
            team: None,
            json: false,
            dry_run: false,
        };
        assert!(args.email.is_some());
        assert!(args.name.is_none());
    }

    #[test]
    fn test_user_list_args_all_flag() {
        let args = UserListArgs {
            org: None,
            team: None,
            role: None,
            all: true,
            json: false,
            scope: Default::default(),
        };
        assert!(args.all);
    }

    #[test]
    fn test_user_roles_args_no_action() {
        let args = UserRolesArgs {
            user: Some("user@example.com".to_string()),
            list: false,
            grant: None,
            revoke: None,
            scope: None,
            json: false,
        };
        assert!(!args.list);
        assert!(args.grant.is_none());
        assert!(args.revoke.is_none());
    }

    #[test]
    fn test_user_roles_args_scope_levels() {
        let scopes = ["tenant", "org", "team", "project"];
        for scope in scopes {
            let args = UserRolesArgs {
                user: Some("user@example.com".to_string()),
                list: false,
                grant: Some("developer".to_string()),
                revoke: None,
                scope: Some(scope.to_string()),
                json: false,
            };
            assert_eq!(args.scope, Some(scope.to_string()));
        }
    }

    #[test]
    fn test_user_invite_args_all_roles() {
        let roles = ["developer", "techlead", "architect", "admin"];
        for role in roles {
            let args = UserInviteArgs {
                email: "test@example.com".to_string(),
                org: Some("test-org".to_string()),
                team: None,
                role: role.to_string(),
                message: None,
                yes: true,
                json: false,
                dry_run: false,
            };
            assert_eq!(args.role, role);
        }
    }

    #[test]
    fn test_user_invite_args_with_message() {
        let args = UserInviteArgs {
            email: "new@example.com".to_string(),
            org: Some("engineering".to_string()),
            team: Some("api".to_string()),
            role: "developer".to_string(),
            message: Some("Please join our team for the Q1 project".to_string()),
            yes: false,
            json: false,
            dry_run: false,
        };
        assert!(args.message.is_some());
        assert!(args.message.unwrap().len() > 10);
    }

    #[test]
    fn test_user_show_args_verbose_mode() {
        let args = UserShowArgs {
            user_id: Some("detailed-user@example.com".to_string()),
            verbose: true,
            json: false,
        };
        assert!(args.verbose);
        assert!(!args.json);
    }

    #[test]
    fn test_user_list_args_org_filter_only() {
        let args = UserListArgs {
            org: Some("platform".to_string()),
            team: None,
            role: None,
            all: false,
            json: false,
            scope: Default::default(),
        };
        assert!(args.org.is_some());
        assert!(args.team.is_none());
        assert!(args.role.is_none());
    }

    #[test]
    fn test_user_list_args_team_filter_only() {
        let args = UserListArgs {
            org: None,
            team: Some("frontend".to_string()),
            role: None,
            all: false,
            json: false,
            scope: Default::default(),
        };
        assert!(args.team.is_some());
        assert!(args.org.is_none());
    }

    #[test]
    fn test_user_list_args_role_filter_only() {
        let args = UserListArgs {
            org: None,
            team: None,
            role: Some("admin".to_string()),
            all: false,
            json: true,
            scope: Default::default(),
        };
        assert!(args.role.is_some());
        assert!(args.json);
    }
}
