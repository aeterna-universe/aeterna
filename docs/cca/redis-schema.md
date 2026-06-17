# CCA Redis Schema Documentation

This document describes the Redis key patterns and data structures used by CCA (Confucius Code Agent) capabilities in Aeterna.

## Overview

CCA uses Redis for:
- **Summary caching**: Fast access to pre-computed summaries
- **Extension state**: Per-session state for extensions
- **Distributed locking**: Coordination between CCA workers
- **Event streaming**: Trajectory and error event publication

## Key Patterns

All CCA-related keys follow the pattern:
```
{namespace}:{tenant_id}:{component}:{...identifiers}
```

### Summary Cache Keys

Pattern: `summary:{tenant_id}:{layer}:{entry_id}:{depth}`

| Field | Type | Description |
|-------|------|-------------|
| tenant_id | string | Tenant identifier |
| layer | string | Memory layer: `agent`, `user`, `session`, `project`, `team`, `org`, `tenant` |
| entry_id | string | Memory entry UUID |
| depth | string | Summary depth: `sentence`, `paragraph`, `detailed` |

**Examples:**
```
summary:acme-corp:session:550e8400-e29b-41d4-a716-446655440000:sentence
summary:acme-corp:project:myproject-123:paragraph
summary:acme-corp:team:api-team:detailed
```

**Value Schema (JSON):**
```json
{
  "depth": "sentence",
  "content": "This is a one-sentence summary of the memory entry.",
  "token_count": 47,
  "generated_at": 1705680000000,
  "source_hash": "xxhash64:abc123def456",
  "personalized": false,
  "personalization_context": null
}
```

**TTL**: Configurable via `cache_ttl_secs` (default: 300 seconds)

### Extension State Keys

Pattern: `ext_state:{tenant_id}:{session_id}:{extension_id}`

| Field | Type | Description |
|-------|------|-------------|
| tenant_id | string | Tenant identifier |
| session_id | string | Session UUID |
| extension_id | string | Extension identifier |

**Examples:**
```
ext_state:acme-corp:sess-123:context-enricher
ext_state:acme-corp:sess-456:code-reviewer
```

**Value Schema (JSON):**
```json
{
  "state": {
    "conversation_history": [...],
    "custom_field": "value"
  },
  "version": 1
}
```

**TTL**: Configurable via `state_ttl_seconds` (default: 3600 seconds)

**Size Limit**: Configurable via `max_state_size_bytes` (default: 1MB)

### Distributed Lock Keys

Pattern: `lock:{component}:{resource_id}`

CCA components use these lock patterns:

| Component | Pattern | Purpose |
|-----------|---------|---------|
| Summary Generator | `lock:summary_gen:{tenant_id}:{entry_id}` | Prevent duplicate summarization |
| Note Distillation | `lock:distill:{tenant_id}:{session_id}` | Serialize session distillation |
| Hindsight Capture | `lock:hindsight:{tenant_id}:{error_hash}` | Deduplicate error capture |
| Meta-Agent Loop | `lock:meta_loop:{tenant_id}:{loop_id}` | Single loop execution |

**Value**: UUID lock token

**TTL**: Lock-specific (typically 60-300 seconds)

**Lock Acquisition Script (Lua):**
```lua
-- SET key value NX EX ttl
if redis.call("SET", KEYS[1], ARGV[1], "NX", "EX", ARGV[2]) then
    return "OK"
else
    return nil
end
```

**Lock Release Script (Lua):**
```lua
if redis.call("GET", KEYS[1]) == ARGV[1] then
    return redis.call("DEL", KEYS[1])
else
    return 0
end
```

### Job Coordination Keys

#### Job Completion Tracking
Pattern: `job_completed:{job_name}`

**Purpose**: Deduplication of scheduled jobs

**Value**: Timestamp of last completion

**TTL**: Deduplication window (configurable)

#### Job Checkpoints
Pattern: `job_checkpoint:{job_name}:{tenant_id}`

**Purpose**: Resume interrupted jobs

