use clap::{Args, Subcommand};
use context::ContextResolver;
use mk_core::hints::{HintPreset, OperationHints};
use serde_json::json;

use crate::output;
use crate::ux_error;

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    crate::backend::connect()
        .await
        .ok()
        .map(|(client, _)| client)
}

fn memory_server_required(operation: &str, message: &str) -> anyhow::Result<()> {
    ux_error::UxError::new(message)
        .why("This memory command requires a live control-plane backend")
        .fix("Start the Aeterna server: aeterna serve")
        .fix("Ensure AETERNA_SERVER_URL is set or a profile is configured and reachable")
        .suggest("aeterna auth login")
        .display();
    anyhow::bail!("Aeterna server not connected for operation: {operation}")
}

#[derive(Subcommand)]
pub enum MemoryCommand {
    #[command(about = "Search memories across layers")]
    Search(MemorySearchArgs),

    #[command(about = "Add a new memory")]
    Add(MemoryAddArgs),

    #[command(about = "Delete a memory by ID")]
    Delete(MemoryDeleteArgs),

    #[command(about = "List memories in a layer")]
    List(MemoryListArgs),

    #[command(about = "Show memory details by ID")]
    Show(MemoryShowArgs),

    #[command(about = "Provide feedback on a memory")]
    Feedback(MemoryFeedbackArgs),

    #[command(about = "Promote a memory to a broader layer")]
    Promote(MemoryPromoteArgs),
}

#[derive(Args)]
pub struct MemorySearchArgs {
    /// Search query
    pub query: String,

    /// Maximum number of results (default: 10)
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Minimum similarity threshold (0.0-1.0)
    #[arg(short, long, default_value = "0.0")]
    pub threshold: f32,

    /// Filter by layer (agent, user, session, project, team, org, tenant)
    #[arg(long)]
    pub layer: Option<String>,

    /// Hints preset (minimal, fast, standard, full, offline, agent)
    #[arg(long)]
    pub preset: Option<String>,

    /// Enable reasoning (overrides preset)
    #[arg(long)]
    pub reasoning: Option<bool>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show verbose output (what hints would do)
    #[arg(short, long)]
    pub verbose: bool,

