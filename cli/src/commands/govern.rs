use clap::{Args, Subcommand, ValueEnum};
use mk_core::types::SYSTEM_USER_ID;
use serde_json::json;

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum GovernCommand {
    #[command(about = "Show governance status and pending approvals")]
    Status(GovernStatusArgs),

    #[command(about = "List pending approval requests")]
    Pending(GovernPendingArgs),

    #[command(about = "Approve a governance request")]
    Approve(GovernApproveArgs),

    #[command(about = "Reject a governance request")]
    Reject(GovernRejectArgs),

    #[command(about = "Configure governance settings")]
    Configure(GovernConfigureArgs),

    #[command(about = "Manage governance roles")]
    Roles(GovernRolesArgs),

    #[command(about = "View governance audit trail")]
    Audit(GovernAuditArgs),
}

#[derive(Args)]
pub struct GovernStatusArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Include detailed metrics
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct GovernPendingArgs {
    /// Filter by type (policy, knowledge, memory, all)
    #[arg(short = 't', long, default_value = "all")]
    pub request_type: String,

    /// Filter by layer (tenant, org, team, project)
    #[arg(short, long)]
    pub layer: Option<String>,

    /// Filter by requestor
    #[arg(long)]
    pub requestor: Option<String>,

    /// Show only my pending requests
    #[arg(long)]
    pub mine: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct GovernApproveArgs {
    /// Request ID to approve
    pub request_id: String,

    /// Approval comment
    #[arg(short, long)]
    pub comment: Option<String>,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct GovernRejectArgs {
    /// Request ID to reject
    pub request_id: String,

    /// Rejection reason (required)
    #[arg(short, long)]
    pub reason: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct GovernConfigureArgs {
    /// Show current configuration
    #[arg(long)]
    pub show: bool,

    /// Use a predefined governance template (standard, strict, permissive)
    #[arg(long, value_enum)]
    pub template: Option<GovernanceTemplate>,

    /// List available templates and their settings
    #[arg(long)]
    pub list_templates: bool,

    /// Set approval requirement (single, quorum, unanimous)
    #[arg(long)]
    pub approval_mode: Option<ApprovalMode>,

    /// Set minimum approvers required (for quorum mode)
    #[arg(long)]
    pub min_approvers: Option<u32>,

    /// Set approval timeout in hours
    #[arg(long)]
    pub timeout_hours: Option<u32>,

    /// Enable/disable auto-approve for low-risk changes
    #[arg(long)]
    pub auto_approve: Option<bool>,

    /// Set escalation contact
    #[arg(long)]
    pub escalation_contact: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct GovernRolesArgs {
    /// Role action (list, assign, revoke)
    #[arg(default_value = "list")]
    pub action: String,

    /// User or agent ID for assign/revoke
    #[arg(long)]
    pub principal: Option<String>,

    /// Role to assign/revoke
    #[arg(long)]
    pub role: Option<String>,

    /// Scope for the role (tenant, org, team, project)
    #[arg(long)]
    pub scope: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct GovernAuditArgs {
    /// Filter by action type (approve, reject, escalate, expire, all)
    #[arg(short, long, default_value = "all")]
    pub action: String,

    /// Filter by time range (1h, 24h, 7d, 30d, 90d)
    #[arg(long, default_value = "7d")]
    pub since: String,

    /// Filter by actor (user or agent ID)
    #[arg(long)]
    pub actor: Option<String>,

    /// Filter by target type (policy, knowledge, memory)
    #[arg(long)]
    pub target_type: Option<String>,

    /// Maximum number of entries to show
    #[arg(short, long, default_value = "50")]
    pub limit: usize,

    /// Export format (json, csv, none)
    #[arg(long, default_value = "none")]
    pub export: ExportFormat,

    /// Output file for export
    #[arg(short, long)]
    pub output: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// #44.d §6 — cross-tenant scoping (`--all-tenants` / `--tenant <slug>`).
    /// Since Bundle D both modes are fully supported on `/govern/audit`
    /// (rows are filtered by `acting_as_tenant_id` server-side).
    #[command(flatten)]
    pub scope: super::tenant_scope::TenantScopeArgs,
}

#[derive(Clone, ValueEnum)]
pub enum ApprovalMode {
    Single,
    Quorum,
    Unanimous,
}

#[derive(Clone, ValueEnum)]
pub enum ExportFormat {
    Json,
    Csv,
    None,
}

#[derive(Clone, ValueEnum)]
pub enum GovernanceTemplate {
    Standard,
    Strict,
    Permissive,
}

pub async fn run(cmd: GovernCommand) -> anyhow::Result<()> {
    match cmd {
        GovernCommand::Status(args) => run_status(args).await,
        GovernCommand::Pending(args) => run_pending(args).await,
        GovernCommand::Approve(args) => run_approve(args).await,
        GovernCommand::Reject(args) => run_reject(args).await,
        GovernCommand::Configure(args) => run_configure(args).await,
        GovernCommand::Roles(args) => run_roles(args).await,
        GovernCommand::Audit(args) => run_audit(args).await,
    }
}

// ---------------------------------------------------------------------------
// Live client helpers (mirrors tenant.rs / permissions.rs pattern)
// ---------------------------------------------------------------------------

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    crate::backend::connect()
        .await
        .ok()
        .map(|(client, _)| client)
}

fn govern_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("This governance command requires a live control-plane backend")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
        .suggest("aeterna auth login")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn run_status(args: GovernStatusArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client().await {
        let result = client.govern_status().await.inspect_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string(), "operation": "govern_status"})
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
            output::header("Governance Status");
            println!();

            output::subheader("Configuration");
            if let Some(config) = result.get("config").or(Some(&result)) {
                println!(
                    "  Approval Mode:    {}",
                    config["approval_mode"].as_str().unwrap_or("?")
                );
                println!(
                    "  Min Approvers:    {}",
                    config["min_approvers"]
                        .as_u64()
                        .map_or_else(|| "?".to_string(), |v| v.to_string())
                );
                println!(
                    "  Timeout:          {} hours",
                    config["timeout_hours"]
                        .as_u64()
                        .map_or_else(|| "?".to_string(), |v| v.to_string())
                );
                let auto = config["auto_approve_enabled"].as_bool().unwrap_or(false);
                println!(
                    "  Auto-approve:     {}",
                    if auto {
                        "enabled (low-risk)"
                    } else {
                        "disabled"
                    }
                );
            }
            println!();

            if let Some(metrics) = result.get("metrics").or(Some(&result)) {
                output::subheader("Activity (Today)");
                println!(
                    "  Pending Requests: {}",
                    metrics["pending_requests"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Approved:         {}",
                    metrics["approved_today"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Rejected:         {}",
                    metrics["rejected_today"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Escalated:        {}",
                    metrics["escalated"].as_u64().unwrap_or(0)
                );
                println!();

                let your_pending = metrics["your_pending_approvals"].as_u64().unwrap_or(0);
                if your_pending > 0 {
                    println!("  ⚡ You have {your_pending} request(s) awaiting your approval");
                    println!();
                    output::hint(
                        "Run 'aeterna govern pending --mine' to see your pending approvals",
                    );
                }
            }

            if args.verbose
                && let Some(recent) = result.get("recent_activity").and_then(|v| v.as_array())
            {
                output::subheader("Recent Activity");
                for item in recent {
                    println!("  • {}", item.as_str().unwrap_or("?"));
                }
            }
        }
        return Ok(());
    }

    if args.json {
        let out = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_status"
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_status");
    }
    govern_server_required(
        "govern_status",
        "Cannot show governance status: server not connected",
    )
}

async fn run_pending(args: GovernPendingArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client().await {
        let type_filter = if args.request_type == "all" {
            None
        } else {
            Some(args.request_type.as_str())
        };
        let result = client
            .govern_pending(
                type_filter,
                args.layer.as_deref(),
                args.requestor.as_deref(),
                args.mine,
            )
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "govern_pending"})
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
            let requests = result["requests"].as_array();
            let count = requests.map_or(0, std::vec::Vec::len);

            output::header(&format!("Pending Requests ({count})"));
            println!();

            if count == 0 {
                println!("  ✓ No pending requests matching your filters");
                println!();
            } else if let Some(reqs) = requests {
                for req in reqs {
                    let status = req["status"].as_str().unwrap_or("?");
                    let status_icon = match status {
                        "ready" => "✓",
                        "pending" => "○",
                        _ => "?",
                    };
                    let id = req["id"].as_str().unwrap_or("?");
                    let title = req["title"].as_str().unwrap_or("?");
                    let rtype = req["type"]
                        .as_str()
                        .or_else(|| req["request_type"].as_str())
                        .unwrap_or("?");
                    let requestor = req["requestor"].as_str().unwrap_or("?");
                    let layer = req["layer"].as_str().unwrap_or("?");
                    let approvals = req["approvals"].as_u64().unwrap_or(0);
                    let required = req["required_approvals"].as_u64().unwrap_or(0);
                    let created = req["created_at"].as_str().unwrap_or("?");

                    println!("  {status_icon} [{id}] {title} ({rtype})");
                    println!(
                        "      Requestor: {requestor}  |  Layer: {layer}  |  Approvals: {approvals}/{required}"
                    );
                    println!("      Created: {created}");
                    println!();
                }

                output::hint(
                    "Use 'aeterna govern approve <id>' or 'aeterna govern reject <id>' to act",
                );
            }
        }
        return Ok(());
    }

    if args.json {
        let out = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_pending"
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_pending");
    }
    govern_server_required(
        "govern_pending",
        "Cannot list pending requests: server not connected",
    )
}

async fn run_approve(args: GovernApproveArgs) -> anyhow::Result<()> {
    if !args.yes {
        eprintln!(
            "This will approve request '{}'. Use --yes to confirm.",
            args.request_id
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let mut body = json!({});
        if let Some(ref comment) = args.comment {
            body["comment"] = json!(comment);
        }

        let result = client
            .govern_approve(&args.request_id, &body)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "govern_approve", "request_id": args.request_id})
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
            output::header("Approve Request");
            println!();

            println!("  Request ID: {}", args.request_id);
            if let Some(title) = result["title"].as_str() {
                println!("  Title:      {title}");
            }
            if let Some(rtype) = result["type"]
                .as_str()
                .or_else(|| result["request_type"].as_str())
            {
                println!("  Type:       {rtype}");
            }
            println!();

            println!("  ✓ Request approved");
            if let Some(comment) = &args.comment {
                println!("    Comment: {comment}");
            }
            println!();

            let fully_approved = result["fully_approved"].as_bool().unwrap_or(false);
            if fully_approved {
                let approvals = result["new_approval_count"]
                    .as_u64()
                    .or_else(|| result["approvals"].as_u64())
                    .unwrap_or(0);
                let required = result["required_approvals"].as_u64().unwrap_or(0);
                println!("  ⚡ Request is now fully approved ({approvals}/{required})");
                println!("    The change will be applied automatically.");
            } else {
                let approvals = result["new_approval_count"]
                    .as_u64()
                    .or_else(|| result["approvals"].as_u64())
                    .unwrap_or(0);
                let required = result["required_approvals"].as_u64().unwrap_or(0);
                println!("  ○ Approval recorded ({approvals}/{required})");
                if required > approvals {
                    println!("    Waiting for {} more approval(s).", required - approvals);
                }
            }
            println!();
        }
        return Ok(());
    }

    if args.json {
        let out = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_approve",
            "request_id": args.request_id
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_approve");
    }
    govern_server_required(
        "govern_approve",
        "Cannot approve request: server not connected",
    )
}

