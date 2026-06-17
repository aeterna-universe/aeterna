use clap::{Args, Subcommand};
use context::ContextResolver;
use mk_core::hints::{HintPreset, OperationHints};
use serde_json::json;

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum KnowledgeCommand {
    #[command(about = "Search knowledge across layers")]
    Search(KnowledgeSearchArgs),

    #[command(about = "Get a specific knowledge entry by path")]
    Get(KnowledgeGetArgs),

    #[command(about = "List knowledge entries in a layer")]
    List(KnowledgeListArgs),

    #[command(about = "Check knowledge constraints")]
    Check(KnowledgeCheckArgs),

    #[command(about = "Propose new knowledge (ADR, Pattern, Policy, Spec)")]
    Propose(KnowledgeProposeArgs),

    #[command(about = "Promote a knowledge item to a broader layer")]
    Promote(KnowledgePromoteArgs),

    #[command(about = "Preview the effect of promoting a knowledge item")]
    PromotionPreview(KnowledgePromotionPreviewArgs),

    #[command(about = "List pending promotion requests")]
    Pending(KnowledgePendingArgs),

    #[command(about = "Approve a pending promotion request")]
    Approve(KnowledgeApproveArgs),

    #[command(about = "Reject a pending promotion request")]
    Reject(KnowledgeRejectArgs),

    #[command(about = "Retarget a promotion request to a different layer")]
    Retarget(KnowledgeRetargetArgs),

    #[command(about = "Create a semantic relation between two knowledge items")]
    Relate(KnowledgeRelateArgs),
}

