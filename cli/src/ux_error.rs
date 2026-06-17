use colored::Colorize;

#[derive(Debug)]
pub struct UxError {
    pub what: String,
    pub why: Option<String>,
    pub how_to_fix: Vec<String>,
    pub suggested_command: Option<String>,
}

impl UxError {
    pub fn new(what: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            why: None,
            how_to_fix: Vec::new(),
            suggested_command: None,
        }
    }

    pub fn why(mut self, reason: impl Into<String>) -> Self {
        self.why = Some(reason.into());
        self
    }

    pub fn fix(mut self, suggestion: impl Into<String>) -> Self {
        self.how_to_fix.push(suggestion.into());
        self
    }

    pub fn suggest(mut self, cmd: impl Into<String>) -> Self {
        self.suggested_command = Some(cmd.into());
        self
    }

    pub fn display(&self) {
        eprintln!();
        eprintln!("{} {}", "error:".red().bold(), self.what.white().bold());

        if let Some(why) = &self.why {
            eprintln!("       {}", why.dimmed());
        }

        if !self.how_to_fix.is_empty() {
            eprintln!();
            eprintln!("{}", "How to fix:".yellow().bold());
            for (i, fix) in self.how_to_fix.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, fix);
            }
        }

        if let Some(cmd) = &self.suggested_command {
            eprintln!();
            eprintln!("{}", "Try this:".green().bold());
            eprintln!("  $ {}", cmd.cyan());
        }
        eprintln!();
    }
}

impl std::fmt::Display for UxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.what)
    }
}

impl std::error::Error for UxError {}

#[must_use]
pub fn context_not_found(path: &str) -> UxError {
    UxError::new(format!("No Aeterna context found at '{path}'"))
        .why("Expected .aeterna/context.toml in this or parent directories")
        .fix("Initialize Aeterna in this directory")
        .fix("Or navigate to a directory with an existing Aeterna project")
        .suggest("aeterna init")
}

#[must_use]
pub fn server_not_connected() -> UxError {
    UxError::new("Cannot connect to Aeterna server")
        .why("The memory/knowledge backend is not running or unreachable")
        .fix("Start the Aeterna server")
        .fix("Check your network connection")
        .fix("Verify the active profile, AETERNA_SERVER_URL, or profile config files")
        .suggest("aeterna auth status")
}

#[must_use]
pub fn invalid_layer(layer: &str, valid: &[&str]) -> UxError {
    UxError::new(format!("Invalid layer: '{layer}'"))
        .why(format!("Valid layers are: {}", valid.join(", ")))
        .fix("Use one of the valid layer names")
        .suggest(format!("aeterna memory list --layer {}", valid[0]))
}

#[must_use]
pub fn invalid_preset(preset: &str) -> UxError {
    UxError::new(format!("Unknown hints preset: '{preset}'"))
        .why("Presets control which features are enabled for an operation")
        .fix("Use one of: minimal, fast, standard, full, offline, agent")
        .suggest("aeterna hints list")
}

#[must_use]
pub fn missing_required_field(field: &str, context: &str) -> UxError {
    UxError::new(format!("Missing required field: '{field}'"))
        .why(format!("This field is required for {context}"))
        .fix(format!("Provide the {field} value"))
}

#[must_use]
pub fn permission_denied(resource: &str) -> UxError {
    UxError::new(format!("Permission denied: {resource}"))
        .why("Your user or agent doesn't have access to this resource")
        .fix("Check your role and permissions")
        .fix("Contact your administrator for access")
        .suggest("aeterna status")
}

#[must_use]
pub fn rate_limited(retry_after: u64) -> UxError {
    UxError::new("Rate limit exceeded")
        .why(format!(
            "Too many requests. Retry after {retry_after} seconds"
        ))
        .fix("Wait before retrying")
        .fix("Consider using the 'minimal' or 'offline' preset to reduce API calls")
        .suggest("aeterna hints explain minimal")
}

#[must_use]
pub fn memory_not_found(id: &str) -> UxError {
    UxError::new(format!("Memory not found: {id}"))
        .why("The specified memory ID doesn't exist or has been deleted")
        .fix("Verify the memory ID is correct")
        .fix("Search for memories to find the right ID")
        .suggest("aeterna memory search <query>")
}

#[must_use]
pub fn knowledge_not_found(path: &str, layer: &str) -> UxError {
    UxError::new(format!("Knowledge not found: {path} in {layer} layer"))
        .why("The specified knowledge entry doesn't exist in this layer")
        .fix("Check the path is correct")
        .fix("Try searching across all layers")
        .suggest(format!("aeterna knowledge search \"{path}\""))
}

#[must_use]
pub fn policy_violation(policy_id: &str, details: &str) -> UxError {
    UxError::new(format!("Policy violation: {policy_id}"))
        .why(details.to_string())
        .fix("Review the policy requirements")
        .fix("Request an exception if needed")
        .suggest(format!(
            "aeterna knowledge get policies/{policy_id}.md --layer tenant"
        ))
}