    /// Dry run - don't actually search, just show what would happen
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct MemoryAddArgs {
    /// Memory content to store
    pub content: String,

    /// Layer to store in (agent, user, session, project, team, org, tenant)
    #[arg(short, long, default_value = "project")]
    pub layer: String,

    /// Tags for the memory (comma-separated)
    #[arg(short, long)]
    pub tags: Option<String>,

    /// Additional metadata as JSON
    #[arg(short, long)]
    pub metadata: Option<String>,

    /// Hints preset
    #[arg(long)]
    pub preset: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - don't actually store, just show what would happen
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct MemoryDeleteArgs {
    /// Memory ID to delete
    pub memory_id: String,

    /// Layer the memory is in
    #[arg(short, long)]
    pub layer: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MemoryListArgs {
    /// Layer to list (agent, user, session, project, team, org, tenant)
    #[arg(short, long, default_value = "project")]
    pub layer: String,

    /// Maximum number of results
    #[arg(short = 'n', long, default_value = "20")]
    pub limit: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MemoryShowArgs {
    /// Memory ID to show
    pub memory_id: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MemoryFeedbackArgs {
    /// Memory ID to provide feedback for
    pub memory_id: String,

    /// Layer the memory is in
    #[arg(short, long)]
    pub layer: String,

    /// Feedback type (helpful, irrelevant, outdated, inaccurate, duplicate)
    #[arg(short = 't', long)]
    pub feedback_type: String,

    /// Score (-1.0 to 1.0)
    #[arg(short, long)]
    pub score: f32,

    /// Optional reasoning for the feedback
    #[arg(short, long)]
    pub reasoning: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MemoryPromoteArgs {
    /// Memory ID to promote
    pub memory_id: String,

    /// Current layer the memory is in
    #[arg(short, long)]
    pub from_layer: String,

    /// Target layer to promote to (must be broader than current)
    #[arg(short, long)]
    pub to_layer: String,

    /// Reason for promotion
    #[arg(short, long)]
    pub reason: Option<String>,

    /// Skip governance approval (requires admin role)
    #[arg(long)]
    pub skip_approval: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - don't actually promote, just show what would happen
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(cmd: MemoryCommand) -> anyhow::Result<()> {
    match cmd {
        MemoryCommand::Search(args) => run_search(args).await,
        MemoryCommand::Add(args) => run_add(args).await,
        MemoryCommand::Delete(args) => run_delete(args).await,
        MemoryCommand::List(args) => run_list(args).await,
        MemoryCommand::Show(args) => run_show(args).await,
        MemoryCommand::Feedback(args) => run_feedback(args).await,
        MemoryCommand::Promote(args) => run_promote(args).await,
    }
}

async fn run_search(args: MemorySearchArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    // Determine hints
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

    // Apply overrides
    let hints = if let Some(reasoning) = args.reasoning {
        base_hints.with_reasoning(reasoning)
    } else {
        base_hints
    };

    if args.dry_run || args.verbose {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "memory_search",
                "query": args.query,
                "limit": args.limit,
                "threshold": args.threshold,
                "layer": args.layer,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "hints": {
                    "preset": format!("{}", hints.preset),
                    "reasoning": hints.reasoning,
                    "multiHop": hints.multi_hop,
                    "summarization": hints.summarization,
                    "caching": hints.caching,
                    "llm": hints.llm,
                    "graph": hints.graph,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Memory Search (Dry Run)");
            println!();
            println!("  Query:     {}", args.query);
            println!("  Limit:     {}", args.limit);
            println!("  Threshold: {}", args.threshold);
            if let Some(layer) = &args.layer {
                println!("  Layer:     {layer}");
            }
            println!();
            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();
            output::header("Active Hints");
            println!("  preset:       {}", hints.preset);
            println!(
                "  reasoning:    {} {}",
                if hints.reasoning { "on" } else { "off" },
                hint_effect(hints.reasoning, "will use MemR³ reflective reasoning")
            );
            println!(
                "  multi_hop:    {} {}",
                if hints.multi_hop { "on" } else { "off" },
                hint_effect(hints.multi_hop, "will follow graph relationships")
            );
            println!(
                "  summarization: {} {}",
                if hints.summarization { "on" } else { "off" },
                hint_effect(hints.summarization, "will summarize results")
            );
            println!(
                "  caching:      {} {}",
                if hints.caching { "on" } else { "off" },
                hint_effect(hints.caching, "will cache query results")
            );
            println!(
                "  llm:          {} {}",
                if hints.llm { "on" } else { "off" },
                hint_effect(hints.llm, "will use LLM for embeddings")
            );
            println!(
                "  graph:        {} {}",
                if hints.graph { "on" } else { "off" },
                hint_effect(hints.graph, "will query knowledge graph")
            );
            println!();

            if args.dry_run {
                output::info("Dry run mode - no actual search performed.");
                output::info("Remove --dry-run to execute the search.");
            }
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let filters = match args.layer.as_ref() {
            Some(layer) => {
                let mut m = serde_json::Map::new();
                m.insert("layer".to_string(), json!(layer));
                m
            }
            None => serde_json::Map::new(),
        };
        let req = crate::client::MemorySearchRequest {
            query: args.query.clone(),
            limit: args.limit,
            threshold: args.threshold,
            filters,
            context_summary: None,
        };
        let result = client.memory_search(&req).await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header("Memory Search Results");
            println!();
            println!("  Total: {}", result.total);
            println!();
            for item in result.items {
                println!("- [{}] {}", item.id, item.content);
            }
        }
        return Ok(());
    }

    memory_server_required(
        "memory search",
        "Memory search requires a live Aeterna server connection",
    )
}

async fn run_add(args: MemoryAddArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    // Validate layer
    let layer = args.layer.to_lowercase();
    let valid_layers = [
        "agent", "user", "session", "project", "team", "org", "tenant",
    ];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_layer(&layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    // Parse tags
    let tags: Vec<String> = args
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    // Parse metadata
    let metadata: serde_json::Value = if let Some(meta_str) = &args.metadata {
        serde_json::from_str(meta_str).map_err(|e| {
            let err = ux_error::invalid_metadata_json(&e.to_string());
            err.display();
            anyhow::anyhow!("Invalid metadata JSON")
        })?
    } else {
        json!({})
    };

    // Determine hints
    let hints = if let Some(preset_str) = &args.preset {
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
                "operation": "memory_add",
                "content": args.content,
                "layer": layer,
                "tags": tags,
                "metadata": metadata,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "hints": {
                    "preset": format!("{}", hints.preset),
                    "governance": hints.governance,
                    "audit": hints.audit,
                    "llm": hints.llm,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Memory Add (Dry Run)");
            println!();
            println!("  Content: {}", truncate(&args.content, 60));
            println!("  Layer:   {layer}");
            if !tags.is_empty() {
                println!("  Tags:    {}", tags.join(", "));
            }
            if metadata != json!({}) {
                println!("  Metadata: {metadata}");
            }
            println!();
            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();
            output::header("Active Hints");
            println!(
                "  governance: {} {}",
                if hints.governance { "on" } else { "off" },
                hint_effect(hints.governance, "will check policies before storing")
            );
            println!(
                "  audit:      {} {}",
                if hints.audit { "on" } else { "off" },
                hint_effect(hints.audit, "will log to audit trail")
            );
            println!(
                "  llm:        {} {}",
                if hints.llm { "on" } else { "off" },
                hint_effect(hints.llm, "will generate embeddings")
            );
            println!();
            output::info("Dry run mode - memory not stored.");
            output::info("Remove --dry-run to store the memory.");
        }
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let mut metadata_map = serde_json::Map::new();
        if let Some(obj) = metadata.as_object() {
            metadata_map = obj.clone();
        }
        if !tags.is_empty() {
            metadata_map.insert("tags".to_string(), json!(tags));
        }

        let req = crate::client::MemoryAddRequest {
            content: args.content.clone(),
            layer: layer.clone(),
            metadata: metadata_map,
        };
        let result = client.memory_add(&req).await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::success(&format!(
                "Stored memory {} in {} layer",
                result.memory_id, result.layer
            ));
        }
        return Ok(());
    }

    memory_server_required(
        "memory add",
        "Memory add requires a live Aeterna server connection",
    )
}

async fn run_delete(args: MemoryDeleteArgs) -> anyhow::Result<()> {
    // Validate layer
    let layer = args.layer.to_lowercase();
    let valid_layers = [
        "agent", "user", "session", "project", "team", "org", "tenant",
    ];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_layer(&layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    if !args.yes {
        output::warn(&format!(
            "This will delete memory '{}' from layer '{}'.",
            args.memory_id, layer
        ));
        output::info("Use --yes to skip this confirmation.");
        return Ok(());
    }

    if let Some(client) = get_live_client().await {
        let req = crate::client::MemoryDeleteRequest {
            layer: layer.clone(),
        };
        let result = client.memory_delete(&args.memory_id, &req).await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::success(&result.message);
        }
        return Ok(());
    }

    memory_server_required(
        "memory delete",
        "Memory delete requires a live Aeterna server connection",
    )
}

async fn run_list(args: MemoryListArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _resolved = resolver.resolve()?;

    // Validate layer
    let layer = args.layer.to_lowercase();
    let valid_layers = [
        "agent", "user", "session", "project", "team", "org", "tenant",
    ];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_layer(&layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    if let Some(client) = get_live_client().await {
        let req = crate::client::MemoryListRequest {
            layer: layer.clone(),
            limit: args.limit,
        };
        let result = client.memory_list(&req).await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::header(&format!("Memories in {layer} layer"));
            println!();
            println!("  Total: {}", result.total);
            println!();
            for item in result.items {
                println!("- [{}] {}", item.id, item.content);
            }
        }
        return Ok(());
    }

    memory_server_required(
        "memory list",
        "Memory list requires a live Aeterna server connection",
    )
}

async fn run_show(_args: MemoryShowArgs) -> anyhow::Result<()> {
    let profile_name = crate::profile::load_resolved(None, None)
        .map_or_else(|_| "default".to_string(), |r| r.profile_name);
    Err(crate::backend::unsupported("memory show", &profile_name))
}

async fn run_feedback(args: MemoryFeedbackArgs) -> anyhow::Result<()> {
    // Validate layer
    let layer = args.layer.to_lowercase();
    let valid_layers = [
        "agent", "user", "session", "project", "team", "org", "tenant",
    ];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_layer(&layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    // Validate feedback type
    let feedback_type = args.feedback_type.to_lowercase();
    let valid_types = [
        "helpful",
        "irrelevant",
        "outdated",
        "inaccurate",
        "duplicate",
    ];
    if !valid_types.contains(&feedback_type.as_str()) {
        let err = ux_error::invalid_feedback_type(&feedback_type);
        err.display();
        return Err(anyhow::anyhow!("Invalid feedback type"));
    }

    // Validate score
    if args.score < -1.0 || args.score > 1.0 {
        let err = ux_error::invalid_score(args.score);
        err.display();
        return Err(anyhow::anyhow!("Invalid score"));
    }

    if let Some(client) = get_live_client().await {
        let req = crate::client::MemoryFeedbackRequest {
            memory_id: args.memory_id.clone(),
            layer: layer.clone(),
            reward_type: feedback_type.clone(),
            score: args.score,
            reasoning: args.reasoning.clone(),
        };
        let result = client.memory_feedback(&req).await?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            output::success(&result.message);
        }
        return Ok(());
    }

    memory_server_required(
        "memory feedback",
        "Memory feedback requires a live Aeterna server connection",
    )
}

fn hint_effect(enabled: bool, effect: &str) -> String {
    if enabled {
        format!("({effect})")
    } else {
        String::new()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s.to_string()
    }
}

fn layer_order(l: &str) -> usize {
    match l {
        "agent" => 0,
        "user" => 1,
        "session" => 2,
        "project" => 3,
        "team" => 4,
        "org" => 5,
        "tenant" => 6,
        _ => 0,
    }
}

async fn run_promote(args: MemoryPromoteArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    let from_layer = args.from_layer.to_lowercase();
    let to_layer = args.to_layer.to_lowercase();
    let valid_layers = [
        "agent", "user", "session", "project", "team", "org", "tenant",
    ];

    if !valid_layers.contains(&from_layer.as_str()) {
        let err = ux_error::invalid_layer(&from_layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid source layer"));
    }

    if !valid_layers.contains(&to_layer.as_str()) {
        let err = ux_error::invalid_layer(&to_layer, &valid_layers);
        err.display();
        return Err(anyhow::anyhow!("Invalid target layer"));
    }

    if layer_order(&to_layer) <= layer_order(&from_layer) {
        let err = ux_error::promotion_direction_invalid(&from_layer, &to_layer);
        err.display();
        return Err(anyhow::anyhow!("Cannot promote to same or narrower layer"));
    }

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "memory_promote",
                "memoryId": args.memory_id,
                "fromLayer": from_layer,
                "toLayer": to_layer,
                "reason": args.reason,
                "skipApproval": args.skip_approval,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "governanceRequired": !args.skip_approval,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Memory Promote (Dry Run)");
            println!();
            println!("  Memory ID:   {}", args.memory_id);
            println!("  From Layer:  {from_layer}");
            println!("  To Layer:    {to_layer}");
            if let Some(reason) = &args.reason {
                println!("  Reason:      {reason}");
            }
            println!();
            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();
            output::header("Governance");
            if args.skip_approval {
                output::warn("Skipping approval - requires admin role");
            } else {
                println!("  Promotion will require approval based on governance config");
                println!("  Use --skip-approval to bypass (admin only)");
            }
            println!();
            output::info("Dry run mode - no promotion performed.");
            output::info("Remove --dry-run to execute the promotion.");
        }
        return Ok(());
    }

    if !args.yes && !args.dry_run {
        output::warn(&format!(
            "This will promote memory '{}' from '{}' to '{}' layer.",
            args.memory_id, from_layer, to_layer
        ));
        if !args.skip_approval {
            output::info("This action may require governance approval.");
        }
        output::info("Use --yes to skip this confirmation.");
        return Ok(());
    }

    let profile_name = crate::profile::load_resolved(None, None)
        .map_or_else(|_| "default".to_string(), |r| r.profile_name);
    Err(crate::backend::unsupported("memory promote", &profile_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hint_effect_enabled() {
        let result = hint_effect(true, "will do something");
        assert_eq!(result, "(will do something)");
    }

    #[test]
    fn test_hint_effect_disabled() {
        let result = hint_effect(false, "will do something");
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
        let result = truncate("hello world this is a long string", 10);
        assert_eq!(result, "hello w...");
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_truncate_with_unicode() {
        let result = truncate("héllo", 10);
        assert_eq!(result, "héllo");
    }

    #[test]
    fn test_layer_order_all_layers() {
        assert_eq!(layer_order("agent"), 0);
        assert_eq!(layer_order("user"), 1);
        assert_eq!(layer_order("session"), 2);
        assert_eq!(layer_order("project"), 3);
        assert_eq!(layer_order("team"), 4);
        assert_eq!(layer_order("org"), 5);
        assert_eq!(layer_order("tenant"), 6);
    }

    #[test]
    fn test_layer_order_unknown() {
        assert_eq!(layer_order("unknown"), 0);
        assert_eq!(layer_order(""), 0);
    }

    #[test]
    fn test_layer_hierarchy_promotion_valid() {
        assert!(layer_order("team") > layer_order("project"));
        assert!(layer_order("org") > layer_order("team"));
        assert!(layer_order("tenant") > layer_order("org"));
    }

    #[test]
    fn test_layer_hierarchy_promotion_invalid() {
        assert!(layer_order("agent") < layer_order("user"));
        assert!(layer_order("project") < layer_order("team"));
    }
}
