# Governance System Troubleshooting Guide

This guide provides comprehensive troubleshooting steps for common governance issues in the Aeterna Memory-Knowledge System.

## Table of Contents

1. [Common Error Messages](#common-error-messages)
2. [Policy Validation Issues](#policy-validation-issues)
3. [Drift Detection Problems](#drift-detection-problems)
4. [Storage Connection Issues](#storage-connection-issues)
5. [Permit.io Integration](#permitio-integration)
6. [Performance Troubleshooting](#performance-troubleshooting)
7. [Debugging Checklist](#debugging-checklist)
8. [FAQ](#faq)

---

## Common Error Messages

### "No mandatory policies detected for this project layer"

**Cause**: This warning occurs when governance evaluation completes but finds no policies marked as `PolicyMode::Mandatory` for the target layer and its hierarchy.

**Why it happens**:
- Project only has optional policies
- Mandatory policies exist at higher layers but don't apply to target layer
- Policy inheritance hierarchy is broken

**Solutions**:
```rust
// Add a mandatory policy at tenant level
let company_policy = Policy {
    id: "baseline-security".to_string(),
    name: "Baseline Security".to_string(),
    layer: KnowledgeLayer::Tenant,
    mode: PolicyMode::Mandatory,  // This is required
    merge_strategy: RuleMergeStrategy::Merge,
    rules: vec![
        PolicyRule {
            id: "no-secrets".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("SECRET_.*"),
            severity: ConstraintSeverity::Block,
            message: "No secrets allowed in code".to_string(),
        }
    ],
    metadata: HashMap::new(),
};

engine.add_policy(company_policy);
```

**Debugging steps**:
1. Enable verbose logging: `RUST_LOG=debug`
2. Check policy hierarchy: ensure mandatory policies exist at Tenant, Org, or Team layers
3. Verify policy IDs are unique across layers
4. Check merge strategies don't override mandatory policies incorrectly

### "Embedding service not configured"

**Cause**: Drift detection with semantic analysis requires an embedding service, but none is provided to the GovernanceEngine.

**Solutions**:
```rust
// Configure embedding service
let embedding_service = Arc::new(YourEmbeddingService::new());
let mut engine = GovernanceEngine::new()
    .with_embedding_service(embedding_service);
```

**Workaround**: Disable semantic drift detection by removing content from drift check context or ensure embedding service is configured.

### "LLM drift analysis failed"

**Cause**: LLM service is unavailable or returns an error during drift analysis.

**Solutions**:
```rust
// Configure LLM service properly
let llm_service = Arc::new(YourLlmService::new());
let mut engine = GovernanceEngine::new()
    .with_llm_service(llm_service);
```

**Debugging**:
- Check LLM service API keys and endpoints
- Verify network connectivity to LLM provider
- Check service quotas and rate limits

---

## Policy Validation Issues

### Policy Rules Not Evaluating

**Symptoms**: Policies exist but violations aren't detected when they should be.

**Common causes**:

#### 1. Wrong Target Context
```rust
// INCORRECT - Missing context key
let mut context = HashMap::new();
// No "dependencies" key for dependency rule

// CORRECT - Provide required context
let mut context = HashMap::new();
context.insert("dependencies".to_string(), serde_json::json!(["jquery", "lodash"]));
```

#### 2. Invalid Regex Patterns
```rust
// INCORRECT - Invalid regex
value: serde_json::json!("^[unclosed regex")

// CORRECT - Valid regex with proper escaping
value: serde_json::json!("^#\\s*ADR\\s+\\d+")
```

#### 3. Rule Type Mismatch
```rust
// INCORRECT - Using Allow rule with MustUse (should be Deny)
PolicyRule {
    rule_type: RuleType::Allow,
    operator: ConstraintOperator::MustUse,
    // ...
}

// CORRECT - Deny rule with MustUse (forbidden dependency)
PolicyRule {
    rule_type: RuleType::Deny,
    operator: ConstraintOperator::MustUse,
    // ...
}
```

### Policy Inheritance Not Working

**Symptoms**: Child layer policies don't inherit or override parent policies correctly.

**Debugging steps**:
```rust
// Enable debug logging to see policy resolution
use tracing::{debug, Level};
tracing_subscriber::fmt()
    .with_max_level(Level::DEBUG)
    .init();

// Check resolved policies manually
let resolved_policies = engine.resolve_active_policies(
    KnowledgeLayer::Project,
    &context,
    Some(&tenant_ctx)
).await;

for policy in resolved_policies {
    debug!("Active policy: {} at layer {:?}", policy.id, policy.layer);
}
```

**Common inheritance issues**:

#### 1. Override Strategy Not Used
```rust
// INCORRECT - Merge strategy won't override parent rule
merge_strategy: RuleMergeStrategy::Merge

// CORRECT - Override to replace parent rule completely
merge_strategy: RuleMergeStrategy::Override
```

#### 2. Mandatory Policy Override Attempt
```rust
// Tenant mandatory policy - CANNOT be overridden
let company_policy = Policy {
    mode: PolicyMode::Mandatory,
    // ...
};

// Project policy with same ID - will be IGNORED
let project_policy = Policy {
    mode: PolicyMode::Optional,
    merge_strategy: RuleMergeStrategy::Override, // Won't work
    // ...
};
```

---

## Drift Detection Problems

### Slow Drift Detection

**Causes**: Large content files, slow embedding services, or inefficient policy evaluation.

**Solutions**:

#### 1. Tune Embedding Service
```rust
// Use faster embedding model or batch processing
let embedding_config = EmbeddingConfig {
    model: "text-embedding-3-small".to_string(), // Faster model
    batch_size: 32,
    timeout_ms: 5000,
};
```

#### 2. Optimize Policy Evaluation
```rust
// Use specific regex patterns instead of broad ones
value: serde_json::json!("^API_KEY\\s*=\\s*['\"][^'\"]+['\"]") // Specific
// vs
value: serde_json::json!(".*KEY.*") // Broad and slow
```

#### 3. Cache Drift Results
```rust
// In HybridGovernanceClient, drift results are cached
let client = HybridGovernanceClient::new(remote_url, engine)
    .with_cache_ttl(Duration::from_secs(600)); // 10 minute cache
```

### False Positive Drift Detection

**Causes**: Semantic similarity threshold too low, overly broad policies, or stale policy version hashes.

**Solutions**:

#### 1. Adjust Semantic Threshold
```rust
// Current threshold is 0.8, increase for fewer false positives
let violations = engine.check_contradictions(
    &tenant_ctx,
    content,
    0.9 // Higher threshold = fewer matches
).await?;
```

#### 2. Update Policy Version Hashes
```rust
// Add version_hash to policy metadata
let mut metadata = HashMap::new();
metadata.insert("version_hash".to_string(), serde_json::json!("abc123"));

let policy = Policy {
    metadata,
    // ...
};
```

---

## Storage Connection Issues

### PostgreSQL Connection Failures

**Common errors and solutions**:

#### 1. Connection Timeout
```toml
# config.toml
[providers.postgres]
host = "localhost"
port = 5432
timeout_seconds = 30  # Increase if needed
pool_size = 10        # Decrease if hitting connection limits
```

#### 2. Authentication Failures
```bash
# Set environment variables (recommended)
export AETERNA_POSTGRES_PASSWORD="your_password"
export AETERNA_POSTGRES_USERNAME="your_user"

# Or use config file
[providers.postgres]
username = "your_user"
password = "your_password"
```

#### 3. Database Not Found
```sql
-- Create database manually
CREATE DATABASE memory_knowledge;

-- Or use connection string with database
postgresql://user:password@localhost:5432/memory_knowledge
```

#### 4. Missing Extensions
```sql
-- Required for vector operations
CREATE EXTENSION IF NOT EXISTS vector;

-- Required for UUID generation
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
```

### Redis Connection Issues

**Common problems**:
```toml
[providers.redis]
host = "localhost"
port = 6379
timeout_seconds = 30
pool_size = 10
```

**Debugging**:
```bash
# Test Redis connection
redis-cli -h localhost -p 6379 ping

# Check Redis is running
redis-cli info server
```

### Qdrant Vector Database Issues

**Configuration**:
```toml
[providers.qdrant]
host = "localhost"
port = 6333
collection = "memory_embeddings"
timeout_seconds = 30
```

**Common issues**:
```bash
# Create collection manually
curl -X PUT http://localhost:6333/collections/memory_embeddings \
  -H 'Content-Type: application/json' \
  -d '{
    "vectors": {
      "size": 1536,
      "distance": "Cosine"
    }
  }'
```

---

## Permit.io Integration

### Hybrid Mode Configuration

For Hybrid and Remote modes, configure remote governance URL:

```bash
# Environment variables
export AETERNA_DEPLOYMENT_MODE="hybrid"
export AETERNA_REMOTE_GOVERNANCE_URL="https://your-governance-api.com"
```

Or in config:
```toml
[deployment]
mode = "hybrid"
remote_url = "https://your-governance-api.com"
sync_enabled = true
```

### Remote API Errors

**Common issues**:

#### 1. Authentication Headers Missing
```rust
// Headers are automatically added by HybridGovernanceClient
// Ensure tenant and user context are properly set
let ctx = TenantContext::new(
    TenantId::new("your-tenant").unwrap(),
    UserId::new("your-user").unwrap(),
);
```

#### 2. Network Timeouts
```rust
let client = HybridGovernanceClient::new(remote_url, engine)
    .with_cache_ttl(Duration::from_secs(300))
    .with_sync_interval(Duration::from_secs(60));
```

#### 3. API Response Format Errors
```rust
// Check API response format matches expected ValidationResult
// Common issue: API returns different JSON structure
```

### Sync Conflicts

**Symptoms**: Changes not syncing between local and remote governance.

**Debugging**:
```rust
// Check sync state
let state = client.get_sync_state().await;
println!("Pending changes: {}", state.pending_changes.len());
println!("Local version: {}", state.local_version);
println!("Remote version: {}", state.remote_version);

// Manually sync pending changes
let synced = client.sync_pending_changes(&ctx).await?;
println!("Synced {} changes", synced);
```

---

## Performance Troubleshooting

### High Memory Usage

**Causes**: Large policy sets, excessive caching, or memory leaks in policy evaluation.

**Solutions**:

#### 1. Optimize Cache Configuration
```rust
// Reduce cache TTL for hybrid client
let client = HybridGovernanceClient::new(remote_url, engine)
    .with_cache_ttl(Duration::from_secs(60)); // Was 300
```

#### 2. Batch Policy Evaluation
```rust
// Process policies in batches rather than all at once
for batch in policies.chunks(10) {
    for policy in batch {
        // Evaluate policy
    }
}
```

### Slow Policy Resolution

**Optimization techniques**:

#### 1. Index Policy Rules
```rust
// Store policies by layer for faster lookup
let mut policies_by_layer: HashMap<KnowledgeLayer, Vec<Policy>> = HashMap::new();
for policy in policies {
    policies_by_layer.entry(policy.layer).or_default().push(policy);
}
```

#### 2. Pre-compile Regex Patterns
```rust
// Compile regex once instead of on every evaluation
lazy_static! {
    static ref SECRET_PATTERN: Regex = Regex::new(r"SECRET_.*").unwrap();
}

// Then use in policy evaluation
if SECRET_PATTERN.is_match(content) {
    // Violation detected
}
```

#### 3. Use Efficient Storage Queries
```sql
-- Add indexes for common queries
CREATE INDEX idx_unit_policies_unit_id ON unit_policies(unit_id);
CREATE INDEX idx_organizational_units_tenant_id ON organizational_units(tenant_id);
CREATE INDEX idx_governance_events_timestamp ON governance_events(timestamp);
```

---

## Debugging Checklist

### Enable Verbose Logging

```bash
# Environment variable
export RUST_LOG=debug,aeterna=trace

# Or in code
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
tracing_subscriber::registry()
    .with(tracing_subscriber::EnvFilter::from_default_env())
    .with(tracing_subscriber::fmt::layer())
    .init();
```

### Inspect Resolved Policies

```rust
// Get all active policies for debugging
let active_policies = engine.resolve_active_policies(
    KnowledgeLayer::Project,
    &context,
    Some(&tenant_ctx)
).await;

for policy in &active_policies {
    println!("Policy: {} at layer {:?}", policy.id, policy.layer);
    for rule in &policy.rules {
        println!("  Rule: {} ({})", rule.id, rule.severity);
    }
}
```

### Trace Policy Inheritance

```rust
// Manually trace policy hierarchy
let layers = [
    KnowledgeLayer::Tenant,
    KnowledgeLayer::Org,
    KnowledgeLayer::Team,
    KnowledgeLayer::Project,
];

for layer in &layers {
    if let Some(layer_policies) = engine.policies.get(layer) {
        println!("Layer {:?} has {} policies", layer, layer_policies.len());
        for policy in layer_policies {
            println!("  - {} ({:?})", policy.id, policy.mode);
        }
    }
}
```

### Validate Context Structure

```rust
// Ensure context has all required keys
fn validate_context(context: &HashMap<String, serde_json::Value>) -> Vec<String> {
    let required_keys = vec!["projectId", "content", "dependencies"];
    let mut missing = Vec::new();
    
    for key in &required_keys {
        if !context.contains_key(*key) {
            missing.push(key.to_string());
        }
    }
    
    missing
}

let missing_keys = validate_context(&context);
if !missing_keys.is_empty() {
    println!("Missing context keys: {:?}", missing_keys);
}
```

### Monitor Storage Performance

```sql
-- Check slow queries
SELECT query, mean_time, calls 
FROM pg_stat_statements 
ORDER BY mean_time DESC 
LIMIT 10;

-- Check table sizes
SELECT 
    schemaname,
    tablename,
    pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) as size
FROM pg_tables 
WHERE schemaname = 'public'
ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;
```

---

## FAQ

### Q: Why are my policies not being evaluated?

**A**: Check that:
1. Context contains the required keys for your rule targets
2. Policy IDs are unique across all layers
3. Regex patterns are valid
4. Rule types match your intent (Allow vs Deny)

### Q: How do I debug policy inheritance?

**A**: 
1. Enable debug logging: `RUST_LOG=debug`
2. Use `resolve_active_policies()` to see what policies are active
3. Check merge strategies - `Override` replaces, `Merge` combines, `Intersect` keeps only common rules

### Q: My drift detection is too slow

**A**: 
1. Reduce content size being analyzed
2. Use faster embedding models
3. Increase cache TTL in hybrid mode
4. Optimize regex patterns to be more specific

### Q: Governance is blocking valid changes

**A**: 
1. Check policy severity levels - `Block` prevents operations
2. Verify rule logic matches your intent
3. Review mandatory vs optional policy modes
4. Check for contradictory policies at different layers

### Q: Remote governance isn't syncing

**A**: 
1. Verify `AETERNA_REMOTE_GOVERNANCE_URL` is set and accessible
2. Check authentication headers are being sent
3. Monitor sync state with `get_sync_state()`
4. Manually trigger sync with `sync_pending_changes()`

### Q: How do I add new constraint operators?

**A**: The system supports these operators:
- `MustExist`: Key must be present in context
- `MustNotExist`: Key must not be present
- `MustUse`: Value must contain specified item
- `MustNotUse`: Value must not contain specified item
- `MustMatch`: Value must match regex pattern
- `MustNotMatch`: Value must not match regex pattern

New operators require modifying the `evaluate_rule` method in `GovernanceEngine`.

### Q: What's the difference between policy modes?

**A**: 
- `Mandatory`: Cannot be overridden by lower layers, always enforced
- `Optional`: Can be overridden by lower layers using `Override` strategy
- Use `Mandatory` for security and compliance rules
- Use `Optional` for style guidelines and preferences

### Q: How do I test policies without affecting production?

**A**: 
1. Use separate tenant contexts for testing
2. Create test policies with test-specific IDs
3. Use the validation API directly with test contexts
4. Run tests with `cargo test governance`

### Q: Why are my policy changes not taking effect?

**A**: 
1. Check if policies are cached - restart service or wait for cache expiry
2. Verify policy IDs are unique (duplicates are ignored)
3. Ensure policy layer is correct in the hierarchy
4. Check for syntax errors in policy configuration

---

## Emergency Procedures

### Governance Engine Not Responding

1. Check service health: `curl http://localhost:8080/health`
2. Review logs: `journalctl -u aeterna-governance -f`
3. Restart service: `systemctl restart aeterna-governance`
4. Check storage connectivity with manual queries

### Storage Outage Recovery

1. Switch to read-only mode to prevent data loss
2. Check backup status and restore if needed
3. Verify storage service is running and accessible
4. Clear any in-memory caches that might be stale
5. Gradually resume operations with monitoring

### Policy Corruption

1. Export current policies from database
2. Identify and remove corrupted policy entries
3. Restore policies from backup or configuration
4. Validate all policies before re-enabling enforcement
5. Monitor system for unexpected behavior

---

## Contact and Support

For additional support:
1. Check system logs with `RUST_LOG=debug` enabled
2. Review this troubleshooting guide
3. Check test files for working examples
4. Consult the architecture documentation
5. Create detailed issue reports with context and error messages