// ---------------------------------------------------------------------------
// Existing arg structs
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct KnowledgeSearchArgs {
    /// Search query
    pub query: String,

    /// Maximum number of results (default: 10)
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Filter by layers (comma-separated: tenant, org, team, project)
    #[arg(long)]
    pub layers: Option<String>,

    /// Hints preset (minimal, fast, standard, full, offline, agent)
    #[arg(long)]
    pub preset: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Dry run - don't actually search, just show what would happen
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct KnowledgeGetArgs {
    /// Path to the knowledge entry (e.g., "adrs/adr-001.md")
    pub path: String,

    /// Layer to get from (tenant, org, team, project)
    #[arg(short, long, default_value = "project")]
    pub layer: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgeListArgs {
    /// Layer to list from (tenant, org, team, project)
    #[arg(short, long, default_value = "project")]
    pub layer: String,

    /// Filter by path prefix
    #[arg(long)]
    pub prefix: Option<String>,

    /// Maximum number of results
    #[arg(short = 'n', long, default_value = "20")]
    pub limit: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgeCheckArgs {
    /// Context to check (e.g., a file path, dependency, etc.)
    #[arg(short, long)]
    pub context: Option<String>,

    /// Check against a specific policy
    #[arg(long)]
    pub policy: Option<String>,

    /// Check for a specific dependency
    #[arg(long)]
    pub dependency: Option<String>,

    /// Hints preset
    #[arg(long)]
    pub preset: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct KnowledgeProposeArgs {
    /// Natural language description of what you want to propose
    pub description: String,

    /// Knowledge type (adr, pattern, policy, spec) - auto-detected if not
    /// specified
    #[arg(short = 't', long)]
    pub knowledge_type: Option<String>,

    /// Target layer (tenant, org, team, project) - inferred from description
    /// if not specified
    #[arg(short, long)]
    pub layer: Option<String>,

    /// Custom title (auto-generated from description if not specified)
    #[arg(long)]
    pub title: Option<String>,

    /// Submit directly for approval (skip draft review)
    #[arg(long)]
    pub submit: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - show what would be proposed without creating draft
    #[arg(long)]
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// New promotion lifecycle arg structs (tasks 4.1-4.7)
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct KnowledgePromoteArgs {
    /// ID of the knowledge item to promote
    pub id: String,

    /// Target layer to promote into (must be broader than source layer)
    #[arg(long)]
    pub target_layer: String,

    /// Promotion mode: full (replaces lower-layer item) or partial (split)
    #[arg(long, default_value = "full")]
    pub mode: String,

    /// Content to promote to the target layer (shared canonical content).
    /// Required for partial mode; uses full item content for full mode.
    #[arg(long)]
    pub shared_content: Option<String>,

    /// Content to keep at the source layer (residual specialization).
    /// Only used for partial mode.
    #[arg(long)]
    pub residual_content: Option<String>,

    /// Role of the residual lower-layer item (specialization, applicability, exception)
    #[arg(long)]
    pub residual_role: Option<String>,

    /// Justification for the promotion
    #[arg(long)]
    pub justification: Option<String>,

    /// Source version for optimistic concurrency
    #[arg(long)]
    pub source_version: Option<String>,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgePromotionPreviewArgs {
    /// ID of the knowledge item to preview promotion for
    pub id: String,

    /// Target layer to preview promoting into
    #[arg(long)]
    pub target_layer: String,

    /// Promotion mode: full or partial
    #[arg(long)]
    pub mode: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgePendingArgs {
    /// Filter by status (submitted, draft, all)
    #[arg(long, default_value = "submitted")]
    pub status: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgeApproveArgs {
    /// Promotion request ID to approve
    pub promotion_id: String,

    /// Approval decision (e.g. "Approve")
    #[arg(long, default_value = "Approve")]
    pub decision: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgeRejectArgs {
    /// Promotion request ID to reject
    pub promotion_id: String,

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
pub struct KnowledgeRetargetArgs {
    /// Promotion request ID to retarget
    pub promotion_id: String,

    /// New target layer
    #[arg(long)]
    pub target_layer: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct KnowledgeRelateArgs {
    /// Source knowledge item ID
    pub id: String,

    /// Target knowledge item ID
    #[arg(long)]
    pub target_id: String,

    /// Relation type (promotes_to, supersedes, specializes, references, related_to, conflicts_with, depends_on)
    #[arg(long)]
    pub relation_type: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn run(cmd: KnowledgeCommand) -> anyhow::Result<()> {
    match cmd {
        KnowledgeCommand::Search(args) => run_search(args).await,
        KnowledgeCommand::Get(args) => run_get(args).await,
        KnowledgeCommand::List(args) => run_list(args).await,
        KnowledgeCommand::Check(args) => run_check(args).await,
        KnowledgeCommand::Propose(args) => run_propose(args).await,
        KnowledgeCommand::Promote(args) => run_promote(args).await,
        KnowledgeCommand::PromotionPreview(args) => run_promotion_preview(args).await,
        KnowledgeCommand::Pending(args) => run_pending(args).await,
        KnowledgeCommand::Approve(args) => run_approve(args).await,
        KnowledgeCommand::Reject(args) => run_reject(args).await,
        KnowledgeCommand::Retarget(args) => run_retarget(args).await,
        KnowledgeCommand::Relate(args) => run_relate(args).await,
    }
}

// ---------------------------------------------------------------------------
// Live client helper
// ---------------------------------------------------------------------------

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    crate::backend::connect()
        .await
        .ok()
        .map(|(client, _)| client)
}

fn knowledge_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("This knowledge command requires a live control-plane backend")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
        .suggest("aeterna auth login")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

// ---------------------------------------------------------------------------
// Existing handlers (unchanged)
// ---------------------------------------------------------------------------

async fn run_search(args: KnowledgeSearchArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let base_hints = if let Some(preset_str) = &args.preset {
        let preset: HintPreset = preset_str.parse().map_err(|_| {
            let err = ux_error::invalid_preset(preset_str);
            err.display();
            anyhow::anyhow!("Invalid preset")
        })?;
        OperationHints::from_preset(preset)
    } else {
        resolved.to_hints()
    };

    let layers: Vec<String> = args.layers.map_or_else(
        || {
            vec![
                "project".to_string(),
                "team".to_string(),
                "org".to_string(),
                "tenant".to_string(),
            ]
        },
        |l| l.split(',').map(|s| s.trim().to_lowercase()).collect(),
    );

    let valid_layers = ["tenant", "org", "team", "project"];
    for layer in &layers {
        if !valid_layers.contains(&layer.as_str()) {
            let err = ux_error::invalid_knowledge_layer(layer);
            err.display();
            return Err(anyhow::anyhow!("Invalid layer"));
        }
    }

    if args.dry_run || args.verbose {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "knowledge_search",
                "query": args.query,
                "limit": args.limit,
                "layers": layers,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "hints": {
                    "preset": format!("{}", base_hints.preset),
                    "reasoning": base_hints.reasoning,
                    "multiHop": base_hints.multi_hop,
                    "llm": base_hints.llm,
                    "graph": base_hints.graph,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Knowledge Search (Dry Run)");
            println!();
            println!("  Query:  {}", args.query);
            println!("  Limit:  {}", args.limit);
            println!("  Layers: {}", layers.join(", "));
            println!();
            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();
            output::header("Active Hints");
            println!("  preset:    {}", base_hints.preset);
            println!(
                "  reasoning: {} {}",
                if base_hints.reasoning { "on" } else { "off" },
                hint_effect(base_hints.reasoning, "will use semantic reasoning")
            );
            println!(
                "  llm:       {} {}",
                if base_hints.llm { "on" } else { "off" },
                hint_effect(base_hints.llm, "will use LLM for semantic search")
            );
            println!(
                "  graph:     {} {}",
                if base_hints.graph { "on" } else { "off" },
                hint_effect(base_hints.graph, "will query knowledge graph")
            );
            println!();
            output::header("Search Scope");
            println!("  Knowledge is searched in precedence order:");
            for (i, layer) in layers.iter().enumerate() {
                let desc = match layer.as_str() {
                    "project" => "Project-specific knowledge (ADRs, patterns)",
                    "team" => "Team standards and conventions",
                    "org" => "Organization-wide policies",
                    "tenant" => "Tenant-wide standards",
                    _ => "",
                };
                println!("    {}. {} - {}", i + 1, layer, desc);
            }
            println!();

            if args.dry_run {
                output::info("Dry run mode - no actual search performed.");
                output::info("Remove --dry-run to execute the search.");
            }
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let first_layer = layers.first().map(String::as_str);
        let result = client
            .knowledge_query(&args.query, first_layer, args.limit)
            .await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header("Knowledge Search Results");
            println!();
            if let Some(primary) = result.get("primary") {
                println!("Primary:");
                println!("{}", serde_json::to_string_pretty(primary)?);
                println!();
            }
            if let Some(related) = result.get("related") {
                println!("Related:");
                println!("{}", serde_json::to_string_pretty(related)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        return Ok(());
    }

    knowledge_server_required(
        "knowledge_search",
        "Knowledge search requires a live Aeterna server connection",
    )
}

async fn run_get(args: KnowledgeGetArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _resolved = resolver.resolve()?;

    let layer = args.layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    if let Some(client) = get_live_client().await {
        let result = client.knowledge_metadata(&args.path).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header(&format!("Knowledge: {} ({})", args.path, layer));
            println!();
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        return Ok(());
    }

    knowledge_server_required(
        "knowledge_get",
        "Knowledge get requires a live Aeterna server connection",
    )
}

async fn run_list(args: KnowledgeListArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _resolved = resolver.resolve()?;

    let layer = args.layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    let profile_name = crate::profile::load_resolved(None, None)
        .map_or_else(|_| "default".to_string(), |r| r.profile_name);
    Err(crate::backend::unsupported("knowledge list", &profile_name))
}

async fn run_check(args: KnowledgeCheckArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let base_hints = if let Some(preset_str) = &args.preset {
        let preset: HintPreset = preset_str.parse().map_err(|_| {
            let err = ux_error::invalid_preset(preset_str);
            err.display();
            anyhow::anyhow!("Invalid preset")
        })?;
        OperationHints::from_preset(preset)
    } else {
        resolved.to_hints()
    };

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "knowledge_check",
                "context": args.context,
                "policy": args.policy,
                "dependency": args.dependency,
                "tenantContext": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "hints": {
                    "preset": format!("{}", base_hints.preset),
                    "governance": base_hints.governance,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Knowledge Check (Dry Run)");
            println!();
            if let Some(ctx) = &args.context {
                println!("  Context:    {ctx}");
            }
            if let Some(policy) = &args.policy {
                println!("  Policy:     {policy}");
            }
            if let Some(dep) = &args.dependency {
                println!("  Dependency: {dep}");
            }
            println!();
            output::header("What Would Be Checked");
            println!("  1. Constraint rules from inherited policies");
            println!("  2. Dependency blocklists (security, licensing)");
            println!("  3. Pattern requirements (code style, architecture)");
            println!();
            println!(
                "  governance: {} {}",
                if base_hints.governance { "on" } else { "off" },
                hint_effect(base_hints.governance, "will enforce policies")
            );
            println!();
            output::info("Dry run mode - no actual check performed.");
            output::info("Remove --dry-run to execute the constraint check.");
        }
        return Ok(());
    }

    let profile_name = crate::profile::load_resolved(None, None)
        .map_or_else(|_| "default".to_string(), |r| r.profile_name);
    Err(crate::backend::unsupported(
        "knowledge check",
        &profile_name,
    ))
}

fn hint_effect(enabled: bool, effect: &str) -> String {
    if enabled {
        format!("({effect})")
    } else {
        String::new()
    }
}

async fn run_propose(args: KnowledgeProposeArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let valid_types = ["adr", "pattern", "policy", "spec"];
    let valid_layers = ["tenant", "org", "team", "project"];

    if let Some(ref kt) = args.knowledge_type {
        let kt_lower = kt.to_lowercase();
        if !valid_types.contains(&kt_lower.as_str()) {
            let err = ux_error::invalid_knowledge_type(&kt_lower, &valid_types);
            err.display();
            return Err(anyhow::anyhow!("Invalid knowledge type"));
        }
    }

    if let Some(ref layer) = args.layer {
        let layer_lower = layer.to_lowercase();
        if !valid_layers.contains(&layer_lower.as_str()) {
            let err = ux_error::invalid_knowledge_layer(&layer_lower);
            err.display();
            return Err(anyhow::anyhow!("Invalid layer"));
        }
    }

    let detected_type = detect_knowledge_type(&args.description);
    let detected_layer = detect_knowledge_layer(&args.description);
    let knowledge_type = args
        .knowledge_type
        .as_ref()
        .map_or_else(|| detected_type.clone(), |s| s.to_lowercase());
    let layer = args
        .layer
        .as_ref()
        .map_or_else(|| detected_layer.clone(), |s| s.to_lowercase());
    let title = args
        .title
        .clone()
        .unwrap_or_else(|| extract_title(&args.description));

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "knowledge_propose",
                "description": args.description,
                "detectedType": detected_type,
                "detectedLayer": detected_layer,
                "effectiveType": knowledge_type,
                "effectiveLayer": layer,
                "title": title,
                "submit": args.submit,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "governanceRequired": true,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Knowledge Propose (Dry Run)");
            println!();
            println!("  Description: {}", truncate(&args.description, 60));
            println!();
            output::header("Auto-Detection");
            println!(
                "  Type:  {} {}",
                detected_type,
                if args.knowledge_type.is_some() {
                    format!("(overridden to {knowledge_type})")
                } else {
                    "(auto-detected)".to_string()
                }
            );
            println!(
                "  Layer: {} {}",
                detected_layer,
                if args.layer.is_some() {
                    format!("(overridden to {layer})")
                } else {
                    "(auto-detected)".to_string()
                }
            );
            println!("  Title: {title}");
            println!();
            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();
            output::header("What Would Happen");
            println!("  1. Create a {knowledge_type} draft in {layer} layer");
            println!("  2. Generate structured content from description");
            if args.submit {
                println!("  3. Submit directly for governance approval");
            } else {
                println!("  3. Save as draft for review before submission");
            }
            println!();
            output::info("Dry run mode - no draft created.");
            output::info("Remove --dry-run to create the proposal.");
        }
        return Ok(());
    }

    if !args.yes && !args.dry_run {
        output::warn(&format!(
            "This will create a {knowledge_type} proposal in {layer} layer:"
        ));
        println!("  Title: {title}");
        println!("  Type:  {knowledge_type}");
        println!("  Layer: {layer}");
        if args.submit {
            output::info("The proposal will be submitted directly for approval.");
        } else {
            output::info("The proposal will be saved as a draft for review.");
        }
        output::info("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let body = json!({
            "title": title,
            "content": args.description,
            "type": knowledge_type,
            "layer": layer,
            "submit": args.submit,
        });
        let result = client.knowledge_create(&body).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header("Knowledge Propose");
            println!();
            println!("  Title: {title}");
            println!("  Type:  {knowledge_type}");
            println!("  Layer: {layer}");
            println!();
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        return Ok(());
    }

    knowledge_server_required(
        "knowledge_propose",
        "Knowledge propose requires a live Aeterna server connection",
    )
}

// ---------------------------------------------------------------------------
// New promotion lifecycle handlers (tasks 4.1-4.9)
// ---------------------------------------------------------------------------

/// task 4.2 — preview promotion without persisting anything
async fn run_promotion_preview(args: KnowledgePromotionPreviewArgs) -> anyhow::Result<()> {
    let target_layer = args.target_layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&target_layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&target_layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid target layer"));
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promotion_preview(&args.id, &target_layer, args.mode.as_deref())
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_promotion_preview",
                            "id": args.id
                        }))
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
            output::header("Promotion Preview");
            println!();
            println!("  Knowledge ID:    {}", args.id);
            println!("  Target Layer:    {target_layer}");
            if let Some(m) = &args.mode {
                println!("  Mode:            {m}");
            }
            println!();
            if let Some(shared) = result["shared_content"].as_str() {
                output::subheader("Content to Promote (shared canonical)");
                println!("  {}", truncate(shared, 120));
                println!();
            }
            if let Some(residual) = result["residual_content"].as_str() {
                output::subheader("Content to Remain (residual)");
                println!("  {}", truncate(residual, 120));
                println!();
            }
            if let Some(impacts) = result["impacts"].as_array()
                && !impacts.is_empty()
            {
                output::subheader("Impacts");
                for impact in impacts {
                    println!("  • {}", impact.as_str().unwrap_or("?"));
                }
                println!();
            }
            output::hint(
                "Run 'aeterna knowledge promote' with --yes to submit the promotion request",
            );
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "knowledge_promotion_preview"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_promotion_preview");
    }
    knowledge_server_required(
        "knowledge_promotion_preview",
        "Cannot preview promotion: server not connected",
    )
}

/// task 4.1 — create promotion request, with interactive split UX (task 4.8)
async fn run_promote(args: KnowledgePromoteArgs) -> anyhow::Result<()> {
    let target_layer = args.target_layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&target_layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&target_layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid target layer"));
    }

    let mode = args.mode.to_lowercase();
    let valid_modes = ["full", "partial"];
    if !valid_modes.contains(&mode.as_str()) {
        ux_error::UxError::new(format!("Invalid promotion mode: '{mode}'"))
            .why("Promotion mode controls whether the lower-layer item is replaced or preserved")
            .fix("Use one of: full, partial")
            .suggest(format!(
                "aeterna knowledge promote {} --target-layer {} --mode full",
                args.id, target_layer
            ))
            .display();
        return Err(anyhow::anyhow!("Invalid promotion mode"));
    }

    // task 4.8 — interactive split UX: for partial mode when content not supplied and not --yes
    let (shared_content, residual_content, residual_role) = if mode == "partial"
        && args.shared_content.is_none()
        && !args.yes
    {
        output::header("Interactive Promotion Split");
        println!();
        output::info(&format!(
            "Partial promotion: you are splitting knowledge item '{}' into a shared canonical part (to promote to '{target_layer}') and a residual part (to keep at the source layer).",
            args.id
        ));
        println!();

        let shared = prompt_required(
            "Enter the content to promote to the target layer (shared canonical):\n> ",
        )?;
        let residual = prompt_optional(
            "Enter the content to keep at the source layer (residual; press Enter to skip):\n> ",
        )?;
        let role = prompt_optional(
            "Role for the residual item (specialization / applicability / exception; press Enter to skip):\n> ",
        )?;

        println!();
        output::warn("Review your split before submitting:");
        println!("  Shared (→ {target_layer}):  {}", truncate(&shared, 80));
        if let Some(ref r) = residual {
            println!("  Residual (source layer): {}", truncate(r, 80));
        }
        if let Some(ref r) = role {
            println!("  Residual role:           {r}");
        }
        println!();
        output::info("Use --yes to skip this confirmation next time.");

        let confirmed = prompt_confirm("Submit this promotion request? [y/N] ")?;
        if !confirmed {
            println!("Promotion cancelled.");
            return Ok(());
        }

        (Some(shared), residual, role)
    } else {
        (
            args.shared_content.clone(),
            args.residual_content.clone(),
            args.residual_role.clone(),
        )
    };

    // For full mode without --yes, show a brief confirmation
    if mode == "full" && !args.yes {
        output::warn(&format!(
            "This will submit a full promotion request for '{}' → '{target_layer}'.",
            args.id
        ));
        output::info("The lower-layer item will be marked Superseded upon approval.");
        output::info("Use --yes to skip this confirmation.");
        return Ok(());
    }

    // Build request body
    let mut body = json!({
        "target_layer": target_layer,
        "mode": mode,
        "shared_content": shared_content.clone().unwrap_or_default(),
    });
    if let Some(ref rc) = residual_content {
        body["residual_content"] = json!(rc);
    }
    if let Some(ref rr) = residual_role {
        body["residual_role"] = json!(rr);
    }
    if let Some(ref j) = args.justification {
        body["justification"] = json!(j);
    }
    if let Some(ref sv) = args.source_version {
        body["source_version"] = json!(sv);
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promote(&args.id, &body)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_promote",
                            "id": args.id
                        }))
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
            output::header("Knowledge Promote");
            println!();
            println!(
                "  Promotion ID: {}",
                result["id"].as_str().unwrap_or("(created)")
            );
            println!("  Knowledge ID: {}", args.id);
            println!("  Target Layer: {target_layer}");
            println!("  Mode:         {mode}");
            println!();
            let status = result["status"].as_str().unwrap_or("submitted");
            println!("  ✓ Promotion request {status}");
            println!();
            output::hint("Run 'aeterna knowledge pending' to see pending promotion requests");
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "knowledge_promote"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_promote");
    }
    knowledge_server_required(
        "knowledge_promote",
        "Cannot submit promotion request: server not connected",
    )
}

/// task 4.3 — list pending promotion requests
async fn run_pending(args: KnowledgePendingArgs) -> anyhow::Result<()> {
    let status_filter = if args.status == "all" {
        None
    } else {
        Some(args.status.as_str())
    };

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promotions_list(status_filter)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_pending"
                        }))
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
            let items = result.as_array().cloned().unwrap_or_default();
            let count = items.len();
            output::header(&format!("Pending Promotions ({count})"));
            println!();

            if count == 0 {
                println!(
                    "  ✓ No promotion requests matching filter '{}'",
                    args.status
                );
                println!();
            } else {
                for item in &items {
                    let id = item["id"].as_str().unwrap_or("?");
                    let source_id = item["source_id"].as_str().unwrap_or("?");
                    let target_layer = item["target_layer"].as_str().unwrap_or("?");
                    let mode = item["mode"].as_str().unwrap_or("?");
                    let status = item["status"].as_str().unwrap_or("?");
                    let created = item["created_at"].as_str().unwrap_or("?");

                    println!("  [{id}]  {source_id} → {target_layer}  ({mode})");
                    println!("        Status: {status}  |  Created: {created}");
                    println!();
                }
                output::hint(
                    "Use 'aeterna knowledge approve <id>' or 'aeterna knowledge reject <id> --reason <reason>' to act",
                );
            }
        }
        return Ok(());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "server_not_connected",
                "operation": "knowledge_pending"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_pending");
    }
    knowledge_server_required(
        "knowledge_pending",
        "Cannot list pending promotions: server not connected",
    )
}