**Value Schema (JSON):**
```json
{
  "job_name": "summary_refresh",
  "tenant_id": "acme-corp",
  "progress": {
    "processed": 150,
    "total": 500,
    "last_entry_id": "entry-123"
  },
  "started_at": 1705680000000,
  "updated_at": 1705680300000
}
```

### Event Stream Keys

#### Trajectory Events
Pattern: `trajectory:{tenant_id}:{session_id}`

**Type**: Redis Stream

**Purpose**: Capture tool execution events for note-taking

**Entry Schema:**
```json
{
  "event_id": "evt-123",
  "timestamp": 1705680000000,
  "tool_name": "file_edit",
  "input": "{...}",
  "output": "{...}",
  "success": true,
  "duration_ms": 150,
  "metadata": "{...}"
}
```

**Consumer Groups**: `note_taking_workers`

#### Governance Events
Pattern: `governance:events:{tenant_id}`

**Type**: Redis Stream

**Purpose**: Policy and constraint events for hindsight learning

**Entry Schema:**
```json
{
  "event": "{...serialized GovernanceEvent...}"
}
```

### Budget Tracking Keys

#### Summarization Budget
Pattern: `budget:summarization:{tenant_id}:{period}`

**Periods**: `hourly:{hour}`, `daily:{date}`

**Purpose**: Track LLM token usage for cost control

**Value Schema (JSON):**
```json
{
  "tokens_used": 45000,
  "requests": 150,
  "errors": 2,
  "period_start": 1705680000000
}
```

**TTL**: 
- Hourly: 2 hours
- Daily: 48 hours

### Circuit Breaker Keys

Pattern: `circuit:{component}:{tenant_id}`

**Purpose**: Track failures for circuit breaker pattern

**Value Schema (JSON):**
```json
{
  "state": "closed",
  "failure_count": 0,
  "last_failure_at": null,
  "opened_at": null,
  "half_open_attempts": 0
}
```

**States**: `closed`, `open`, `half_open`

## Data Structures by Component

### Context Architect

| Key Pattern | Type | TTL | Purpose |
|-------------|------|-----|---------|
| `summary:{t}:{l}:{e}:{d}` | String (JSON) | 300s | Summary cache |
| `lock:summary_gen:{t}:{e}` | String | 60s | Generation lock |
| `budget:summarization:{t}:{p}` | String (JSON) | 2-48h | Cost tracking |
| `circuit:summarization:{t}` | String (JSON) | None | LLM circuit breaker |

### Note-Taking Agent

| Key Pattern | Type | TTL | Purpose |
|-------------|------|-----|---------|
| `trajectory:{t}:{s}` | Stream | 24h | Event capture |
| `lock:distill:{t}:{s}` | String | 300s | Distillation lock |
| `job_checkpoint:note_distill:{t}` | String (JSON) | 1h | Resume state |

### Hindsight Learning

| Key Pattern | Type | TTL | Purpose |
|-------------|------|-----|---------|
| `lock:hindsight:{t}:{h}` | String | 60s | Capture lock |
| `error_embedding:{t}:{h}` | String (JSON) | 7d | Cached embeddings |

### Meta-Agent

| Key Pattern | Type | TTL | Purpose |
|-------------|------|-----|---------|
| `lock:meta_loop:{t}:{l}` | String | 600s | Loop execution lock |
| `meta_state:{t}:{l}` | String (JSON) | 1h | Loop state |
| `job_checkpoint:meta_agent:{t}` | String (JSON) | 1h | Resume state |

### Extension System

| Key Pattern | Type | TTL | Purpose |
|-------------|------|-----|---------|
| `ext_state:{t}:{s}:{e}` | String (JSON) | 3600s | Extension state |
| `ext_lru:{t}` | Sorted Set | None | LRU eviction tracking |

## Memory Management

### LRU Eviction for Extension State

When tenant state exceeds configured limits, LRU eviction is triggered:

```redis
-- Track access time in sorted set
ZADD ext_lru:{tenant_id} {timestamp} {session_id}:{extension_id}

-- Find oldest entries
ZRANGE ext_lru:{tenant_id} 0 10

-- Evict oldest entry
DEL ext_state:{tenant_id}:{oldest_session}:{oldest_extension}
ZREM ext_lru:{tenant_id} {oldest_session}:{oldest_extension}
```

