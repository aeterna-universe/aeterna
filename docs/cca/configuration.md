# CCA Configuration

This document details all configuration options for CCA (Confucius Code Agent) capabilities in Aeterna. Configuration is specified in `config/aeterna.toml` under the `[cca]` section.

## Configuration Overview

CCA provides four main configuration sections:

1. **CcaConfig** - Top-level enable/disable for all CCA capabilities
2. **ContextArchitectConfig** - Controls hierarchical context assembly
3. **NoteTakingConfig** - Manages trajectory capture and distillation
4. **HindsightConfig** - Error pattern recognition settings
5. **MetaAgentConfig** - Build-test-improve loop parameters

## Complete Configuration Example

```toml
[cca]
# Master switch for all CCA capabilities
enabled = true

[cca.context_architect]
# Enable Context Architect component
enabled = true

# Default token budget for context assembly (100-32000)
# Recommendation: 4000 for GPT-3.5, 8000 for GPT-4, 16000 for GPT-4-32k
default_token_budget = 4000

# Memory layer priorities (queried in this order)
# Higher priority layers are queried first and included preferentially
layer_priorities = ["session", "project", "team", "org", "tenant"]

# Minimum relevance score for including memory entries (0.0-1.0)
# Lower = more entries, higher = only highly relevant
min_relevance_score = 0.3

# Enable caching of assembled contexts
enable_caching = true

# Cache time-to-live in seconds
cache_ttl_secs = 300

# Staleness policy when cache is expired
# Options: "serve_stale_warn" | "regenerate_blocking" | "regenerate_async"
staleness_policy = "serve_stale_warn"

# Timeout for context assembly in milliseconds
assembly_timeout_ms = 100

# Enable parallel queries across memory layers (recommended for performance)
enable_parallel_queries = true

# Enable early termination when token budget is satisfied
# (stops querying lower-priority layers if budget already filled)
enable_early_termination = true

[cca.note_taking]
# Enable Note-Taking Agent component
enabled = true

# Number of trajectory events before auto-distillation triggers
# Higher = more context for distillation, lower = more frequent updates
auto_distill_threshold = 10

# Allow manual triggering of distillation via API
manual_trigger_enabled = true

# Enable detection and redaction of sensitive patterns (API keys, tokens, etc.)
sensitive_patterns_enabled = true

# Capture mode controls what events are recorded
# Options: "all" | "sampled" | "errors_only" | "disabled"
capture_mode = "all"

# Sampling rate when capture_mode = "sampled" (percentage 1-100)
# Example: 10 = capture 1 in 10 events
sampling_rate = 10

# Maximum overhead budget per event capture in milliseconds
# Events exceeding this are dropped to prevent performance impact
overhead_budget_ms = 5

# Maximum queue size for async event buffering
queue_size = 1000

# Batch size for writing events to storage
batch_size = 10

# Batch flush interval in milliseconds
# Events are written every N ms or when batch_size is reached
batch_flush_ms = 100

[cca.hindsight]
# Enable Hindsight Learning component
enabled = true

# Semantic similarity threshold for matching errors (0.0-1.0)
# Higher = only very similar errors, lower = broader matching
semantic_threshold = 0.8

# Maximum number of matching resolutions to return
max_results = 5

# Success rate threshold for promoting resolutions to broader layers
# Example: 0.8 = promote if 80% success rate after multiple applications
promotion_threshold = 0.8

# Automatically capture all errors (vs manual capture only)
auto_capture_enabled = true

[cca.meta_agent]
# Enable Meta-Agent component
enabled = true

# Maximum iterations for build-test-improve loop
max_iterations = 3

# Timeout for entire iteration (build + test + improve) in seconds
iteration_timeout_secs = 300

# Timeout for build phase in seconds
build_timeout_secs = 120

# Timeout for test phase in seconds
test_timeout_secs = 60

# Automatically escalate to higher authority (human/team lead) on repeated failures
auto_escalate_on_failure = true
```

## Configuration Sections

### 1. CcaConfig (Top-Level)

The master configuration for all CCA capabilities.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Master switch for all CCA capabilities. If false, all CCA components are disabled. |
| `context_architect` | object | See below | Configuration for Context Architect component |
| `note_taking` | object | See below | Configuration for Note-Taking Agent component |
| `hindsight` | object | See below | Configuration for Hindsight Learning component |
| `meta_agent` | object | See below | Configuration for Meta-Agent component |

### 2. ContextArchitectConfig