async fn run_reject(args: GovernRejectArgs) -> anyhow::Result<()> {
    if args.reason.is_empty() {
        ux_error::UxError::new("Rejection reason is required")
            .why("Requestors need feedback to understand why their request was rejected")
            .fix("Provide a reason using the --reason flag")
            .suggest(format!(
                "aeterna govern reject {} --reason \"Need security review first\"",
                args.request_id
            ))
            .display();
        crate::exit_code::ExitCode::Usage.exit();
    }

    if !args.yes {
        eprintln!(
            "This will reject request '{}'. Use --yes to confirm.",
            args.request_id
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let body = json!({ "reason": args.reason });

        let result = client
            .govern_reject(&args.request_id, &body)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "govern_reject", "request_id": args.request_id})
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
            output::header("Reject Request");
            println!();

            println!("  Request ID: {}", args.request_id);
            if let Some(title) = result["title"].as_str() {
                println!("  Title:      {title}");
            }
            if let Some(rtype) = result["type"]
                .as_str()
                .or_else(|| result["request_type"].as_str())
            {
                println!("  Type:       {rtype}");
            }
            if let Some(requestor) = result["requestor"].as_str() {
                println!("  Requestor:  {requestor}");
            }
            println!();

            println!("  ✗ Request rejected");
            println!("    Reason: {}", args.reason);
            println!();
            if let Some(requestor) = result["requestor"].as_str() {
                println!("  ℹ Requestor '{requestor}' has been notified");
            } else {
                println!("  ℹ Requestor has been notified");
            }
            println!();
        }
        return Ok(());
    }

    if args.json {
        let out = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_reject",
            "request_id": args.request_id
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_reject");
    }
    govern_server_required(
        "govern_reject",
        "Cannot reject request: server not connected",
    )
}

