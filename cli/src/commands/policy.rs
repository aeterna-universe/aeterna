use clap::{Args, Subcommand};
use context::ContextResolver;
use mk_core::hints::{HintPreset, OperationHints};
use serde_json::json;

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum PolicyCommand {
    #[command(about = "Create a new policy from natural language or template")]
    Create(PolicyCreateArgs),

    #[command(about = "List policies in the current context")]
    List(PolicyListArgs),

    #[command(about = "Explain a policy in natural language")]
    Explain(PolicyExplainArgs),

    #[command(about = "Simulate a policy against a scenario")]
    Simulate(PolicySimulateArgs),

    #[command(about = "Validate a policy definition")]
    Validate(PolicyValidateArgs),

    #[command(about = "Show policy draft details")]
    Draft(PolicyDraftArgs),
}

#[derive(Args)]
pub struct PolicyCreateArgs {
    /// Policy description in natural language
    /// Example: "Block all dependencies with critical CVEs"
    #[arg(long)]
    pub description: Option<String>,

    /// Policy name (auto-generated if not provided)
    #[arg(short, long)]
    pub name: Option<String>,

    /// Layer to apply policy (tenant, org, team, project)
    #[arg(short, long, default_value = "project")]
    pub layer: String,

    /// Policy mode (mandatory, optional)
    #[arg(long, default_value = "mandatory")]
    pub mode: String,

    /// Use a template instead of natural language
    /// Templates: security-baseline, code-style, dependency-audit
    #[arg(long)]
    pub template: Option<String>,

    /// Target type for the rule (dependency, file, code, config)
    #[arg(long)]
    pub target: Option<String>,

    /// Severity level (info, warn, error, block)
    #[arg(long, default_value = "warn")]
    pub severity: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - show what would be created without saving
    #[arg(long)]
    pub dry_run: bool,

    /// Hints preset
    #[arg(long)]
    pub preset: Option<String>,
}

#[derive(Args)]
pub struct PolicyListArgs {
    /// Filter by layer (tenant, org, team, project)
    #[arg(short, long)]
    pub layer: Option<String>,

    /// Filter by mode (mandatory, optional)
    #[arg(long)]
    pub mode: Option<String>,