#[must_use]
pub fn config_error(message: &str) -> UxError {
    UxError::new(format!("Configuration error: {message}"))
        .why("The active profile or config file may be invalid or missing required fields")
        .fix("Check .aeterna/config.toml and ~/.config/aeterna/config.toml")
        .fix("Run 'aeterna config show' to inspect effective settings")
        .suggest("aeterna config validate")
}

#[must_use]
pub fn git_error(operation: &str, reason: &str) -> UxError {
    UxError::new(format!("Git error during {operation}: {reason}"))
        .why("The knowledge repository uses Git for version control")
        .fix("Ensure you're in a Git repository")
        .fix("Check your Git configuration")
        .suggest("git status")
}

#[must_use]
pub fn timeout_error(operation: &str, timeout_ms: u64) -> UxError {
    UxError::new(format!("Operation timed out: {operation}"))
        .why(format!("Operation took longer than {timeout_ms}ms"))
        .fix("Check server/network connectivity")
        .fix("Try with fewer results or simpler query")
        .fix("Use 'offline' preset to skip network calls")
        .suggest("aeterna hints explain offline")
}

#[must_use]
pub fn invalid_feedback_type(feedback_type: &str) -> UxError {
    UxError::new(format!("Invalid feedback type: '{feedback_type}'"))
        .why("Feedback type must describe how the memory was useful or not")
        .fix("Use one of: helpful, irrelevant, outdated, inaccurate, duplicate")
        .suggest("aeterna memory feedback <id> --layer project --feedback-type helpful --score 0.8")
}

#[must_use]
pub fn invalid_score(score: f32) -> UxError {
    UxError::new(format!("Invalid score: {score}"))
        .why("Score must be between -1.0 (completely wrong) and 1.0 (very helpful)")
        .fix("Use a value in the range -1.0 to 1.0")
        .suggest("aeterna memory feedback <id> --layer project --feedback-type helpful --score 0.8")
}

#[must_use]
pub fn invalid_metadata_json(error: &str) -> UxError {
    UxError::new("Invalid metadata JSON")
        .why(format!("Parse error: {error}"))
        .fix("Provide valid JSON for the --metadata flag")
        .fix("Example: --metadata '{\"key\": \"value\"}'")
}

#[must_use]
pub fn invalid_knowledge_layer(layer: &str) -> UxError {
    UxError::new(format!("Invalid knowledge layer: '{layer}'"))
        .why("Knowledge layers represent organizational hierarchy")
        .fix("Use one of: tenant, org, team, project")
        .suggest("aeterna knowledge list --layer project")
}

#[must_use]
pub fn promotion_direction_invalid(from_layer: &str, to_layer: &str) -> UxError {
    UxError::new(format!(
        "Cannot promote from '{from_layer}' to '{to_layer}'"
    ))
    .why("Promotion must be to a broader (higher) layer in the hierarchy")
    .fix("Layer hierarchy: agent < user < session < project < team < org < tenant")
    .fix("Choose a target layer that is broader than the source")
    .suggest(format!(
        "aeterna memory promote <id> --from-layer {from_layer} --to-layer team"
    ))
}