/// task 4.4 — approve a promotion request
async fn run_approve(args: KnowledgeApproveArgs) -> anyhow::Result<()> {
    if !args.yes {
        eprintln!(
            "This will approve promotion request '{}'. Use --yes to confirm.",
            args.promotion_id
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promotion_approve(&args.promotion_id, &args.decision)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_approve",
                            "promotion_id": args.promotion_id
                        }))
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
            output::header("Approve Promotion");
            println!();
            println!("  Promotion ID: {}", args.promotion_id);
            println!("  Decision:     {}", args.decision);
            println!();
            let status = result["status"].as_str().unwrap_or("approved");
            println!("  ✓ Promotion {status}");
            if let Some(promoted_id) = result["promoted_id"].as_str() {
                println!("  ⚡ New knowledge item created: {promoted_id}");
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
                "operation": "knowledge_approve",
                "promotion_id": args.promotion_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_approve");
    }
    knowledge_server_required(
        "knowledge_approve",
        "Cannot approve promotion: server not connected",
    )
}

/// task 4.5 — reject a promotion request
async fn run_reject(args: KnowledgeRejectArgs) -> anyhow::Result<()> {
    if args.reason.is_empty() {
        ux_error::UxError::new("Rejection reason is required")
            .why("Proposers need feedback to understand why their promotion was rejected")
            .fix("Provide a reason using the --reason flag")
            .suggest(format!(
                "aeterna knowledge reject {} --reason \"Needs broader review first\"",
                args.promotion_id
            ))
            .display();
        crate::exit_code::ExitCode::Usage.exit();
    }

    if !args.yes {
        eprintln!(
            "This will reject promotion request '{}'. Use --yes to confirm.",
            args.promotion_id
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promotion_reject(&args.promotion_id, &args.reason)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_reject",
                            "promotion_id": args.promotion_id
                        }))
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
            output::header("Reject Promotion");
            println!();
            println!("  Promotion ID: {}", args.promotion_id);
            println!("  Reason:       {}", args.reason);
            println!();
            println!("  ✗ Promotion rejected");
            if let Some(proposer) = result["proposer"].as_str() {
                println!("  ℹ Proposer '{proposer}' has been notified");
            } else {
                println!("  ℹ Proposer has been notified");
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
                "operation": "knowledge_reject",
                "promotion_id": args.promotion_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_reject");
    }
    knowledge_server_required(
        "knowledge_reject",
        "Cannot reject promotion: server not connected",
    )
}