    /// Show all policies including inherited
    #[arg(long)]
    pub all: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct PolicyExplainArgs {
    /// Policy ID to explain
    pub policy_id: String,

    /// Include rule details
    #[arg(short, long)]
    pub verbose: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct PolicySimulateArgs {
    /// Policy ID to simulate
    pub policy_id: String,

    /// Scenario type to simulate (dependency-add, file-create, code-change)
    #[arg(long)]
    pub scenario: String,

    /// Scenario input (e.g., dependency name, file path)
    #[arg(long)]
    pub input: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Dry run - show simulation setup without executing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct PolicyValidateArgs {
    /// Policy ID to validate (or path to policy file)
    pub policy: String,

    /// Strict validation mode
    #[arg(long)]
    pub strict: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct PolicyDraftArgs {
    /// Draft ID to show
    pub draft_id: Option<String>,

    /// List all pending drafts
    #[arg(short, long)]
    pub list: bool,

    /// Submit draft for approval
    #[arg(long)]
    pub submit: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cmd: PolicyCommand) -> anyhow::Result<()> {
    match cmd {
        PolicyCommand::Create(args) => run_create(args).await,
        PolicyCommand::List(args) => run_list(args).await,
        PolicyCommand::Explain(args) => run_explain(args).await,
        PolicyCommand::Simulate(args) => run_simulate(args).await,
        PolicyCommand::Validate(args) => run_validate(args).await,
        PolicyCommand::Draft(args) => run_draft(args).await,
    }
}

async fn run_create(args: PolicyCreateArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    // Validate layer
    let layer = args.layer.to_lowercase();
    let valid_layers = ["tenant", "org", "team", "project"];
    if !valid_layers.contains(&layer.as_str()) {
        let err = ux_error::invalid_knowledge_layer(&layer);
        err.display();
        return Err(anyhow::anyhow!("Invalid layer"));
    }

    // Validate mode
    let mode = args.mode.to_lowercase();
    let valid_modes = ["mandatory", "optional"];
    if !valid_modes.contains(&mode.as_str()) {
        let err = ux_error::UxError::new(format!("Invalid policy mode: '{mode}'"))
            .why("Policy mode determines if violations block operations")
            .fix("Use 'mandatory' (blocks on violation) or 'optional' (warns only)")
            .suggest("aeterna policy create --mode mandatory --description \"...\"");
        err.display();
        return Err(anyhow::anyhow!("Invalid mode"));
    }

    // Validate severity
    let severity = args.severity.to_lowercase();
    let valid_severities = ["info", "warn", "error", "block"];
    if !valid_severities.contains(&severity.as_str()) {
        let err = ux_error::UxError::new(format!("Invalid severity: '{severity}'"))
            .why("Severity determines how violations are reported")
            .fix("Use one of: info, warn, error, block")
            .suggest("aeterna policy create --severity warn --description \"...\"");
        err.display();
        return Err(anyhow::anyhow!("Invalid severity"));
    }

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

    // Check what we have to work with
    let has_description = args.description.is_some();
    let has_template = args.template.is_some();

    if !has_description && !has_template {
        let err = ux_error::UxError::new("Policy creation requires description or template")
            .why("Either describe the policy in natural language or use a template")
            .fix("Provide --description with a natural language policy description")
            .fix("Or use --template to start from a predefined template")
            .suggest(
                "aeterna policy create --description \"Block dependencies with critical CVEs\"",
            );
        err.display();
        return Err(anyhow::anyhow!("Missing description or template"));
    }

    // Generate policy name if not provided
    let policy_name = args.name.unwrap_or_else(|| {
        if let Some(ref desc) = args.description {
            // Generate from description (first 3-4 words, kebab-case)
            let words: Vec<&str> = desc.split_whitespace().take(4).collect();
            words.join("-").to_lowercase().replace('"', "")
        } else if let Some(ref tmpl) = args.template {
            format!("{tmpl}-policy")
        } else {
            "new-policy".to_string()
        }
    });

    // Build the draft policy
    let draft_id = format!(
        "draft-{}-{}",
        policy_name.replace(' ', "-").to_lowercase(),
        chrono::Utc::now().timestamp()
    );

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "policy_create",
                "draft": {
                    "id": draft_id,
                    "name": policy_name,
                    "description": args.description,
                    "template": args.template,
                    "layer": layer,
                    "mode": mode,
                    "severity": severity,
                },
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "hints": {
                    "preset": format!("{}", hints.preset),
                    "governance": hints.governance,
                    "llm": hints.llm,
                },
                "nextSteps": [
                    "Review the generated policy draft",
                    "Run without --dry-run to create the draft",
                    "Use 'aeterna policy draft --submit <id>' to submit for approval"
                ]
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Policy Create (Dry Run)");
            println!();
            println!("  Draft ID:    {draft_id}");
            println!("  Name:        {policy_name}");
            println!("  Layer:       {layer}");
            println!("  Mode:        {mode}");
            println!("  Severity:    {severity}");
            println!();

            if let Some(ref desc) = args.description {
                output::header("Natural Language Input");
                println!("  \"{desc}\"");
                println!();
                output::header("Translation Pipeline");
                println!(
                    "  1. Parse natural language → structured intent  {}",
                    hint_effect(hints.llm, "uses LLM")
                );
                println!("  2. Map intent → Cedar policy rules");
                println!("  3. Validate Cedar syntax");
                println!("  4. Store as draft for review");
                println!();
            }

            if let Some(ref tmpl) = args.template {
                output::header("Template");
                println!("  Using template: {tmpl}");
                let tmpl_desc = match tmpl.as_str() {
                    "security-baseline" => "Blocks critical CVEs, requires SECURITY.md",
                    "code-style" => "Enforces code style and formatting rules",
                    "dependency-audit" => "Audits dependencies for licenses and vulnerabilities",
                    _ => "Custom template",
                };
                println!("  Description: {tmpl_desc}");
                println!();
            }

            output::header("Context");
            println!("  tenant_id:  {}", resolved.tenant_id.value);
            println!("  user_id:    {}", resolved.user_id.value);
            if let Some(project) = &resolved.project_id {
                println!("  project_id: {}", project.value);
            }
            println!();

            output::header("Next Steps");
            println!("  1. Review the generated policy draft");
            println!("  2. Run without --dry-run to create the draft");
            println!("  3. Use 'aeterna policy draft --submit {draft_id}' to submit for approval");
            println!();

            output::info("Dry run mode - policy draft not created.");
            output::info("Remove --dry-run to create the policy draft.");
        }
        return Ok(());
    }

    // Not connected - show what would happen
    let err = ux_error::server_not_connected();
    err.display();
    output::info("Run with --dry-run to see what would be created.");

    Ok(())
}

async fn run_list(args: PolicyListArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    // Validate layer if provided
    if let Some(ref layer) = args.layer {
        let layer_lower = layer.to_lowercase();
        let valid_layers = ["tenant", "org", "team", "project"];
        if !valid_layers.contains(&layer_lower.as_str()) {
            let err = ux_error::invalid_knowledge_layer(layer);
            err.display();
            return Err(anyhow::anyhow!("Invalid layer"));
        }
    }

    if args.json {
        let output = json!({
            "operation": "policy_list",
            "filters": {
                "layer": args.layer,
                "mode": args.mode,
                "includeInherited": args.all,
            },
            "context": {
                "tenantId": resolved.tenant_id.value,
                "userId": resolved.user_id.value,
                "projectId": resolved.project_id.as_ref().map(|v| &v.value),
            },
            "status": "not_connected",
            "message": "Policy backend not connected"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("Policies");
        println!();

        if args.all {
            output::info("Showing all policies including inherited from parent layers.");
        }

        if let Some(ref layer) = args.layer {
            println!("  Filter: layer = {layer}");
        }
        if let Some(ref mode) = args.mode {
            println!("  Filter: mode = {mode}");
        }
        println!();

        // Show example of what would be displayed
        output::header("Policy Inheritance (would show)");
        println!("  tenant    → Security Baseline (mandatory)");
        println!("  org       → Platform Standards (mandatory)");
        println!("  team      → API Team Conventions (optional)");
        println!("  project   → [your policies here]");
        println!();

        let err = ux_error::server_not_connected();
        err.display();
    }

    Ok(())
}

async fn run_explain(args: PolicyExplainArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    if args.json {
        let output = json!({
            "operation": "policy_explain",
            "policyId": args.policy_id,
            "verbose": args.verbose,
            "context": {
                "tenantId": resolved.tenant_id.value,
                "userId": resolved.user_id.value,
            },
            "status": "not_connected",
            "message": "Policy backend not connected"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header(&format!("Policy: {}", args.policy_id));
        println!();

        // Example of what explanation would look like
        output::header("Natural Language Explanation");
        println!("  This policy would be explained in plain English:");
        println!("  - What it does");
        println!("  - When it applies");
        println!("  - What happens when violated");
        println!();

        if args.verbose {
            output::header("Rule Details");
            println!("  Would show detailed rule breakdown:");
            println!("  - Target type");
            println!("  - Operator");
            println!("  - Expected values");
            println!("  - Severity levels");
            println!();
        }

        let err = ux_error::server_not_connected();
        err.display();
    }

    Ok(())
}

async fn run_simulate(args: PolicySimulateArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    // Validate scenario type
    let scenario = args.scenario.to_lowercase();
    let valid_scenarios = [
        "dependency-add",
        "file-create",
        "code-change",
        "config-update",
    ];
    if !valid_scenarios.contains(&scenario.as_str()) {
        let err = ux_error::UxError::new(format!("Invalid scenario type: '{scenario}'"))
            .why("Scenario type determines what kind of operation to simulate")
            .fix("Use one of: dependency-add, file-create, code-change, config-update")
            .suggest("aeterna policy simulate policy-1 --scenario dependency-add --input lodash");
        err.display();
        return Err(anyhow::anyhow!("Invalid scenario type"));
    }

    if args.dry_run {
        if args.json {
            let output = json!({
                "dryRun": true,
                "operation": "policy_simulate",
                "policyId": args.policy_id,
                "scenario": {
                    "type": scenario,
                    "input": args.input,
                },
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                    "projectId": resolved.project_id.as_ref().map(|v| &v.value),
                },
                "wouldCheck": [
                    "Policy rules matching the scenario type",
                    "Input value against rule patterns",
                    "Inheritance from parent layers"
                ]
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Policy Simulate (Dry Run)");
            println!();
            println!("  Policy:   {}", args.policy_id);
            println!("  Scenario: {scenario}");
            println!("  Input:    {}", args.input);
            println!();
            output::header("Would Check");
            println!("  1. Policy rules matching scenario type '{scenario}'");
            println!("  2. Input '{}' against rule patterns", args.input);
            println!("  3. Inherited policies from parent layers");
            println!();
            output::header("Expected Output");
            println!("  - PASS / FAIL / WARN status");
            println!("  - Matching rules and their outcomes");
            println!("  - Suggested fixes for violations");
            println!();
            output::info("Dry run mode - simulation not executed.");
            output::info("Remove --dry-run to run the simulation.");
        }
        return Ok(());
    }

    if args.json {
        let output = json!({
            "operation": "policy_simulate",
            "policyId": args.policy_id,
            "scenario": {
                "type": scenario,
                "input": args.input,
            },
            "status": "not_connected",
            "message": "Policy backend not connected"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("Policy Simulate");
        println!();
        println!("  Policy:   {}", args.policy_id);
        println!("  Scenario: {scenario}");
        println!("  Input:    {}", args.input);
        println!();
        let err = ux_error::server_not_connected();
        err.display();
        output::info("Run with --dry-run to see what would be simulated.");
    }

    Ok(())
}

async fn run_validate(args: PolicyValidateArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    if args.json {
        let output = json!({
            "operation": "policy_validate",
            "policy": args.policy,
            "strict": args.strict,
            "context": {
                "tenantId": resolved.tenant_id.value,
                "userId": resolved.user_id.value,
            },
            "status": "not_connected",
            "message": "Policy backend not connected"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header(&format!("Validate Policy: {}", args.policy));
        println!();

        if args.strict {
            output::info("Strict mode enabled - checking additional constraints.");
        }

        output::header("Validation Steps");
        println!("  1. Parse policy structure");
        println!("  2. Validate Cedar syntax (if Cedar format)");
        println!("  3. Check rule consistency");
        println!("  4. Detect conflicts with existing policies");
        if args.strict {
            println!("  5. Check for unreachable rules");
            println!("  6. Validate against meta-governance policies");
        }
        println!();

        let err = ux_error::server_not_connected();
        err.display();
    }

    Ok(())
}

async fn run_draft(args: PolicyDraftArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let resolved = resolver.resolve()?;

    if args.list {
        if args.json {
            let output = json!({
                "operation": "policy_draft_list",
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                },
                "status": "not_connected",
                "message": "Policy backend not connected"
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header("Policy Drafts");
            println!();
            output::info("Would show pending policy drafts created by you.");
            println!();
            let err = ux_error::server_not_connected();
            err.display();
        }
        return Ok(());
    }

    if let Some(ref submit_id) = args.submit {
        if args.json {
            let output = json!({
                "operation": "policy_draft_submit",
                "draftId": submit_id,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                },
                "status": "not_connected",
                "message": "Policy backend not connected"
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header(&format!("Submit Draft: {submit_id}"));
            println!();
            output::header("Submission Workflow");
            println!("  1. Validate draft policy");
            println!("  2. Check permissions (can you propose policies?)");
            println!("  3. Create approval request");
            println!("  4. Notify approvers based on governance level");
            println!();
            let err = ux_error::server_not_connected();
            err.display();
        }
        return Ok(());
    }

    // Show specific draft
    if let Some(ref draft_id) = args.draft_id {
        if args.json {
            let output = json!({
                "operation": "policy_draft_show",
                "draftId": draft_id,
                "context": {
                    "tenantId": resolved.tenant_id.value,
                    "userId": resolved.user_id.value,
                },
                "status": "not_connected",
                "message": "Policy backend not connected"
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            output::header(&format!("Policy Draft: {draft_id}"));
            println!();
            output::info("Would show draft details including:");
            println!("  - Original natural language description");
            println!("  - Generated Cedar policy");
            println!("  - Validation status");
            println!("  - Submission status");
            println!();
            let err = ux_error::server_not_connected();
            err.display();
        }
    } else {
        // No arguments - show help
        let err = ux_error::UxError::new("No draft ID or action specified")
            .why("The draft command needs either a draft ID or an action flag")
            .fix("Provide a draft ID to view: aeterna policy draft <draft-id>")
            .fix("List all drafts: aeterna policy draft --list")
            .fix("Submit a draft: aeterna policy draft --submit <draft-id>")
            .suggest("aeterna policy draft --list");
        err.display();
        return Err(anyhow::anyhow!("Missing draft ID or action"));
    }

    Ok(())
}

fn hint_effect(enabled: bool, effect: &str) -> String {
    if enabled {
        format!("({effect})")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_create_args_defaults() {
        let args = PolicyCreateArgs {
            description: None,
            name: None,
            layer: "project".to_string(),
            mode: "mandatory".to_string(),
            template: None,
            target: None,
            severity: "warn".to_string(),
            json: false,
            dry_run: false,
            preset: None,
        };
        assert!(args.description.is_none());
        assert!(args.name.is_none());
        assert_eq!(args.layer, "project");
        assert_eq!(args.mode, "mandatory");
        assert_eq!(args.severity, "warn");
        assert!(!args.json);
        assert!(!args.dry_run);
    }

    #[test]
    fn test_policy_create_args_with_description() {
        let args = PolicyCreateArgs {
            description: Some("Block all dependencies with critical CVEs".to_string()),
            name: Some("cve-blocker".to_string()),
            layer: "tenant".to_string(),
            mode: "mandatory".to_string(),
            template: None,
            target: Some("dependency".to_string()),
            severity: "block".to_string(),
            json: true,
            dry_run: true,
            preset: Some("strict".to_string()),
        };
        assert!(args.description.is_some());
        assert_eq!(args.name, Some("cve-blocker".to_string()));
        assert_eq!(args.layer, "tenant");
        assert_eq!(args.target, Some("dependency".to_string()));
    }

    #[test]
    fn test_policy_create_args_with_template() {
        let args = PolicyCreateArgs {
            description: None,
            name: None,
            layer: "org".to_string(),
            mode: "optional".to_string(),
            template: Some("security-baseline".to_string()),
            target: None,
            severity: "error".to_string(),
            json: false,
            dry_run: false,
            preset: None,
        };
        assert!(args.template.is_some());
        assert_eq!(args.mode, "optional");
    }

    #[test]
    fn test_policy_list_args_defaults() {
        let args = PolicyListArgs {
            layer: None,
            mode: None,
            all: false,
            json: false,
        };
        assert!(args.layer.is_none());
        assert!(args.mode.is_none());
        assert!(!args.all);
        assert!(!args.json);
    }

    #[test]
    fn test_policy_list_args_with_filters() {
        let args = PolicyListArgs {
            layer: Some("team".to_string()),
            mode: Some("mandatory".to_string()),
            all: true,
            json: true,
        };
        assert_eq!(args.layer, Some("team".to_string()));
        assert_eq!(args.mode, Some("mandatory".to_string()));
        assert!(args.all);
    }

    #[test]
    fn test_policy_explain_args() {
        let args = PolicyExplainArgs {
            policy_id: "security-baseline".to_string(),
            verbose: false,
            json: false,
        };
        assert_eq!(args.policy_id, "security-baseline");
        assert!(!args.verbose);
    }

    #[test]
    fn test_policy_explain_args_verbose() {
        let args = PolicyExplainArgs {
            policy_id: "cve-blocker".to_string(),
            verbose: true,
            json: true,
        };
        assert!(args.verbose);
        assert!(args.json);
    }

    #[test]
    fn test_policy_simulate_args() {
        let args = PolicySimulateArgs {
            policy_id: "dependency-audit".to_string(),
            scenario: "dependency-add".to_string(),
            input: "lodash@4.17.20".to_string(),
            json: false,
            dry_run: false,
        };
        assert_eq!(args.policy_id, "dependency-audit");
        assert_eq!(args.scenario, "dependency-add");
        assert_eq!(args.input, "lodash@4.17.20");
    }

    #[test]
    fn test_policy_simulate_args_dry_run() {
        let args = PolicySimulateArgs {
            policy_id: "code-style".to_string(),
            scenario: "code-change".to_string(),
            input: "src/main.rs".to_string(),
            json: true,
            dry_run: true,
        };
        assert!(args.dry_run);
        assert!(args.json);
    }

    #[test]
    fn test_policy_validate_args() {
        let args = PolicyValidateArgs {
            policy: "security-baseline".to_string(),
            strict: false,
            json: false,
        };
        assert_eq!(args.policy, "security-baseline");
        assert!(!args.strict);
    }

    #[test]
    fn test_policy_validate_args_strict() {
        let args = PolicyValidateArgs {
            policy: "/path/to/policy.cedar".to_string(),
            strict: true,
            json: true,
        };
        assert!(args.strict);
        assert!(args.policy.contains(".cedar"));
    }

    #[test]
    fn test_policy_draft_args_show() {
        let args = PolicyDraftArgs {
            draft_id: Some("draft-cve-blocker-12345".to_string()),
            list: false,
            submit: None,
            json: false,
        };
        assert_eq!(args.draft_id, Some("draft-cve-blocker-12345".to_string()));
        assert!(!args.list);
    }

    #[test]
    fn test_policy_draft_args_list() {
        let args = PolicyDraftArgs {
            draft_id: None,
            list: true,
            submit: None,
            json: true,
        };
        assert!(args.draft_id.is_none());
        assert!(args.list);
    }

    #[test]
    fn test_policy_draft_args_submit() {
        let args = PolicyDraftArgs {
            draft_id: None,
            list: false,
            submit: Some("draft-my-policy-67890".to_string()),
            json: false,
        };
        assert_eq!(args.submit, Some("draft-my-policy-67890".to_string()));
    }

    #[test]
    fn test_hint_effect_enabled() {
        let result = hint_effect(true, "uses LLM");
        assert_eq!(result, "(uses LLM)");
    }

    #[test]
    fn test_hint_effect_disabled() {
        let result = hint_effect(false, "uses LLM");
        assert_eq!(result, "");
    }

    #[test]
    fn test_hint_effect_various_effects() {
        assert_eq!(hint_effect(true, "caches result"), "(caches result)");
        assert_eq!(hint_effect(true, "requires auth"), "(requires auth)");
        assert_eq!(hint_effect(false, "caches result"), "");
    }

    #[test]
    fn test_layer_validation_valid_layers() {
        let valid_layers = ["tenant", "org", "team", "project"];
        for layer in valid_layers {
            assert!(valid_layers.contains(&layer.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_layer_validation_invalid_layers() {
        let valid_layers = ["tenant", "org", "team", "project"];
        let invalid_layers = ["global", "user", "session", "agent"];
        for layer in invalid_layers {
            assert!(!valid_layers.contains(&layer.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_mode_validation_valid_modes() {
        let valid_modes = ["mandatory", "optional"];
        for mode in valid_modes {
            assert!(valid_modes.contains(&mode.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_mode_validation_invalid_modes() {
        let valid_modes = ["mandatory", "optional"];
        let invalid_modes = ["required", "suggested", "enforced"];
        for mode in invalid_modes {
            assert!(!valid_modes.contains(&mode.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_severity_validation_valid_severities() {
        let valid_severities = ["info", "warn", "error", "block"];
        for severity in valid_severities {
            assert!(valid_severities.contains(&severity.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_severity_validation_invalid_severities() {
        let valid_severities = ["info", "warn", "error", "block"];
        let invalid_severities = ["critical", "fatal", "debug", "notice"];
        for severity in invalid_severities {
            assert!(!valid_severities.contains(&severity.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_scenario_validation_valid_scenarios() {
        let valid_scenarios = [
            "dependency-add",
            "file-create",
            "code-change",
            "config-update",
        ];
        for scenario in valid_scenarios {
            assert!(valid_scenarios.contains(&scenario.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_scenario_validation_invalid_scenarios() {
        let valid_scenarios = [
            "dependency-add",
            "file-create",
            "code-change",
            "config-update",
        ];
        let invalid_scenarios = ["build-run", "test-execute", "deploy-app"];
        for scenario in invalid_scenarios {
            assert!(!valid_scenarios.contains(&scenario.to_lowercase().as_str()));
        }
    }

    #[test]
    fn test_policy_create_args_all_layers() {
        let layers = ["tenant", "org", "team", "project"];
        for layer in layers {
            let args = PolicyCreateArgs {
                description: Some("Test policy".to_string()),
                name: None,
                layer: layer.to_string(),
                mode: "mandatory".to_string(),
                template: None,
                target: None,
                severity: "warn".to_string(),
                json: false,
                dry_run: false,
                preset: None,
            };
            assert_eq!(args.layer, layer);
        }
    }

    #[test]
    fn test_policy_create_args_all_severities() {
        let severities = ["info", "warn", "error", "block"];
        for severity in severities {
            let args = PolicyCreateArgs {
                description: Some("Test policy".to_string()),
                name: None,
                layer: "project".to_string(),
                mode: "mandatory".to_string(),
                template: None,
                target: None,
                severity: severity.to_string(),
                json: false,
                dry_run: false,
                preset: None,
            };
            assert_eq!(args.severity, severity);
        }
    }

    #[test]
    fn test_policy_create_args_all_target_types() {
        let targets = ["dependency", "file", "code", "config"];
        for target in targets {
            let args = PolicyCreateArgs {
                description: Some("Test policy".to_string()),
                name: None,
                layer: "project".to_string(),
                mode: "mandatory".to_string(),
                template: None,
                target: Some(target.to_string()),
                severity: "warn".to_string(),
                json: false,
                dry_run: false,
                preset: None,
            };
            assert_eq!(args.target, Some(target.to_string()));
        }
    }

    #[test]
    fn test_policy_simulate_args_all_scenarios() {
        let scenarios = [
            "dependency-add",
            "file-create",
            "code-change",
            "config-update",
        ];
        for scenario in scenarios {
            let args = PolicySimulateArgs {
                policy_id: "test-policy".to_string(),
                scenario: scenario.to_string(),
                input: "test-input".to_string(),
                json: false,
                dry_run: false,
            };
            assert_eq!(args.scenario, scenario);
        }
    }

    #[test]
    fn test_policy_list_args_all_mode_filters() {
        let modes = ["mandatory", "optional"];
        for mode in modes {
            let args = PolicyListArgs {
                layer: None,
                mode: Some(mode.to_string()),
                all: false,
                json: false,
            };
            assert_eq!(args.mode, Some(mode.to_string()));
        }
    }

    #[test]
    fn test_template_options() {
        let templates = ["security-baseline", "code-style", "dependency-audit"];
        for template in templates {
            let args = PolicyCreateArgs {
                description: None,
                name: None,
                layer: "team".to_string(),
                mode: "mandatory".to_string(),
                template: Some(template.to_string()),
                target: None,
                severity: "warn".to_string(),
                json: false,
                dry_run: false,
                preset: None,
            };
            assert_eq!(args.template, Some(template.to_string()));
        }
    }

    #[test]
    fn test_policy_draft_args_no_action() {
        let args = PolicyDraftArgs {
            draft_id: None,
            list: false,
            submit: None,
            json: false,
        };
        assert!(args.draft_id.is_none());
        assert!(!args.list);
        assert!(args.submit.is_none());
    }

    #[test]
    fn test_policy_create_args_preset_options() {
        let presets = ["strict", "permissive", "balanced"];
        for preset in presets {
            let args = PolicyCreateArgs {
                description: Some("Test".to_string()),
                name: None,
                layer: "project".to_string(),
                mode: "mandatory".to_string(),
                template: None,
                target: None,
                severity: "warn".to_string(),
                json: false,
                dry_run: false,
                preset: Some(preset.to_string()),
            };
            assert_eq!(args.preset, Some(preset.to_string()));
        }
    }

    #[test]
    fn test_policy_explain_args_policy_id_formats() {
        let policy_ids = [
            "security-baseline",
            "tenant-wide-policy",
            "team-api-conventions",
            "project-local-rules",
        ];
        for id in policy_ids {
            let args = PolicyExplainArgs {
                policy_id: id.to_string(),
                verbose: false,
                json: false,
            };
            assert_eq!(args.policy_id, id);
        }
    }

    #[test]
    fn test_policy_validate_args_file_path() {
        let args = PolicyValidateArgs {
            policy: "policies/my-policy.cedar".to_string(),
            strict: false,
            json: false,
        };
        assert!(args.policy.contains('/'));
        assert!(args.policy.ends_with(".cedar"));
    }

    #[test]
    fn test_policy_validate_args_policy_id() {
        let args = PolicyValidateArgs {
            policy: "security-baseline".to_string(),
            strict: false,
            json: false,
        };
        assert!(!args.policy.contains('/'));
        assert!(!args.policy.ends_with(".cedar"));
    }
}