async fn run_configure(args: GovernConfigureArgs) -> anyhow::Result<()> {
    if args.list_templates {
        let templates = [
            (
                "standard",
                "Balanced governance with quorum-based approvals (2 approvers, 72h timeout)",
                "quorum",
                2u32,
                72u32,
                false,
            ),
            (
                "strict",
                "Maximum control with unanimous approvals (3+ approvers, 24h timeout, no \
                 auto-approve)",
                "unanimous",
                3,
                24,
                false,
            ),
            (
                "permissive",
                "Minimal friction with single approvals (1 approver, auto-approve low-risk)",
                "single",
                1,
                168,
                true,
            ),
        ];

        if args.json {
            let output: Vec<_> = templates
                .iter()
                .map(|(name, desc, mode, approvers, timeout, auto)| {
                    serde_json::json!({
                        "name": name,
                        "description": desc,
                        "settings": {
                            "approval_mode": mode,
                            "min_approvers": approvers,
                            "timeout_hours": timeout,
                            "auto_approve_low_risk": auto,
                        }
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Available Governance Templates");
            println!();

            for (name, desc, mode, approvers, timeout, auto) in templates {
                println!("  \x1b[1m{name}\x1b[0m - {desc}");
                println!("    Approval Mode:  {mode}");
                println!("    Min Approvers:  {approvers}");
                println!("    Timeout:        {timeout} hours");
                println!(
                    "    Auto-approve:   {}",
                    if auto { "yes (low-risk)" } else { "no" }
                );
                println!();
            }

            output::hint("Use --template <name> to apply a template");
        }
        return Ok(());
    }

    let has_changes = args.approval_mode.is_some()
        || args.min_approvers.is_some()
        || args.timeout_hours.is_some()
        || args.auto_approve.is_some()
        || args.escalation_contact.is_some()
        || args.template.is_some();

    if args.show || !has_changes {
        if let Some(client) = get_live_client().await {
            let result = client.govern_config_show().await.inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "govern_config_show"})
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
                output::header("Governance Configuration");
                println!();

                let cfg = if result.get("config").is_some() {
                    &result["config"]
                } else {
                    &result
                };
                println!(
                    "  Approval Mode:       {}",
                    cfg["approval_mode"].as_str().unwrap_or("?")
                );
                println!(
                    "  Min Approvers:       {}",
                    cfg["min_approvers"]
                        .as_u64()
                        .map_or_else(|| "?".to_string(), |v| v.to_string())
                );
                println!(
                    "  Timeout:             {} hours",
                    cfg["timeout_hours"]
                        .as_u64()
                        .map_or_else(|| "?".to_string(), |v| v.to_string())
                );
                let auto = cfg["auto_approve_enabled"].as_bool().unwrap_or(false);
                println!(
                    "  Auto-approve:        {}",
                    if auto { "enabled" } else { "disabled" }
                );
                println!(
                    "  Escalation Contact:  {}",
                    cfg["escalation_contact"].as_str().unwrap_or("(not set)")
                );
                println!();
                output::hint("Use --approval-mode, --min-approvers, etc. to change settings");
            }
            return Ok(());
        }

        if args.json {
            let out = json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "govern_config_show"
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
            anyhow::bail!("Aeterna server not connected for operation: govern_config_show");
        }
        return govern_server_required(
            "govern_config_show",
            "Cannot show governance config: server not connected",
        );
    }

    let mut body = json!({});

    if let Some(ref cli_template) = args.template {
        match cli_template {
            GovernanceTemplate::Standard => {
                body["approval_mode"] = json!("quorum");
                body["min_approvers"] = json!(2);
                body["timeout_hours"] = json!(72);
                body["auto_approve_enabled"] = json!(false);
            }
            GovernanceTemplate::Strict => {
                body["approval_mode"] = json!("unanimous");
                body["min_approvers"] = json!(3);
                body["timeout_hours"] = json!(24);
                body["auto_approve_enabled"] = json!(false);
            }
            GovernanceTemplate::Permissive => {
                body["approval_mode"] = json!("single");
                body["min_approvers"] = json!(1);
                body["timeout_hours"] = json!(168);
                body["auto_approve_enabled"] = json!(true);
            }
        }
    }

    if let Some(mode) = args.approval_mode {
        let mode_str = match mode {
            ApprovalMode::Single => "single",
            ApprovalMode::Quorum => "quorum",
            ApprovalMode::Unanimous => "unanimous",
        };
        body["approval_mode"] = json!(mode_str);
    }

    if let Some(min) = args.min_approvers {
        body["min_approvers"] = json!(min);
    }

    if let Some(timeout) = args.timeout_hours {
        body["timeout_hours"] = json!(timeout);
    }

    if let Some(auto) = args.auto_approve {
        body["auto_approve_enabled"] = json!(auto);
    }

    if let Some(contact) = args.escalation_contact {
        body["escalation_contact"] = json!(contact);
    }

    if let Some(client) = get_live_client().await {
        let result = client.govern_config_update(&body).await.inspect_err(|e| {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"success": false, "error": e.to_string(), "operation": "govern_config_update"})
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
            output::header("Update Governance Configuration");
            println!();

            if let Some(changes) = result["changes"].as_array() {
                output::subheader("Changes Applied");
                for change in changes {
                    println!("  ✓ {}", change.as_str().unwrap_or("?"));
                }
                println!();
            }

            println!("  Configuration updated successfully.");
            println!();
        }
        return Ok(());
    }

    if args.json {
        let out = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_config_update"
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_config_update");
    }
    govern_server_required(
        "govern_config_update",
        "Cannot update governance config: server not connected",
    )
}