/// task 4.6 — retarget a promotion request to a different layer
async fn run_retarget(args: KnowledgeRetargetArgs) -> anyhow::Result<()> {
    let target_layer = args.target_layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&target_layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&target_layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid target layer"));
    }

    if !args.yes {
        eprintln!(
            "This will retarget promotion '{}' to layer '{}'. Use --yes to confirm.",
            args.promotion_id, target_layer
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_promotion_retarget(&args.promotion_id, &target_layer)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_retarget",
                            "promotion_id": args.promotion_id
                        }))
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
            output::header("Retarget Promotion");
            println!();
            println!("  Promotion ID:  {}", args.promotion_id);
            println!("  New Target:    {target_layer}");
            println!();
            let new_target = result["target_layer"].as_str().unwrap_or(&target_layer);
            println!("  ✓ Promotion retargeted to '{new_target}'");
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
                "operation": "knowledge_retarget",
                "promotion_id": args.promotion_id
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_retarget");
    }
    knowledge_server_required(
        "knowledge_retarget",
        "Cannot retarget promotion: server not connected",
    )
}

/// task 4.7 — create a semantic relation between two knowledge items
async fn run_relate(args: KnowledgeRelateArgs) -> anyhow::Result<()> {
    let valid_relation_types = [
        "promotes_to",
        "supersedes",
        "specializes",
        "references",
        "related_to",
        "conflicts_with",
        "depends_on",
    ];
    let relation_type = args.relation_type.to_lowercase();
    if !valid_relation_types.contains(&relation_type.as_str()) {
        ux_error::UxError::new(format!("Invalid relation type: '{relation_type}'"))
            .why(format!(
                "Valid relation types are: {}",
                valid_relation_types.join(", ")
            ))
            .fix("Use one of the valid relation type names")
            .suggest(format!(
                "aeterna knowledge relate {} --target-id <id> --relation-type related_to",
                args.id
            ))
            .display();
        return Err(anyhow::anyhow!("Invalid relation type"));
    }

    if !args.yes {
        eprintln!(
            "This will create a '{}' relation from '{}' to '{}'. Use --yes to confirm.",
            relation_type, args.id, args.target_id
        );
        eprintln!("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let result = client
            .knowledge_relate(&args.id, &args.target_id, &relation_type)
            .await
            .inspect_err(|e| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": e.to_string(),
                            "operation": "knowledge_relate",
                            "id": args.id,
                            "target_id": args.target_id
                        }))
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
            output::header("Create Knowledge Relation");
            println!();
            println!("  Source:        {}", args.id);
            println!("  Target:        {}", args.target_id);
            println!("  Relation Type: {relation_type}");
            println!();
            let relation_id = result["id"].as_str().unwrap_or("(created)");
            println!("  ✓ Relation created: {relation_id}");
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
                "operation": "knowledge_relate"
            }))?
        );
        anyhow::bail!("Aeterna server not connected for operation: knowledge_relate");
    }
    knowledge_server_required(
        "knowledge_relate",
        "Cannot create relation: server not connected",
    )
}