#[must_use]
pub fn invalid_knowledge_type(knowledge_type: &str, valid: &[&str]) -> UxError {
    UxError::new(format!("Invalid knowledge type: '{knowledge_type}'"))
        .why(format!("Valid types are: {}", valid.join(", ")))
        .fix("Use one of the valid knowledge type names")
        .fix("Or omit --type to let the system auto-detect from your description")
        .suggest("aeterna knowledge propose \"We decided to use PostgreSQL\" --type adr")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ux_error_new() {
        let err = UxError::new("test error");
        assert_eq!(err.what, "test error");
        assert!(err.why.is_none());
        assert!(err.how_to_fix.is_empty());
        assert!(err.suggested_command.is_none());
    }

    #[test]
    fn test_ux_error_builder_chain() {
        let err = UxError::new("test error")
            .why("because reasons")
            .fix("try this")
            .fix("or this")
            .suggest("run command");

        assert_eq!(err.what, "test error");
        assert_eq!(err.why, Some("because reasons".to_string()));
        assert_eq!(err.how_to_fix.len(), 2);
        assert_eq!(err.how_to_fix[0], "try this");
        assert_eq!(err.how_to_fix[1], "or this");
        assert_eq!(err.suggested_command, Some("run command".to_string()));
    }

    #[test]
    fn test_ux_error_display() {
        let err = UxError::new("test error");
        assert_eq!(format!("{err}"), "test error");
    }

    #[test]
    fn test_context_not_found() {
        let err = context_not_found("/some/path");
        assert!(err.what.contains("/some/path"));
        assert!(err.why.is_some());
        assert!(!err.how_to_fix.is_empty());
        assert_eq!(err.suggested_command, Some("aeterna init".to_string()));
    }

    #[test]
    fn test_server_not_connected() {
        let err = server_not_connected();
        assert!(err.what.contains("Cannot connect"));
        assert!(err.why.is_some());
        assert_eq!(err.how_to_fix.len(), 3);
        assert_eq!(
            err.suggested_command,
            Some("aeterna auth status".to_string())
        );
    }

    #[test]
    fn test_invalid_layer() {
        let valid = ["agent", "user", "project"];
        let err = invalid_layer("invalid", &valid);
        assert!(err.what.contains("invalid"));
        assert!(err.why.as_ref().unwrap().contains("agent"));
        assert!(!err.how_to_fix.is_empty());
        assert!(err.suggested_command.is_some());
    }

    #[test]
    fn test_invalid_preset() {
        let err = invalid_preset("badpreset");
        assert!(err.what.contains("badpreset"));
        assert!(err.why.is_some());
        assert!(!err.how_to_fix.is_empty());
    }

    #[test]
    fn test_missing_required_field() {
        let err = missing_required_field("user_id", "authentication");
        assert!(err.what.contains("user_id"));
        assert!(err.why.as_ref().unwrap().contains("authentication"));
    }

    #[test]
    fn test_permission_denied() {
        let err = permission_denied("knowledge/secrets");
        assert!(err.what.contains("knowledge/secrets"));
        assert!(err.why.is_some());
        assert_eq!(err.how_to_fix.len(), 2);
    }

    #[test]
    fn test_rate_limited() {
        let err = rate_limited(60);
        assert!(err.what.contains("Rate limit"));
        assert!(err.why.as_ref().unwrap().contains("60"));
    }

    #[test]
    fn test_memory_not_found() {
        let err = memory_not_found("mem-123");
        assert!(err.what.contains("mem-123"));
        assert!(err.why.is_some());
        assert_eq!(err.how_to_fix.len(), 2);
    }

    #[test]
    fn test_knowledge_not_found() {
        let err = knowledge_not_found("adrs/001.md", "project");
        assert!(err.what.contains("adrs/001.md"));
        assert!(err.what.contains("project"));
    }

    #[test]
    fn test_policy_violation() {
        let err = policy_violation("sec-001", "Dependency blocked");
        assert!(err.what.contains("sec-001"));
        assert_eq!(err.why, Some("Dependency blocked".to_string()));
    }

    #[test]
    fn test_config_error() {
        let err = config_error("invalid toml syntax");
        assert!(err.what.contains("invalid toml syntax"));
        assert_eq!(
            err.suggested_command,
            Some("aeterna config validate".to_string())
        );
    }

    #[test]
    fn test_git_error() {
        let err = git_error("commit", "not a git repository");
        assert!(err.what.contains("commit"));
        assert!(err.what.contains("not a git repository"));
    }

    #[test]
    fn test_timeout_error() {
        let err = timeout_error("search", 5000);
        assert!(err.what.contains("search"));
        assert!(err.why.as_ref().unwrap().contains("5000"));
    }

    #[test]
    fn test_invalid_feedback_type() {
        let err = invalid_feedback_type("unknown");
        assert!(err.what.contains("unknown"));
        assert!(err.why.as_ref().unwrap().contains("useful"));
    }

    #[test]
    fn test_invalid_score_below_range() {
        let err = invalid_score(-1.5);
        assert!(err.what.contains("-1.5"));
        assert!(err.why.as_ref().unwrap().contains("-1.0"));
    }

    #[test]
    fn test_invalid_score_above_range() {
        let err = invalid_score(1.5);
        assert!(err.what.contains("1.5"));
    }

    #[test]
    fn test_invalid_metadata_json() {
        let err = invalid_metadata_json("unexpected token");
        assert!(err.what.contains("Invalid metadata JSON"));
        assert!(err.why.as_ref().unwrap().contains("unexpected token"));
    }

    #[test]
    fn test_invalid_knowledge_layer() {
        let err = invalid_knowledge_layer("invalid");
        assert!(err.what.contains("invalid"));
        assert!(err.why.as_ref().unwrap().contains("organizational"));
    }

    #[test]
    fn test_promotion_direction_invalid() {
        let err = promotion_direction_invalid("team", "user");
        assert!(err.what.contains("team"));
        assert!(err.what.contains("user"));
        assert!(err.why.as_ref().unwrap().contains("broader"));
    }

    #[test]
    fn test_invalid_knowledge_type() {
        let valid = ["adr", "pattern", "policy"];
        let err = invalid_knowledge_type("unknown", &valid);
        assert!(err.what.contains("unknown"));
        assert!(err.why.as_ref().unwrap().contains("adr"));
    }
}