async fn run_roles(args: GovernRolesArgs) -> anyhow::Result<()> {
    match args.action.as_str() {
        "list" => {
            if let Some(client) = get_live_client().await {
                let result = client.govern_roles_list().await.inspect_err(|e| {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": e.to_string(), "operation": "govern_roles_list"})
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
                    output::header("Role Assignments");
                    println!();
                    if let Some(roles) = result.as_array() {
                        println!("  {:<38} {:<10} {:<12} Scope", "Principal", "Type", "Role");
                        println!("  {}", "-".repeat(88));
                        for role in roles {
                            println!(
                                "  {:<38} {:<10} {:<12} {}",
                                role["principal"].as_str().unwrap_or("?"),
                                role["principalType"]
                                    .as_str()
                                    .or_else(|| role["principal_type"].as_str())
                                    .unwrap_or("?"),
                                role["role"].as_str().unwrap_or("?"),
                                role["scope"].as_str().unwrap_or("?")
                            );
                        }
                    } else {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    }
                    println!();
                }
            } else if args.json {
                let output = json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "govern_roles_list"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                anyhow::bail!("Aeterna server not connected for operation: govern_roles_list");
            } else {
                govern_server_required(
                    "govern_roles_list",
                    "Cannot list governance roles: server not connected",
                )?;
            }
        }
        "assign" => {
            let principal = args
                .principal
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--principal is required for assign action"))?;
            let role = args
                .role
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--role is required for assign action"))?;
            let scope = args.scope.as_deref().unwrap_or("project");

            if let Some(client) = get_live_client().await {
                let result = client
                    .govern_role_assign(&json!({
                        "principal": principal,
                        "role": role,
                        "scope": scope,
                    }))
                    .await
                    .inspect_err(|e| {
                        if args.json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(
                                    &json!({"success": false, "error": e.to_string(), "operation": "govern_role_assign"})
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
                    output::header("Assign Role");
                    println!();
                    println!(
                        "  ✓ Assigned role '{}' to '{}' at scope '{}'",
                        result["role"].as_str().unwrap_or(role),
                        result["principal"].as_str().unwrap_or(principal),
                        result["scope"].as_str().unwrap_or(scope)
                    );
                    println!();
                }
            } else if args.json {
                let output = json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "govern_role_assign"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                anyhow::bail!("Aeterna server not connected for operation: govern_role_assign");
            } else {
                govern_server_required(
                    "govern_role_assign",
                    "Cannot assign governance role: server not connected",
                )?;
            }
        }
        "revoke" => {
            let principal = args
                .principal
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--principal is required for revoke action"))?;
            let role = args
                .role
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--role is required for revoke action"))?;

            if let Some(client) = get_live_client().await {
                let result = client.govern_role_revoke(principal, role).await.inspect_err(|e| {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &json!({"success": false, "error": e.to_string(), "operation": "govern_role_revoke"})
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
                    output::header("Revoke Role");
                    println!();
                    println!(
                        "  ✓ Revoked role '{}' from '{}'",
                        result["role"].as_str().unwrap_or(role),
                        result["principal"].as_str().unwrap_or(principal)
                    );
                    println!();
                }
            } else if args.json {
                let output = json!({
                    "success": false,
                    "error": "server_not_connected",
                    "operation": "govern_role_revoke"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                anyhow::bail!("Aeterna server not connected for operation: govern_role_revoke");
            } else {
                govern_server_required(
                    "govern_role_revoke",
                    "Cannot revoke governance role: server not connected",
                )?;
            }
        }
        _ => {
            ux_error::UxError::new(format!("Unknown roles action: {}", args.action))
                .fix("Use one of: list, assign, revoke")
                .suggest("aeterna govern roles list")
                .display();
            crate::exit_code::ExitCode::Usage.exit();
        }
    }

    Ok(())
}

async fn run_audit(args: GovernAuditArgs) -> anyhow::Result<()> {
    if let Some(client) = get_live_client().await {
        let action = if args.action == "all" {
            None
        } else {
            Some(args.action.as_str())
        };
        let scope = args.scope.to_query_param();
        let result = client
            .govern_audit(
                action,
                Some(args.since.as_str()),
                args.actor.as_deref(),
                args.target_type.as_deref(),
                args.limit,
                scope.as_deref(),
            )
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"success": false, "error": e.to_string(), "operation": "govern_audit"})
                        )
                        .unwrap()
                    );
                } else {
                    ux_error::UxError::new(e.to_string())
                        .fix("Run: aeterna auth login")
                        .display();
                }
            })?;

        // /govern/audit can return either the legacy bare array (no
        // ?tenant=) or the #44.d cross-tenant envelope (?tenant=*). Use
        // the shared normalizer so the rest of the rendering path stays
        // envelope-agnostic.
        let payload = super::tenant_scope::ListPayload::from_json(&result);
        let filtered: Vec<serde_json::Value> = payload.items.iter().map(|v| (*v).clone()).collect();
        let cross_tenant_banner = payload.banner.clone();

        match args.export {
            ExportFormat::None => {
                if args.json {
                    let output = json!({
                        "since": args.since,
                        "total": filtered.len(),
                        "entries": filtered,
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    output::header(&format!("Governance Audit Trail (last {})", args.since));
                    if let Some(banner) = &cross_tenant_banner {
                        println!("  {banner}");
                    }
                    println!();

                    if filtered.is_empty() {
                        println!("  No audit entries matching your filters");
                    } else {
                        for entry in &filtered {
                            let action = entry["action"].as_str().unwrap_or("?");
                            let icon = match action {
                                "approve" => "✓",
                                "reject" => "✗",
                                "escalate" => "↑",
                                "expire" => "⏱",
                                _ => "•",
                            };
                            let created_at = entry["created_at"]
                                .as_str()
                                .or_else(|| entry["timestamp"].as_str())
                                .unwrap_or("?");
                            let actor = entry["actor_email"]
                                .as_str()
                                .or_else(|| entry["actor"].as_str())
                                .unwrap_or(SYSTEM_USER_ID);
                            let target_type = entry["target_type"].as_str().unwrap_or("?");
                            let target_id = entry["target_id"].as_str().unwrap_or("?");
                            let details = entry["details"].to_string();

                            println!(
                                "  {} [{}] {} by {}",
                                icon,
                                created_at,
                                action.to_uppercase(),
                                actor
                            );
                            println!("      {target_type} {target_id} - {details}");
                            println!();
                        }
                    }

                    output::hint("Use --export json or --export csv to export audit data");
                }
            }
            ExportFormat::Json => {
                let output = json!({
                    "exported_at": chrono::Utc::now().to_rfc3339(),
                    "since": args.since,
                    "entries": filtered,
                });

                if let Some(path) = args.output {
                    std::fs::write(&path, serde_json::to_string_pretty(&output)?)?;
                    println!(
                        "Exported {} entries to {}",
                        output["entries"].as_array().map_or(0, std::vec::Vec::len),
                        path
                    );
                } else {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
            }
            ExportFormat::Csv => {
                let mut csv =
                    String::from("id,timestamp,action,actor,target_type,target_id,details\n");
                for entry in &filtered {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{},\"{}\"\n",
                        entry["id"].as_str().unwrap_or(""),
                        entry["created_at"]
                            .as_str()
                            .or_else(|| entry["timestamp"].as_str())
                            .unwrap_or(""),
                        entry["action"].as_str().unwrap_or(""),
                        entry["actor_email"]
                            .as_str()
                            .or_else(|| entry["actor"].as_str())
                            .unwrap_or(""),
                        entry["target_type"].as_str().unwrap_or(""),
                        entry["target_id"].as_str().unwrap_or(""),
                        entry["details"].to_string().replace('"', "\"\"")
                    ));
                }

                if let Some(path) = args.output {
                    std::fs::write(&path, &csv)?;
                    println!("Exported {} entries to {}", filtered.len(), path);
                } else {
                    print!("{csv}");
                }
            }
        }

        return Ok(());
    }

    if args.json {
        let output = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "govern_audit"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        anyhow::bail!("Aeterna server not connected for operation: govern_audit");
    }

    govern_server_required(
        "govern_audit",
        "Cannot list governance audit entries: server not connected",
    )
}