### Compression

Large state values (>10KB) are compressed with zstd before storage:

```rust
// Compression prefix indicates compressed data
const COMPRESSED_PREFIX: &[u8] = b"ZSTD:";

// Check if value is compressed
if value.starts_with(COMPRESSED_PREFIX) {
    let compressed = &value[COMPRESSED_PREFIX.len()..];
    zstd::decode_all(compressed)?
} else {
    value
}
```

## Monitoring Keys

### Metrics (for observability integration)

| Metric Key | Type | Description |
|------------|------|-------------|
| `metrics:cca:summary_latency:{t}` | List | Summary generation latencies |
| `metrics:cca:cache_hits:{t}` | Counter | Summary cache hit count |
| `metrics:cca:cache_misses:{t}` | Counter | Summary cache miss count |
| `metrics:cca:distillation_count:{t}` | Counter | Notes distilled |
| `metrics:cca:hindsight_matches:{t}` | Counter | Error pattern matches |
| `metrics:cca:ext_evictions:{t}` | Counter | State evictions |

## Configuration

### Environment Variables

```bash
# Redis connection
RD_HOST=localhost
RD_PORT=6379
RD_DB=0

# CCA-specific settings (also configurable via config file)
CCA_CACHE_TTL_SECS=300
CCA_STATE_TTL_SECS=3600
CCA_MAX_STATE_BYTES=1048576
```

### Config File (aeterna.toml)

```toml
[storage.redis]
host = "localhost"
port = 6379
db = 0

[cca.context_architect]
cache_ttl_secs = 300
enable_caching = true

[cca.extension]
state_ttl_seconds = 3600
max_state_size_bytes = 1048576
```

## Cleanup Procedures

### Expired Cache Cleanup

Summaries use TTL-based expiration. No manual cleanup needed.

### Orphaned State Cleanup

For session-scoped state after session end:

```bash
# Find orphaned state keys (sessions that ended)
redis-cli KEYS "ext_state:*" | xargs -I {} redis-cli TTL {}

# Force cleanup of specific tenant
redis-cli KEYS "ext_state:tenant-id:*" | xargs redis-cli DEL
```

### Stream Trimming

Trajectory streams should be trimmed to prevent unbounded growth:

```redis
-- Keep last 1000 entries per stream
XTRIM trajectory:{tenant_id}:{session_id} MAXLEN ~ 1000
```

Automated via scheduler:
```rust
let scheduler = JobScheduler::new()
    .with_job("stream_trim", Duration::from_secs(3600), |redis| async {
        redis.xtrim_all_trajectory_streams(1000).await
    });
```

## High Availability Considerations

### Redis Cluster

CCA keys are designed for Redis Cluster compatibility:

- All keys include tenant_id for consistent hashing
- No cross-slot operations within atomic transactions
- Lock scripts use single-key operations

### Redis Sentinel

For Sentinel deployments, configure connection string:

```toml
[storage.redis]
connection_string = "redis+sentinel://sentinel1:26379,sentinel2:26379/mymaster"
```

### Replication Lag

Summary cache reads tolerate eventual consistency. For critical paths (locks, state), ensure:

```rust
// Use WAIT for synchronous replication on critical writes
redis.wait(1, Duration::from_millis(100)).await?;
```

## Troubleshooting

### High Memory Usage

Check key distribution:
```bash
redis-cli --scan --pattern "summary:*" | wc -l
redis-cli --scan --pattern "ext_state:*" | wc -l
redis-cli INFO memory
```

### Lock Contention

Monitor lock acquisition failures:
```bash
redis-cli MONITOR | grep "lock:"
```

### Stream Lag

Check consumer group lag:
```bash
redis-cli XINFO GROUPS trajectory:tenant-id:session-id
```

## Related Documentation

- [PostgreSQL Migrations](./migrations)
- [Configuration Guide](./configuration)