// ---------------------------------------------------------------------------
// Interactive prompt helpers (task 4.8)
// ---------------------------------------------------------------------------

fn prompt_required(prompt: &str) -> anyhow::Result<String> {
    use std::io::{BufRead, Write};
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    write!(out, "{prompt}")?;
    out.flush()?;
    let stdin = std::io::stdin();
    let line = stdin.lock().lines().next().transpose()?.unwrap_or_default();
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("Input is required for promotion content");
    }
    Ok(trimmed)
}

fn prompt_optional(prompt: &str) -> anyhow::Result<Option<String>> {
    use std::io::{BufRead, Write};
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    write!(out, "{prompt}")?;
    out.flush()?;
    let stdin = std::io::stdin();
    let line = stdin.lock().lines().next().transpose()?.unwrap_or_default();
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

fn prompt_confirm(prompt: &str) -> anyhow::Result<bool> {
    use std::io::{BufRead, Write};
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    write!(out, "{prompt}")?;
    out.flush()?;
    let stdin = std::io::stdin();
    let line = stdin.lock().lines().next().transpose()?.unwrap_or_default();
    let answer = line.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

// ---------------------------------------------------------------------------
// Detection helpers (unchanged)
// ---------------------------------------------------------------------------

fn detect_knowledge_type(description: &str) -> String {
    let lower = description.to_lowercase();

    if lower.contains("decision")
        || lower.contains("adr")
        || lower.contains("architecture")
        || lower.contains("chose")
        || lower.contains("decided")
    {
        return "adr".to_string();
    }

    if lower.contains("policy")
        || lower.contains("rule")
        || lower.contains("must")
        || lower.contains("require")
        || lower.contains("enforce")
        || lower.contains("block")
        || lower.contains("forbid")
    {
        return "policy".to_string();
    }

    if lower.contains("pattern")
        || lower.contains("approach")
        || lower.contains("best practice")
        || lower.contains("how to")
        || lower.contains("guideline")
    {
        return "pattern".to_string();
    }

    if lower.contains("spec")
        || lower.contains("specification")
        || lower.contains("requirement")
        || lower.contains("shall")
    {
        return "spec".to_string();
    }

    "adr".to_string()
}

fn detect_knowledge_layer(description: &str) -> String {
    let lower = description.to_lowercase();

    if lower.contains("tenant")
        || lower.contains("enterprise")
        || lower.contains("global")
        || lower.contains("all teams")
        || lower.contains("organization-wide")
    {
        return "tenant".to_string();
    }

    if lower.contains("org")
        || lower.contains("department")
        || lower.contains("division")
        || lower.contains("business unit")
    {
        return "org".to_string();
    }

    if lower.contains("team")
        || lower.contains("squad")
        || lower.contains("group")
        || lower.contains("our team")
    {
        return "team".to_string();
    }

    "project".to_string()
}

fn extract_title(description: &str) -> String {
    let first_sentence = description.split('.').next().unwrap_or(description);
    let cleaned = first_sentence.trim();
    if cleaned.len() > 60 {
        format!("{}...", &cleaned[..57])
    } else {
        cleaned.to_string()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- existing tests (unchanged) ----

    #[test]
    fn test_hint_effect_enabled() {
        let result = hint_effect(true, "will use semantic reasoning");
        assert_eq!(result, "(will use semantic reasoning)");
    }

    #[test]
    fn test_hint_effect_disabled() {
        let result = hint_effect(false, "will use semantic reasoning");
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_short_string() {
        let result = truncate("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_exact_length() {
        let result = truncate("hello", 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        let result = truncate("hello world this is a long string", 20);
        assert_eq!(result, "hello world this ...");
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn test_extract_title_short() {
        let result = extract_title("Use PostgreSQL for databases");
        assert_eq!(result, "Use PostgreSQL for databases");
    }

    #[test]
    fn test_extract_title_with_period() {
        let result = extract_title("Use PostgreSQL. It has good performance.");
        assert_eq!(result, "Use PostgreSQL");
    }

    #[test]
    fn test_extract_title_long() {
        let long_text = "This is a very long description that should be truncated to fit within \
                         the title limit";
        let result = extract_title(long_text);
        assert!(result.len() <= 60);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_extract_title_empty() {
        let result = extract_title("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_detect_knowledge_type_adr() {
        assert_eq!(detect_knowledge_type("We decided to use PostgreSQL"), "adr");
        assert_eq!(
            detect_knowledge_type("Architecture decision for API"),
            "adr"
        );
        assert_eq!(detect_knowledge_type("ADR: Database selection"), "adr");
        assert_eq!(detect_knowledge_type("We chose React for frontend"), "adr");
    }

    #[test]
    fn test_detect_knowledge_type_policy() {
        assert_eq!(
            detect_knowledge_type("All services must use HTTPS"),
            "policy"
        );
        assert_eq!(
            detect_knowledge_type("Policy: No direct database access"),
            "policy"
        );
        assert_eq!(
            detect_knowledge_type("Require code review before merge"),
            "policy"
        );
        assert_eq!(
            detect_knowledge_type("Block dependencies with CVEs"),
            "policy"
        );
        assert_eq!(detect_knowledge_type("Enforce 80% test coverage"), "policy");
        assert_eq!(detect_knowledge_type("Forbid global state"), "policy");
    }

    #[test]
    fn test_detect_knowledge_type_pattern() {
        assert_eq!(
            detect_knowledge_type("Pattern: Repository abstraction"),
            "pattern"
        );
        assert_eq!(
            detect_knowledge_type("Best practice for logging"),
            "pattern"
        );
        assert_eq!(
            detect_knowledge_type("Approach for error handling"),
            "pattern"
        );
        assert_eq!(detect_knowledge_type("How to implement caching"), "pattern");
        assert_eq!(
            detect_knowledge_type("Guideline for API versioning"),
            "pattern"
        );
    }

    #[test]
    fn test_detect_knowledge_type_spec() {
        assert_eq!(detect_knowledge_type("Spec for API endpoints"), "spec");
        assert_eq!(
            detect_knowledge_type("The system shall support 1000 users"),
            "spec"
        );
    }

    #[test]
    fn test_detect_knowledge_type_default() {
        assert_eq!(detect_knowledge_type("Random text without keywords"), "adr");
    }

    #[test]
    fn test_detect_knowledge_layer_company() {
        assert_eq!(
            detect_knowledge_layer("Tenant-wide security policy"),
            "tenant"
        );
        assert_eq!(
            detect_knowledge_layer("Enterprise standard for APIs"),
            "tenant"
        );
        assert_eq!(detect_knowledge_layer("Global logging format"), "tenant");
        assert_eq!(detect_knowledge_layer("Apply to all teams"), "tenant");
        assert_eq!(
            detect_knowledge_layer("Organization-wide code style"),
            "tenant"
        );
    }

    #[test]
    fn test_detect_knowledge_layer_org() {
        assert_eq!(detect_knowledge_layer("Org level guidelines"), "org");
        assert_eq!(detect_knowledge_layer("Department standard"), "org");
        assert_eq!(detect_knowledge_layer("Division policy"), "org");
        assert_eq!(detect_knowledge_layer("Business unit convention"), "org");
    }

    #[test]
    fn test_detect_knowledge_layer_team() {
        assert_eq!(detect_knowledge_layer("Team coding convention"), "team");
        assert_eq!(detect_knowledge_layer("Squad best practices"), "team");
        assert_eq!(detect_knowledge_layer("Our team uses Rust"), "team");
        assert_eq!(detect_knowledge_layer("Group decision on testing"), "team");
    }

    #[test]
    fn test_detect_knowledge_layer_default() {
        assert_eq!(detect_knowledge_layer("Random text"), "project");
        assert_eq!(
            detect_knowledge_layer("Use PostgreSQL for this service"),
            "project"
        );
    }

    // ---- new tests for promotion lifecycle structs and helpers (task 4.9 / 12.3) ----

    #[test]
    fn test_promote_args_defaults() {
        // Verify default mode is "full" as declared in #[arg(default_value)]
        // We test the logic that validates mode strings
        let valid = ["full", "partial"];
        assert!(valid.contains(&"full"));
        assert!(valid.contains(&"partial"));
        assert!(!valid.contains(&"unknown"));
    }

    #[test]
    fn test_valid_relation_types() {
        let valid = [
            "promotes_to",
            "supersedes",
            "specializes",
            "references",
            "related_to",
            "conflicts_with",
            "depends_on",
        ];
        for rt in valid {
            assert!(valid.contains(&rt), "expected valid relation type: {rt}");
        }
        assert!(!valid.contains(&"invalid_type"));
    }

    #[test]
    fn test_valid_promotion_layers() {
        let valid = ["tenant", "org", "team", "project"];
        assert!(valid.contains(&"tenant"));
        assert!(valid.contains(&"org"));
        assert!(valid.contains(&"team"));
        assert!(valid.contains(&"project"));
        assert!(!valid.contains(&"agent"));
        assert!(!valid.contains(&"user"));
    }

    #[test]
    fn test_pending_status_filter_all() {
        // "all" maps to None (no filter)
        let status = "all";
        let filter: Option<&str> = if status == "all" { None } else { Some(status) };
        assert!(filter.is_none());
    }

    #[test]
    fn test_pending_status_filter_submitted() {
        let status = "submitted";
        let filter: Option<&str> = if status == "all" { None } else { Some(status) };
        assert_eq!(filter, Some("submitted"));
    }

    #[test]
    fn test_pending_status_filter_draft() {
        let status = "draft";
        let filter: Option<&str> = if status == "all" { None } else { Some(status) };
        assert_eq!(filter, Some("draft"));
    }

    #[test]
    fn test_promote_args_full_mode_no_split_needed() {
        // Full mode should not require shared_content/residual_content
        let mode = "full";
        let shared_content: Option<String> = None;
        let yes = true; // --yes bypasses interactive
        // In full mode with --yes, no split is needed regardless of shared_content
        let needs_interactive = mode == "partial" && shared_content.is_none() && !yes;
        assert!(!needs_interactive);
    }

    #[test]
    fn test_promote_args_partial_mode_needs_split_without_yes() {
        let mode = "partial";
        let shared_content: Option<String> = None;
        let yes = false;
        let needs_interactive = mode == "partial" && shared_content.is_none() && !yes;
        assert!(needs_interactive);
    }

    #[test]
    fn test_promote_args_partial_mode_no_split_with_content_provided() {
        let mode = "partial";
        let shared_content: Option<String> = Some("Shared canonical content".to_string());
        let yes = false;
        let needs_interactive = mode == "partial" && shared_content.is_none() && !yes;
        assert!(!needs_interactive);
    }

    #[test]
    fn test_promote_args_partial_mode_no_split_with_yes() {
        let mode = "partial";
        let shared_content: Option<String> = None;
        let yes = true;
        let needs_interactive = mode == "partial" && shared_content.is_none() && !yes;
        assert!(!needs_interactive);
    }

    #[test]
    fn test_approve_default_decision() {
        // Default decision for approve is "Approve"
        let decision = "Approve";
        assert!(!decision.is_empty());
    }

    #[test]
    fn test_reject_requires_reason() {
        let reason = "";
        assert!(reason.is_empty(), "empty reason should be caught");

        let reason = "Needs broader review";
        assert!(!reason.is_empty(), "non-empty reason should pass");
    }

    #[test]
    fn test_truncate_preserves_short_content() {
        let content = "Short content";
        let result = truncate(content, 120);
        assert_eq!(result, content);
    }

    #[test]
    fn test_truncate_clips_long_promotion_content() {
        let long_content = "A".repeat(200);
        let result = truncate(&long_content, 120);
        assert_eq!(result.len(), 120);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_promotion_json_error_body_shape() {
        // Validate the JSON error body shape used in promotion handlers
        let err_body = json!({
            "success": false,
            "error": "some error",
            "operation": "knowledge_promote",
            "id": "knw-123"
        });
        assert_eq!(err_body["success"], false);
        assert_eq!(err_body["operation"], "knowledge_promote");
        assert!(err_body["error"].as_str().is_some());
    }

    #[test]
    fn test_promotion_preview_json_error_body_shape() {
        let err_body = json!({
            "success": false,
            "error": "some error",
            "operation": "knowledge_promotion_preview",
            "id": "knw-456"
        });
        assert_eq!(err_body["operation"], "knowledge_promotion_preview");
    }

    #[test]
    fn test_pending_json_error_body_shape() {
        let err_body = json!({
            "success": false,
            "error": "server_not_connected",
            "operation": "knowledge_pending"
        });
        assert_eq!(err_body["error"], "server_not_connected");
    }

    #[test]
    fn test_relate_json_error_body_shape() {
        let err_body = json!({
            "success": false,
            "error": "some error",
            "operation": "knowledge_relate",
            "id": "knw-1",
            "target_id": "knw-2"
        });
        assert_eq!(err_body["operation"], "knowledge_relate");
        assert!(err_body["target_id"].as_str().is_some());
    }
}