Controls hierarchical context assembly and compression.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable Context Architect component |
| `default_token_budget` | integer | `4000` | Default token budget for context assembly (100-32000). Adjust based on your LLM's context window. |
| `layer_priorities` | array | `["session", "project", "team", "org", "tenant"]` | Memory layer query order. First = highest priority. |
| `min_relevance_score` | float | `0.3` | Minimum relevance score for including entries (0.0-1.0). Lower = more entries. |
| `enable_caching` | boolean | `true` | Enable caching of assembled contexts for performance |
| `cache_ttl_secs` | integer | `300` | Cache time-to-live in seconds (5 minutes default) |
| `staleness_policy` | string | `"serve_stale_warn"` | How to handle expired cache: `"serve_stale_warn"` (return stale + warn), `"regenerate_blocking"` (wait for fresh), `"regenerate_async"` (return stale + refresh async) |
| `assembly_timeout_ms` | integer | `100` | Maximum time to spend assembling context (milliseconds) |
| `enable_parallel_queries` | boolean | `true` | Query multiple memory layers simultaneously (recommended) |
| `enable_early_termination` | boolean | `true` | Stop querying lower-priority layers if budget already satisfied |

#### Layer Priorities Explained

The `layer_priorities` array determines:

1. Which layers are queried
2. The order of querying (first to last)
3. Which entries are kept when over token budget

Example configurations:

```toml
# Session-focused (recent context prioritized)
layer_priorities = ["session", "user", "project"]

# Project-focused (broad project context)
layer_priorities = ["project", "session", "team"]

# Organizational knowledge (team/tenant standards)
layer_priorities = ["team", "org", "tenant", "project"]

# All layers (comprehensive but may exceed budget)
layer_priorities = ["agent", "user", "session", "project", "team", "org", "tenant"]
```

#### Staleness Policy Explained

| Policy | Behavior | Use Case |
|--------|----------|----------|
| `serve_stale_warn` | Return cached data + log warning | High availability, can tolerate slightly outdated context |
| `regenerate_blocking` | Wait for fresh data (blocks request) | Accuracy critical, latency acceptable |
| `regenerate_async` | Return stale + refresh in background | Best of both: fast response + eventual freshness |

### 3. NoteTakingConfig

Manages trajectory capture and distillation to knowledge.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable Note-Taking Agent component |
| `auto_distill_threshold` | integer | `10` | Number of events before automatic distillation |
| `manual_trigger_enabled` | boolean | `true` | Allow manual distillation via `note_capture` tool |
| `sensitive_patterns_enabled` | boolean | `true` | Detect and redact sensitive data (API keys, passwords, tokens) |
| `capture_mode` | string | `"all"` | What to capture: `"all"`, `"sampled"`, `"errors_only"`, `"disabled"` |
| `sampling_rate` | integer | `10` | Percentage when `capture_mode = "sampled"` (1-100) |
| `overhead_budget_ms` | integer | `5` | Max overhead per event capture (milliseconds) |
| `queue_size` | integer | `1000` | Max events in async buffer |
| `batch_size` | integer | `10` | Events per batch write |
| `batch_flush_ms` | integer | `100` | Flush interval (milliseconds) |

#### Capture Mode Explained

| Mode | Behavior | Use Case |
|------|----------|----------|
| `all` | Capture every trajectory event | Development, debugging, comprehensive learning |
| `sampled` | Capture 1 in N events (see `sampling_rate`) | Production with high throughput, cost control |
| `errors_only` | Only capture failed events | Error-focused learning, minimal overhead |
| `disabled` | No capture | Temporarily disable without removing config |

#### Performance Tuning

For high-throughput scenarios:

```toml
[cca.note_taking]
# Use sampling to reduce volume
capture_mode = "sampled"
sampling_rate = 20  # 1 in 20 events (5%)

# Increase batch size and queue for better throughput
queue_size = 5000
batch_size = 50
batch_flush_ms = 500  # Flush less frequently

# Reduce per-event overhead budget
overhead_budget_ms = 2
```

For comprehensive learning:

```toml
[cca.note_taking]
# Capture everything
capture_mode = "all"

# Frequent distillation for immediate insights
auto_distill_threshold = 5

# Smaller batches for low latency
batch_size = 5
batch_flush_ms = 50
```

### 4. HindsightConfig

Error pattern recognition and resolution tracking.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable Hindsight Learning component |
| `semantic_threshold` | float | `0.8` | Similarity threshold for matching errors (0.0-1.0). Higher = stricter matching. |
| `max_results` | integer | `5` | Maximum number of resolution suggestions to return |
| `promotion_threshold` | float | `0.8` | Success rate required to promote resolution to broader layer (0.0-1.0) |
| `auto_capture_enabled` | boolean | `true` | Automatically capture all errors (vs manual only) |

#### Semantic Threshold Tuning

| Threshold | Matching Behavior | Use Case |
|-----------|-------------------|----------|
| 0.95-1.0 | Nearly identical errors only | Prevent false positives, highly specific resolutions |
| 0.8-0.95 | Similar errors with same root cause | Balanced (recommended) |
| 0.6-0.8 | Broad error families | Exploratory, learning phase |
| <0.6 | Very loose matching | Not recommended (too many false matches) |