// Helper types

struct GovernanceStatus {
    approval_mode: String,
    min_approvers: u32,
    timeout_hours: u32,
    auto_approve_enabled: bool,
    pending_requests: u32,
    approved_today: u32,
    rejected_today: u32,
    escalated: u32,
    your_pending_approvals: u32,
}

struct PendingRequest {
    id: String,
    request_type: String,
    title: String,
    requestor: String,
    layer: String,
    created_at: String,
    approvals: u32,
    required_approvals: u32,
    status: String,
}

struct GovernanceConfig {
    approval_mode: String,
    min_approvers: u32,
    timeout_hours: u32,
    auto_approve_enabled: bool,
    escalation_contact: Option<String>,
}

struct RoleAssignment {
    principal: String,
    principal_type: String,
    role: String,
    scope: String,
}

struct AuditEntry {
    id: String,
    timestamp: String,
    action: String,
    actor: String,
    target_type: String,
    target_id: String,
    details: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approval_mode_values() {
        let _single = ApprovalMode::Single;
        let _quorum = ApprovalMode::Quorum;
        let _unanimous = ApprovalMode::Unanimous;
    }

    #[test]
    fn test_export_format_values() {
        let _json = ExportFormat::Json;
        let _csv = ExportFormat::Csv;
        let _none = ExportFormat::None;
    }

    #[test]
    fn test_governance_template_values() {
        let _standard = GovernanceTemplate::Standard;
        let _strict = GovernanceTemplate::Strict;
        let _permissive = GovernanceTemplate::Permissive;
    }

    #[test]
    fn test_pending_request_creation() {
        let req = PendingRequest {
            id: "req_123".to_string(),
            request_type: "policy".to_string(),
            title: "Test policy".to_string(),
            requestor: "alice".to_string(),
            layer: "team".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 1,
            required_approvals: 2,
            status: "pending".to_string(),
        };
        assert_eq!(req.id, "req_123");
        assert!(req.approvals < req.required_approvals);
    }

    #[test]
    fn test_pending_request_fully_approved() {
        let req = PendingRequest {
            id: "req_456".to_string(),
            request_type: "knowledge".to_string(),
            title: "Promote ADR".to_string(),
            requestor: "bob".to_string(),
            layer: "org".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 2,
            required_approvals: 2,
            status: "ready".to_string(),
        };
        assert_eq!(req.approvals, req.required_approvals);
        assert_eq!(req.status, "ready");
    }

    #[test]
    fn test_pending_request_types() {
        let policy_req = PendingRequest {
            id: "req_1".to_string(),
            request_type: "policy".to_string(),
            title: "Policy request".to_string(),
            requestor: "alice".to_string(),
            layer: "tenant".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 0,
            required_approvals: 1,
            status: "pending".to_string(),
        };

        let knowledge_req = PendingRequest {
            id: "req_2".to_string(),
            request_type: "knowledge".to_string(),
            title: "Knowledge request".to_string(),
            requestor: "bob".to_string(),
            layer: "org".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 0,
            required_approvals: 2,
            status: "pending".to_string(),
        };

        let memory_req = PendingRequest {
            id: "req_3".to_string(),
            request_type: "memory".to_string(),
            title: "Memory request".to_string(),
            requestor: "agent_1".to_string(),
            layer: "team".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 0,
            required_approvals: 1,
            status: "pending".to_string(),
        };

        assert_eq!(policy_req.request_type, "policy");
        assert_eq!(knowledge_req.request_type, "knowledge");
        assert_eq!(memory_req.request_type, "memory");
    }

    #[test]
    fn test_role_assignment_creation() {
        let role = RoleAssignment {
            principal: "alice".to_string(),
            principal_type: "user".to_string(),
            role: "admin".to_string(),
            scope: "tenant:acme".to_string(),
        };
        assert_eq!(role.role, "admin");
    }

    #[test]
    fn test_role_assignment_agent_type() {
        let role = RoleAssignment {
            principal: "agent_codex".to_string(),
            principal_type: "agent".to_string(),
            role: "developer".to_string(),
            scope: "project:payments".to_string(),
        };
        assert_eq!(role.principal_type, "agent");
        assert_eq!(role.role, "developer");
    }

    #[test]
    fn test_role_assignment_all_roles() {
        let roles = ["admin", "architect", "techlead", "developer"];
        for role_name in roles {
            let role = RoleAssignment {
                principal: "user".to_string(),
                principal_type: "user".to_string(),
                role: role_name.to_string(),
                scope: "tenant:test".to_string(),
            };
            assert_eq!(role.role, role_name);
        }
    }

    #[test]
    fn test_role_assignment_all_scopes() {
        let scopes = [
            "tenant:acme",
            "org:platform",
            "team:api",
            "project:payments",
        ];
        for scope in scopes {
            let role = RoleAssignment {
                principal: "alice".to_string(),
                principal_type: "user".to_string(),
                role: "developer".to_string(),
                scope: scope.to_string(),
            };
            assert_eq!(role.scope, scope);
        }
    }

    #[test]
    fn test_audit_entry_creation() {
        let entry = AuditEntry {
            id: "aud_001".to_string(),
            timestamp: "2024-01-15T10:00:00Z".to_string(),
            action: "approve".to_string(),
            actor: "alice".to_string(),
            target_type: "policy".to_string(),
            target_id: "req_123".to_string(),
            details: "Approved".to_string(),
        };
        assert_eq!(entry.action, "approve");
    }

    #[test]
    fn test_audit_entry_all_actions() {
        let actions = ["approve", "reject", "escalate", "expire"];
        for action in actions {
            let entry = AuditEntry {
                id: format!("aud_{action}"),
                timestamp: "2024-01-15T10:00:00Z".to_string(),
                action: action.to_string(),
                actor: "system".to_string(),
                target_type: "policy".to_string(),
                target_id: "req_123".to_string(),
                details: format!("Action: {action}"),
            };
            assert_eq!(entry.action, action);
        }
    }

    #[test]
    fn test_audit_entry_target_types() {
        let target_types = ["policy", "knowledge", "memory"];
        for target_type in target_types {
            let entry = AuditEntry {
                id: "aud_001".to_string(),
                timestamp: "2024-01-15T10:00:00Z".to_string(),
                action: "approve".to_string(),
                actor: "alice".to_string(),
                target_type: target_type.to_string(),
                target_id: "req_123".to_string(),
                details: "Approved".to_string(),
            };
            assert_eq!(entry.target_type, target_type);
        }
    }