Example:

```toml
# Strict matching for production
[cca.hindsight]
semantic_threshold = 0.9
max_results = 3
promotion_threshold = 0.9  # Only promote highly reliable resolutions

# Exploratory learning mode
[cca.hindsight]
semantic_threshold = 0.7
max_results = 10
promotion_threshold = 0.7
```

### 5. MetaAgentConfig

Build-test-improve loop settings.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable Meta-Agent component |
| `max_iterations` | integer | `3` | Maximum build-test-improve iterations |
| `iteration_timeout_secs` | integer | `300` | Timeout for full iteration (5 minutes) |
| `build_timeout_secs` | integer | `120` | Timeout for build phase (2 minutes) |
| `test_timeout_secs` | integer | `60` | Timeout for test phase (1 minute) |
| `auto_escalate_on_failure` | boolean | `true` | Escalate to human if all iterations fail |

#### Iteration Budget Tuning

For quick iterations (e.g., small scripts):

```toml
[cca.meta_agent]
max_iterations = 5
iteration_timeout_secs = 60  # 1 minute per iteration
build_timeout_secs = 30
test_timeout_secs = 20
```

For complex builds (e.g., large projects):

```toml
[cca.meta_agent]
max_iterations = 3
iteration_timeout_secs = 600  # 10 minutes per iteration
build_timeout_secs = 300  # 5 minutes for build
test_timeout_secs = 180  # 3 minutes for tests
```

## Environment-Specific Configurations

### Development Environment

```toml
[cca]
enabled = true

[cca.context_architect]
default_token_budget = 8000  # Generous budget
enable_parallel_queries = true
enable_early_termination = false  # Get all context for debugging
min_relevance_score = 0.2  # Lower threshold

[cca.note_taking]
capture_mode = "all"  # Capture everything
auto_distill_threshold = 5  # Frequent distillation
manual_trigger_enabled = true

[cca.hindsight]
semantic_threshold = 0.7  # Broader matching for learning
max_results = 10

[cca.meta_agent]
max_iterations = 5  # More attempts
auto_escalate_on_failure = false  # Handle manually
```

### Production Environment

```toml
[cca]
enabled = true

[cca.context_architect]
default_token_budget = 4000  # Cost control
enable_parallel_queries = true
enable_early_termination = true  # Performance optimization
enable_caching = true
cache_ttl_secs = 300

[cca.note_taking]
capture_mode = "sampled"  # Reduce volume
sampling_rate = 10  # 10% sampling
auto_distill_threshold = 20  # Less frequent
overhead_budget_ms = 3  # Strict overhead control

[cca.hindsight]
semantic_threshold = 0.85  # Stricter matching
max_results = 5
promotion_threshold = 0.85

[cca.meta_agent]
max_iterations = 3
auto_escalate_on_failure = true  # Alert on failures
```

### High-Throughput Environment

```toml
[cca]
enabled = true

[cca.context_architect]
default_token_budget = 2000  # Smaller budgets for speed
enable_parallel_queries = true
enable_early_termination = true
assembly_timeout_ms = 50  # Aggressive timeout

[cca.note_taking]
capture_mode = "errors_only"  # Minimal capture
queue_size = 10000  # Large buffer
batch_size = 100  # Large batches
batch_flush_ms = 1000  # Infrequent flushes
overhead_budget_ms = 1  # Very strict

[cca.hindsight]
enabled = true  # Still useful for errors
auto_capture_enabled = true

[cca.meta_agent]
enabled = false  # Disable for latency-sensitive scenarios
```

## Validation and Defaults

All configuration is validated on startup using the `validator` crate. If validation fails, Aeterna will not start and will output detailed error messages.

Default values are provided for all options, so minimal configuration is required:

```toml
# Minimal configuration (all components enabled with defaults)
[cca]
enabled = true
```

This is equivalent to the full default configuration shown at the beginning of this document.

## Configuration Hot Reload

CCA configuration supports hot reload without restarting the server (requires `config` crate hot reload feature):

1. Modify `config/aeterna.toml`
2. Send SIGHUP to Aeterna process: `kill -HUP <pid>`
3. Configuration is reloaded within 5 seconds

Note: Some changes (e.g., enabling/disabling components) may require draining in-flight requests.

## Monitoring Configuration Impact

Track these metrics to understand configuration effectiveness:

- **Token budget utilization**: `context_architect.tokens_used / context_architect.token_budget`
- **Cache hit rate**: `context_architect.cache_hits / context_architect.total_queries`
- **Note capture overhead**: `note_taking.capture_duration_ms`
- **Hindsight match rate**: `hindsight.matches_found / hindsight.queries`
- **Meta-agent iteration distribution**: Histogram of iterations needed

Adjust configuration based on these metrics to optimize for your workload.