    #[test]
    fn test_governance_status_creation() {
        let status = GovernanceStatus {
            approval_mode: "quorum".to_string(),
            min_approvers: 2,
            timeout_hours: 72,
            auto_approve_enabled: true,
            pending_requests: 3,
            approved_today: 7,
            rejected_today: 1,
            escalated: 0,
            your_pending_approvals: 2,
        };
        assert_eq!(status.approval_mode, "quorum");
        assert_eq!(status.min_approvers, 2);
        assert!(status.auto_approve_enabled);
    }

    #[test]
    fn test_governance_status_strict_mode() {
        let status = GovernanceStatus {
            approval_mode: "unanimous".to_string(),
            min_approvers: 3,
            timeout_hours: 24,
            auto_approve_enabled: false,
            pending_requests: 5,
            approved_today: 2,
            rejected_today: 3,
            escalated: 1,
            your_pending_approvals: 4,
        };
        assert_eq!(status.approval_mode, "unanimous");
        assert!(!status.auto_approve_enabled);
        assert!(status.escalated > 0);
    }

    #[test]
    fn test_governance_config_creation() {
        let config = GovernanceConfig {
            approval_mode: "quorum".to_string(),
            min_approvers: 2,
            timeout_hours: 72,
            auto_approve_enabled: true,
            escalation_contact: Some("security-team@acme.com".to_string()),
        };
        assert_eq!(config.approval_mode, "quorum");
        assert!(config.escalation_contact.is_some());
    }

    #[test]
    fn test_governance_config_no_escalation() {
        let config = GovernanceConfig {
            approval_mode: "single".to_string(),
            min_approvers: 1,
            timeout_hours: 168,
            auto_approve_enabled: true,
            escalation_contact: None,
        };
        assert!(config.escalation_contact.is_none());
    }

    #[test]
    fn test_govern_status_args_defaults() {
        let args = GovernStatusArgs {
            json: false,
            verbose: false,
        };
        assert!(!args.json);
        assert!(!args.verbose);
    }

    #[test]
    fn test_govern_pending_args_defaults() {
        let args = GovernPendingArgs {
            request_type: "all".to_string(),
            layer: None,
            requestor: None,
            mine: false,
            json: false,
        };
        assert_eq!(args.request_type, "all");
        assert!(args.layer.is_none());
    }

    #[test]
    fn test_govern_pending_args_with_filters() {
        let args = GovernPendingArgs {
            request_type: "policy".to_string(),
            layer: Some("org".to_string()),
            requestor: Some("alice".to_string()),
            mine: true,
            json: true,
        };
        assert_eq!(args.request_type, "policy");
        assert_eq!(args.layer.as_deref(), Some("org"));
        assert!(args.mine);
    }

    #[test]
    fn test_govern_approve_args() {
        let args = GovernApproveArgs {
            request_id: "req_123".to_string(),
            comment: Some("LGTM".to_string()),
            yes: true,
            json: false,
        };
        assert_eq!(args.request_id, "req_123");
        assert!(args.comment.is_some());
        assert!(args.yes);
    }

    #[test]
    fn test_govern_reject_args() {
        let args = GovernRejectArgs {
            request_id: "req_456".to_string(),
            reason: "Security concerns".to_string(),
            yes: false,
            json: true,
        };
        assert_eq!(args.request_id, "req_456");
        assert!(!args.reason.is_empty());
    }

    #[test]
    fn test_govern_configure_args_show() {
        let args = GovernConfigureArgs {
            show: true,
            template: None,
            list_templates: false,
            approval_mode: None,
            min_approvers: None,
            timeout_hours: None,
            auto_approve: None,
            escalation_contact: None,
            json: false,
        };
        assert!(args.show);
        assert!(args.template.is_none());
    }

    #[test]
    fn test_govern_configure_args_with_template() {
        let args = GovernConfigureArgs {
            show: false,
            template: Some(GovernanceTemplate::Strict),
            list_templates: false,
            approval_mode: None,
            min_approvers: None,
            timeout_hours: None,
            auto_approve: None,
            escalation_contact: None,
            json: false,
        };
        assert!(args.template.is_some());
    }

    #[test]
    fn test_govern_configure_args_with_overrides() {
        let args = GovernConfigureArgs {
            show: false,
            template: None,
            list_templates: false,
            approval_mode: Some(ApprovalMode::Quorum),
            min_approvers: Some(3),
            timeout_hours: Some(48),
            auto_approve: Some(false),
            escalation_contact: Some("ops@example.com".to_string()),
            json: false,
        };
        assert!(args.approval_mode.is_some());
        assert_eq!(args.min_approvers, Some(3));
    }

    #[test]
    fn test_govern_roles_args_list() {
        let args = GovernRolesArgs {
            action: "list".to_string(),
            principal: None,
            role: None,
            scope: None,
            json: false,
        };
        assert_eq!(args.action, "list");
    }

    #[test]
    fn test_govern_roles_args_assign() {
        let args = GovernRolesArgs {
            action: "assign".to_string(),
            principal: Some("alice".to_string()),
            role: Some("admin".to_string()),
            scope: Some("tenant:acme".to_string()),
            json: false,
        };
        assert_eq!(args.action, "assign");
        assert!(args.principal.is_some());
        assert!(args.role.is_some());
    }

    #[test]
    fn test_govern_roles_args_revoke() {
        let args = GovernRolesArgs {
            action: "revoke".to_string(),
            principal: Some("bob".to_string()),
            role: Some("developer".to_string()),
            scope: None,
            json: true,
        };
        assert_eq!(args.action, "revoke");
        assert!(args.json);
    }

    #[test]
    fn test_govern_audit_args_defaults() {
        let args = GovernAuditArgs {
            action: "all".to_string(),
            since: "7d".to_string(),
            actor: None,
            target_type: None,
            limit: 50,
            export: ExportFormat::None,
            output: None,
            json: false,
            scope: Default::default(),
        };
        assert_eq!(args.action, "all");
        assert_eq!(args.since, "7d");
        assert_eq!(args.limit, 50);
    }

    #[test]
    fn test_govern_audit_args_with_filters() {
        let args = GovernAuditArgs {
            action: "approve".to_string(),
            since: "24h".to_string(),
            actor: Some("alice".to_string()),
            target_type: Some("policy".to_string()),
            limit: 100,
            export: ExportFormat::Json,
            output: Some("audit.json".to_string()),
            json: false,
            scope: Default::default(),
        };
        assert_eq!(args.action, "approve");
        assert!(args.actor.is_some());
    }

    #[test]
    fn test_govern_audit_args_csv_export() {
        let args = GovernAuditArgs {
            action: "all".to_string(),
            since: "30d".to_string(),
            actor: None,
            target_type: None,
            limit: 1000,
            export: ExportFormat::Csv,
            output: Some("audit.csv".to_string()),
            json: false,
            scope: Default::default(),
        };
        matches!(args.export, ExportFormat::Csv);
        assert!(args.output.is_some());
    }

    #[test]
    fn test_filter_pending_requests_by_type() {
        let requests = [
            PendingRequest {
                id: "req_1".to_string(),
                request_type: "policy".to_string(),
                title: "Policy 1".to_string(),
                requestor: "alice".to_string(),
                layer: "org".to_string(),
                created_at: "2024-01-15T10:00:00Z".to_string(),
                approvals: 0,
                required_approvals: 2,
                status: "pending".to_string(),
            },
            PendingRequest {
                id: "req_2".to_string(),
                request_type: "knowledge".to_string(),
                title: "Knowledge 1".to_string(),
                requestor: "bob".to_string(),
                layer: "tenant".to_string(),
                created_at: "2024-01-15T08:00:00Z".to_string(),
                approvals: 0,
                required_approvals: 2,
                status: "pending".to_string(),
            },
        ];

        let filtered: Vec<_> = requests
            .iter()
            .filter(|r| r.request_type == "policy")
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "req_1");
    }

    #[test]
    fn test_filter_pending_requests_by_layer() {
        let requests = [
            PendingRequest {
                id: "req_1".to_string(),
                request_type: "policy".to_string(),
                title: "Policy 1".to_string(),
                requestor: "alice".to_string(),
                layer: "org".to_string(),
                created_at: "2024-01-15T10:00:00Z".to_string(),
                approvals: 0,
                required_approvals: 2,
                status: "pending".to_string(),
            },
            PendingRequest {
                id: "req_2".to_string(),
                request_type: "policy".to_string(),
                title: "Policy 2".to_string(),
                requestor: "bob".to_string(),
                layer: "tenant".to_string(),
                created_at: "2024-01-15T08:00:00Z".to_string(),
                approvals: 0,
                required_approvals: 2,
                status: "pending".to_string(),
            },
        ];

        let layer_filter = Some("tenant".to_string());
        let filtered: Vec<_> = requests
            .iter()
            .filter(|r| layer_filter.as_ref().is_none_or(|l| &r.layer == l))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "req_2");
    }

    #[test]
    fn test_filter_audit_entries_by_action() {
        let entries = [
            AuditEntry {
                id: "aud_1".to_string(),
                timestamp: "2024-01-15T14:30:00Z".to_string(),
                action: "approve".to_string(),
                actor: "alice".to_string(),
                target_type: "policy".to_string(),
                target_id: "req_1".to_string(),
                details: "Approved".to_string(),
            },
            AuditEntry {
                id: "aud_2".to_string(),
                timestamp: "2024-01-15T12:15:00Z".to_string(),
                action: "reject".to_string(),
                actor: "bob".to_string(),
                target_type: "knowledge".to_string(),
                target_id: "req_2".to_string(),
                details: "Rejected".to_string(),
            },
        ];

        let action_filter = "approve";
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.action == action_filter)
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "aud_1");
    }

    #[test]
    fn test_filter_audit_entries_by_actor() {
        let entries = [
            AuditEntry {
                id: "aud_1".to_string(),
                timestamp: "2024-01-15T14:30:00Z".to_string(),
                action: "approve".to_string(),
                actor: "alice".to_string(),
                target_type: "policy".to_string(),
                target_id: "req_1".to_string(),
                details: "Approved".to_string(),
            },
            AuditEntry {
                id: "aud_2".to_string(),
                timestamp: "2024-01-15T12:15:00Z".to_string(),
                action: "approve".to_string(),
                actor: "bob".to_string(),
                target_type: "knowledge".to_string(),
                target_id: "req_2".to_string(),
                details: "Approved".to_string(),
            },
        ];

        let actor_filter = Some("bob".to_string());
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| actor_filter.as_ref().is_none_or(|a| &e.actor == a))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].actor, "bob");
    }

    #[test]
    fn test_approval_count_logic() {
        let mut req = PendingRequest {
            id: "req_123".to_string(),
            request_type: "policy".to_string(),
            title: "Test policy".to_string(),
            requestor: "alice".to_string(),
            layer: "team".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            approvals: 1,
            required_approvals: 2,
            status: "pending".to_string(),
        };

        assert!(req.approvals < req.required_approvals);

        req.approvals += 1;
        assert!(req.approvals >= req.required_approvals);
    }
}
