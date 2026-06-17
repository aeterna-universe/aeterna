# CCA Database Migrations

This document describes the database migrations required for CCA (Confucius Code Agent) capabilities in Aeterna.

## Migration Overview

CCA introduces two PostgreSQL migrations and extends the Redis schema:

| Migration | File | Purpose |
|-----------|------|---------|
| 007 | `007_cca_summaries.sql` | Summary schema extensions for Context Architect |
| 008 | `008_hindsight_tables.sql` | Error signatures, resolutions, and hindsight notes |

## Prerequisites

Before running migrations:

1. **PostgreSQL 16+** (stock; no extensions required — semantic vectors live in Qdrant)
2. **Existing Aeterna schema** (migrations 001-006 already applied)
3. **Database backup** taken before migration

```bash
# Check current migration state
psql -c "SELECT * FROM schema_migrations ORDER BY version;"

# Create backup
pg_dump -Fc aeterna > backup_pre_cca_$(date +%Y%m%d).dump
```

## Migration 007: CCA Summary Schema

### Purpose
Adds hierarchical summary storage to memory and knowledge entries for Context Architect compression.

### Changes

#### Table Modifications

**memory_entries** (existing table):
```sql
-- New columns added
summaries JSONB DEFAULT '{}'           -- Pre-computed summaries at multiple depths
context_vector VECTOR(1536)            -- Semantic vector for relevance matching
summary_updated_at BIGINT              -- Timestamp of last summary update
```

**knowledge_items** (existing table):
```sql
-- New column added
summaries JSONB DEFAULT '{}'           -- Pre-computed summaries
```

#### New Tables

**layer_summary_cache**:
```sql
CREATE TABLE layer_summary_cache (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    memory_layer TEXT NOT NULL,         -- 'agent', 'user', 'session', etc.
    entry_id TEXT NOT NULL,
    depth TEXT NOT NULL,                -- 'sentence', 'paragraph', 'detailed'
    content TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    source_hash TEXT NOT NULL,          -- Hash of source content for staleness detection
    personalized BOOLEAN DEFAULT FALSE,
    personalization_context TEXT,
    generated_at BIGINT NOT NULL,
    expires_at BIGINT,
    UNIQUE (tenant_id, entry_id, depth)
);
```

#### Indexes

```sql
-- Fast lookup by layer and update time
CREATE INDEX idx_memory_entries_summary_updated 
    ON memory_entries(tenant_id, memory_layer, summary_updated_at)
    WHERE summary_updated_at IS NOT NULL;

-- Cache lookup
CREATE INDEX idx_layer_summary_cache_lookup 
    ON layer_summary_cache(tenant_id, memory_layer, entry_id);

-- Cache expiry cleanup
CREATE INDEX idx_layer_summary_cache_expiry 
    ON layer_summary_cache(expires_at)
    WHERE expires_at IS NOT NULL;
```

### JSONB Schema: summaries

The `summaries` JSONB column uses this structure:

```json
{
  "sentence": {
    "depth": "sentence",
    "content": "One-line summary (~50 tokens)",
    "token_count": 47,
    "generated_at": 1705680000000,
    "source_hash": "abc123",
    "personalized": false,
    "personalization_context": null
  },
  "paragraph": {
    "depth": "paragraph",
    "content": "Paragraph summary (~200 tokens)",
    "token_count": 195,
    "generated_at": 1705680000000,
    "source_hash": "abc123",
    "personalized": true,
    "personalization_context": "user:alice:preferences"
  },
  "detailed": {
    "depth": "detailed",
    "content": "Detailed summary (~500 tokens)",
    "token_count": 487,
    "generated_at": 1705680000000,
    "source_hash": "abc123",
    "personalized": false,
    "personalization_context": null
  }
}
```

## Migration 008: Hindsight Learning Tables

### Purpose
Adds tables for error signature capture, resolution tracking, and hindsight notes for the Hindsight Learning component.

### New Tables

#### error_signatures
Stores normalized error patterns for semantic matching:

```sql
CREATE TABLE error_signatures (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    error_type TEXT NOT NULL,           -- e.g., 'TypeError', 'ConnectionError'
    message_pattern TEXT NOT NULL,      -- Normalized error message pattern
    stack_patterns JSONB DEFAULT '[]',  -- Array of stack trace patterns
    context_patterns JSONB DEFAULT '[]',-- Array of context patterns
    embedding JSONB,                    -- Semantic embedding for similarity search
    occurrence_count INTEGER DEFAULT 1,
    first_seen_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (tenant_id) REFERENCES organizational_units(id)
);
```

#### resolutions
Stores successful fix patterns linked to error signatures:

```sql
CREATE TABLE resolutions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    error_signature_id TEXT NOT NULL,
    description TEXT NOT NULL,          -- Human-readable resolution description
    changes JSONB DEFAULT '[]',         -- Array of code changes that fixed the error
    success_rate REAL DEFAULT 0.0,      -- 0.0 to 1.0
    application_count INTEGER DEFAULT 0,
    last_success_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (error_signature_id) REFERENCES error_signatures(id) ON DELETE CASCADE
);
```

#### hindsight_notes
Stores distilled learnings from error/resolution pairs:

```sql
CREATE TABLE hindsight_notes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    error_signature_id TEXT NOT NULL,
    content TEXT NOT NULL,              -- Markdown content of the note
    tags JSONB DEFAULT '[]',            -- Array of tags for filtering
    resolution_ids JSONB DEFAULT '[]',  -- Array of resolution IDs referenced
    quality_score REAL,                 -- Quality score from distillation
    promoted_to_layer TEXT,             -- Layer if promoted (e.g., 'team', 'org')
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (error_signature_id) REFERENCES error_signatures(id) ON DELETE CASCADE
);
```

### Indexes

```sql
-- Error signature lookups
CREATE INDEX idx_error_signatures_tenant ON error_signatures(tenant_id);
CREATE INDEX idx_error_signatures_type ON error_signatures(tenant_id, error_type);
CREATE INDEX idx_error_signatures_last_seen ON error_signatures(tenant_id, last_seen_at DESC);

-- Resolution lookups
CREATE INDEX idx_resolutions_error_signature ON resolutions(error_signature_id);
CREATE INDEX idx_resolutions_success_rate ON resolutions(tenant_id, success_rate DESC);
CREATE INDEX idx_resolutions_application_count ON resolutions(tenant_id, application_count DESC);

-- Hindsight note lookups
CREATE INDEX idx_hindsight_notes_tenant ON hindsight_notes(tenant_id);
CREATE INDEX idx_hindsight_notes_error_signature ON hindsight_notes(error_signature_id);
CREATE INDEX idx_hindsight_notes_quality ON hindsight_notes(tenant_id, quality_score DESC) WHERE quality_score IS NOT NULL;
CREATE INDEX idx_hindsight_notes_tags ON hindsight_notes USING GIN (tags);
```

### Row-Level Security (RLS)

All new tables have RLS enabled with tenant isolation policies:

```sql
-- Enable RLS
ALTER TABLE error_signatures ENABLE ROW LEVEL SECURITY;
ALTER TABLE resolutions ENABLE ROW LEVEL SECURITY;
ALTER TABLE hindsight_notes ENABLE ROW LEVEL SECURITY;

-- Policies ensure tenant isolation via app.tenant_id session variable
CREATE POLICY error_signatures_tenant_isolation ON error_signatures
    USING (tenant_id = current_setting('app.tenant_id', true));
```

## Running Migrations

### Using sqlx-cli

```bash
# Install sqlx-cli if needed
cargo install sqlx-cli

# Run pending migrations
sqlx migrate run --source storage/migrations

# Verify migration status
sqlx migrate info --source storage/migrations
```

### Manual Execution

```bash
# Migration 007
psql -d aeterna -f storage/migrations/007_cca_summaries.sql

# Migration 008
psql -d aeterna -f storage/migrations/008_hindsight_tables.sql

# Verify
psql -d aeterna -c "\dt *summary*"
psql -d aeterna -c "\dt *error*"
psql -d aeterna -c "\dt *resolution*"
psql -d aeterna -c "\dt *hindsight*"
```

## Post-Migration Verification

```sql
-- Verify new columns on memory_entries
SELECT column_name, data_type, column_default 
FROM information_schema.columns 
WHERE table_name = 'memory_entries' 
AND column_name IN ('summaries', 'context_vector', 'summary_updated_at');

-- Verify new tables exist
SELECT table_name FROM information_schema.tables 
WHERE table_schema = 'public' 
AND table_name IN ('layer_summary_cache', 'error_signatures', 'resolutions', 'hindsight_notes');

-- Verify RLS is enabled
SELECT tablename, rowsecurity 
FROM pg_tables 
WHERE tablename IN ('error_signatures', 'resolutions', 'hindsight_notes');

-- Verify indexes
SELECT indexname FROM pg_indexes 
WHERE tablename IN ('memory_entries', 'layer_summary_cache', 'error_signatures', 'resolutions', 'hindsight_notes')
ORDER BY indexname;
```

## Data Migration for Existing Deployments

This section provides a comprehensive guide for migrating existing Aeterna deployments to include CCA capabilities.

### Pre-Migration Checklist

Before starting the data migration:

- [ ] **Backup complete**: Full PostgreSQL and Redis backup taken
- [ ] **Downtime scheduled**: Plan 15-30 minutes for large deployments (>100K entries)
- [ ] **LLM budget allocated**: Summary generation requires LLM tokens (~750 tokens per entry)
- [ ] **Redis capacity verified**: Summary cache may increase Redis memory by 2-3x
- [ ] **Application stopped**: Stop all Aeterna instances to prevent race conditions

### Phase 1: Schema Migration

Apply the database migrations first (see sections above):

```bash
# Verify current state
sqlx migrate info --source storage/migrations

# Apply migrations
sqlx migrate run --source storage/migrations

# Verify
psql -d aeterna -c "\d memory_entries" | grep -E "(summaries|context_vector|summary_updated_at)"
```

### Phase 2: Memory Entry Summary Population

Existing memory entries need summaries generated. This should be done in batches to manage LLM costs.

#### Option A: Immediate Full Migration

For smaller deployments (<10K entries), populate all summaries immediately:

```rust
use knowledge::context_architect::{SummaryGenerator, SummaryConfig};
use aeterna_storage::PostgresStorage;

async fn migrate_all_summaries(
    storage: &PostgresStorage,
    llm_client: &LlmClient,
    tenant_id: &str,
) -> Result<MigrationStats> {
    let generator = SummaryGenerator::new(llm_client.clone());
    let mut stats = MigrationStats::default();
    
    // Process in batches of 100
    let mut offset = 0;
    loop {
        let entries = storage.query(
            "SELECT id, content FROM memory_entries 
             WHERE tenant_id = $1 AND (summaries = '{}' OR summaries IS NULL)
             ORDER BY created_at DESC
             LIMIT 100 OFFSET $2",
            &[tenant_id, &offset]
        ).await?;
        
        if entries.is_empty() {
            break;
        }
        
        for entry in entries {
            match generator.generate_all_depths(&entry.content, None).await {
                Ok(summaries) => {
                    storage.update_summaries(&entry.id, &summaries).await?;
                    stats.migrated += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to generate summary for {}: {}", entry.id, e);
                    stats.failed += 1;
                }
            }
        }
        
        offset += 100;
        stats.processed += entries.len();
        
        // Rate limit to avoid LLM throttling
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    Ok(stats)
}
```

#### Option B: Lazy Migration (Recommended for Large Deployments)

For larger deployments, generate summaries on-demand:

```rust
// Configure lazy migration in config/aeterna.toml
[cca.context_architect]
lazy_migration = true
lazy_migration_batch_size = 50
lazy_migration_interval_seconds = 60
```

With lazy migration enabled:
1. Summaries are generated when entries are accessed
2. A background job processes old entries during low-traffic periods
3. The system gracefully degrades (returns full content) when summaries don't exist

#### Option C: Prioritized Migration

Migrate high-value entries first:

```sql
-- Create priority view for migration
CREATE OR REPLACE VIEW migration_priority AS
SELECT 
    id,
    content,
    memory_layer,
    -- Priority score: recent + frequently accessed + higher layers
    (CASE memory_layer 
        WHEN 'tenant' THEN 100
        WHEN 'org' THEN 80
        WHEN 'team' THEN 60
        WHEN 'project' THEN 40
        WHEN 'session' THEN 20
        WHEN 'user' THEN 10
        WHEN 'agent' THEN 5
    END) +
    (EXTRACT(EPOCH FROM NOW()) - created_at) / 86400 * -1 + -- Recency bonus
    COALESCE(access_count, 0) * 2 -- Access frequency bonus
    AS priority_score
FROM memory_entries
WHERE summaries = '{}' OR summaries IS NULL
ORDER BY priority_score DESC;

-- Migrate top 1000 priority entries
SELECT id, content FROM migration_priority LIMIT 1000;
```

### Phase 3: Context Vector Population

Context vectors enable semantic similarity matching. Generate them alongside summaries:

```rust
use knowledge::context_architect::ContextVectorGenerator;

async fn populate_context_vectors(
    storage: &PostgresStorage,
    embedding_client: &EmbeddingClient,
    tenant_id: &str,
) -> Result<()> {
    let generator = ContextVectorGenerator::new(embedding_client.clone());
    
    let entries = storage.query(
        "SELECT id, content FROM memory_entries 
         WHERE tenant_id = $1 AND context_vector IS NULL
         LIMIT 1000",
        &[tenant_id]
    ).await?;
    
    // Batch embedding for efficiency (up to 100 at a time)
    for chunk in entries.chunks(100) {
        let contents: Vec<_> = chunk.iter().map(|e| e.content.as_str()).collect();
        let vectors = generator.generate_batch(&contents).await?;
        
        for (entry, vector) in chunk.iter().zip(vectors) {
            storage.update_context_vector(&entry.id, &vector).await?;
        }
    }
    
    Ok(())
}
```

### Phase 4: Error Signature Migration

If you have existing error logs, import them into the hindsight system:

```rust
use knowledge::hindsight::{ErrorCapture, ErrorContext};

async fn import_existing_errors(
    error_capture: &ErrorCapture,
    error_logs: &[ExistingErrorLog],
) -> Result<ImportStats> {
    let mut stats = ImportStats::default();
    
    for log in error_logs {
        let context = ErrorContext {
            file_path: log.file_path.clone(),
            function_name: log.function_name.clone(),
            tool_name: log.tool_name.clone(),
            timestamp: log.timestamp,
        };
        
        match error_capture.capture_from_log(&log.message, &log.stack_trace, &context).await {
            Ok(signature_id) => {
                stats.imported += 1;
                
                // If there's a known resolution, import that too
                if let Some(resolution) = &log.resolution {
                    error_capture.record_resolution(&signature_id, resolution).await?;
                    stats.resolutions += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to import error: {}", e);
                stats.failed += 1;
            }
        }
    }
    
    Ok(stats)
}
```

### Phase 5: Redis Schema Population

Warm the Redis cache after PostgreSQL migration:

```rust
use aeterna_storage::RedisStorage;

async fn warm_redis_cache(
    postgres: &PostgresStorage,
    redis: &RedisStorage,
    tenant_id: &str,
) -> Result<()> {
    // Cache recently accessed summaries
    let recent_entries = postgres.query(
        "SELECT id, memory_layer, summaries FROM memory_entries
         WHERE tenant_id = $1 AND summaries != '{}'
         ORDER BY updated_at DESC
         LIMIT 1000",
        &[tenant_id]
    ).await?;
    
    for entry in recent_entries {
        for (depth, summary) in entry.summaries.iter() {
            let key = format!("summary:{}:{}:{}:{}", tenant_id, entry.memory_layer, entry.id, depth);
            redis.set_with_ttl(&key, &summary, Duration::from_secs(3600)).await?;
        }
    }
    
    Ok(())
}
```

### Migration Monitoring

Track migration progress with these queries:

```sql
-- Overall migration progress
SELECT 
    COUNT(*) FILTER (WHERE summaries != '{}') as migrated,
    COUNT(*) FILTER (WHERE summaries = '{}' OR summaries IS NULL) as pending,
    COUNT(*) as total,
    ROUND(100.0 * COUNT(*) FILTER (WHERE summaries != '{}') / COUNT(*), 2) as percent_complete
FROM memory_entries
WHERE tenant_id = 'your-tenant';

-- Migration progress by layer
SELECT 
    memory_layer,
    COUNT(*) FILTER (WHERE summaries != '{}') as migrated,
    COUNT(*) FILTER (WHERE summaries = '{}') as pending
FROM memory_entries
WHERE tenant_id = 'your-tenant'
GROUP BY memory_layer
ORDER BY memory_layer;

-- Context vector population progress
SELECT 
    COUNT(*) FILTER (WHERE context_vector IS NOT NULL) as with_vector,
    COUNT(*) FILTER (WHERE context_vector IS NULL) as without_vector,
    COUNT(*) as total
FROM memory_entries
WHERE tenant_id = 'your-tenant';

-- Hindsight import progress
SELECT 
    COUNT(*) as total_signatures,
    SUM(occurrence_count) as total_occurrences,
    COUNT(*) FILTER (WHERE (SELECT COUNT(*) FROM resolutions r WHERE r.error_signature_id = error_signatures.id) > 0) as with_resolutions
FROM error_signatures
WHERE tenant_id = 'your-tenant';
```

### Estimated Migration Times

| Entries | Full Migration | Lazy Migration Warm-up |
|---------|----------------|------------------------|
| 1,000 | ~5 minutes | Instant |
| 10,000 | ~45 minutes | ~2 hours background |
| 100,000 | ~8 hours | ~24 hours background |
| 1,000,000 | ~80 hours | ~7 days background |

### LLM Token Cost Estimates

| Summary Type | Tokens/Entry | Cost (GPT-4o) |
|--------------|--------------|---------------|
| Sentence | ~100 | ~$0.0005 |
| Paragraph | ~300 | ~$0.0015 |
| Detailed | ~600 | ~$0.003 |
| All depths | ~1000 | ~$0.005 |

For 100K entries with all depths: ~$500 LLM cost.

### Post-Migration Validation

```sql
-- Verify summary quality (check for empty or truncated summaries)
SELECT id, memory_layer, 
       LENGTH(summaries::text) as summary_size,
       jsonb_object_keys(summaries) as depths
FROM memory_entries
WHERE tenant_id = 'your-tenant' 
  AND summaries != '{}'
LIMIT 10;

-- Verify context vectors are correct dimension
SELECT id, array_length(context_vector, 1) as vector_dim
FROM memory_entries
WHERE context_vector IS NOT NULL
LIMIT 10;

-- Verify hindsight relationships
SELECT 
    e.id as error_id,
    e.error_type,
    COUNT(r.id) as resolution_count,
    COUNT(h.id) as note_count
FROM error_signatures e
LEFT JOIN resolutions r ON r.error_signature_id = e.id
LEFT JOIN hindsight_notes h ON h.error_signature_id = e.id
WHERE e.tenant_id = 'your-tenant'
GROUP BY e.id, e.error_type
LIMIT 10;
```

### Rollback Procedure

If migration fails or causes issues, see the [Rollback Procedures](#rollback-procedures) section below.

## Performance Considerations

### Index Maintenance

The new GIN index on `hindsight_notes.tags` requires periodic maintenance:

```sql
-- Reindex periodically for GIN indexes
REINDEX INDEX CONCURRENTLY idx_hindsight_notes_tags;

-- Analyze tables after bulk operations
ANALYZE memory_entries;
ANALYZE layer_summary_cache;
ANALYZE error_signatures;
ANALYZE resolutions;
ANALYZE hindsight_notes;
```

### Storage Estimates

| Table | Row Size (avg) | Growth Rate |
|-------|----------------|-------------|
| layer_summary_cache | ~2KB | 3x memory_entries |
| error_signatures | ~1KB | Per unique error type |
| resolutions | ~500B | Per successful fix |
| hindsight_notes | ~2KB | Per distilled note |

### Cleanup Jobs

Configure periodic cleanup for expired cache entries:

```sql
-- Delete expired summary cache entries (run daily)
DELETE FROM layer_summary_cache 
WHERE expires_at IS NOT NULL 
AND expires_at < EXTRACT(EPOCH FROM NOW()) * 1000;
```

## Troubleshooting

### Migration Fails: Vector Extension Missing

```
ERROR: type "vector" does not exist
```

Solution:
```sql
CREATE EXTENSION IF NOT EXISTS vector;
```

### Migration Fails: Foreign Key Violation

```
ERROR: insert or update on table violates foreign key constraint
```

Solution: Ensure `organizational_units` table has all required tenant IDs before migration.

### RLS Blocking Access

```
ERROR: new row violates row-level security policy
```

Solution: Set the tenant context before operations:
```sql
SET app.tenant_id = 'your-tenant-id';
```

## Rollback Procedures

If CCA migrations cause issues, use these procedures to safely roll back.

### Important: Rollback Order

**Always rollback in reverse order**: 008 first, then 007.

Migration 008 has foreign keys to tables created in 007, so it must be removed first.

### Pre-Rollback Checklist

- [ ] **Stop all Aeterna instances** to prevent data corruption
- [ ] **Backup current state** including CCA data you may want to preserve
- [ ] **Document any manually-added data** in CCA tables
- [ ] **Notify dependent services** of temporary CCA unavailability

### Rollback Migration 008 (Hindsight Tables)

#### Step 1: Backup Hindsight Data (Optional)

If you want to preserve hindsight data for re-migration later:

```sql
-- Export error signatures
COPY (SELECT * FROM error_signatures WHERE tenant_id = 'your-tenant')
TO '/tmp/backup_error_signatures.csv' WITH CSV HEADER;

-- Export resolutions
COPY (SELECT * FROM resolutions WHERE tenant_id = 'your-tenant')
TO '/tmp/backup_resolutions.csv' WITH CSV HEADER;

-- Export hindsight notes
COPY (SELECT * FROM hindsight_notes WHERE tenant_id = 'your-tenant')
TO '/tmp/backup_hindsight_notes.csv' WITH CSV HEADER;
```

#### Step 2: Drop RLS Policies

```sql
-- Drop RLS policies for hindsight_notes
DROP POLICY IF EXISTS hindsight_notes_delete_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_update_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_insert_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_tenant_isolation ON hindsight_notes;

-- Drop RLS policies for resolutions
DROP POLICY IF EXISTS resolutions_delete_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_update_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_insert_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_tenant_isolation ON resolutions;

-- Drop RLS policies for error_signatures
DROP POLICY IF EXISTS error_signatures_delete_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_update_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_insert_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_tenant_isolation ON error_signatures;
```

#### Step 3: Drop Indexes

```sql
-- Hindsight notes indexes
DROP INDEX IF EXISTS idx_hindsight_notes_tags;
DROP INDEX IF EXISTS idx_hindsight_notes_quality;
DROP INDEX IF EXISTS idx_hindsight_notes_error_signature;
DROP INDEX IF EXISTS idx_hindsight_notes_tenant;

-- Resolutions indexes
DROP INDEX IF EXISTS idx_resolutions_application_count;
DROP INDEX IF EXISTS idx_resolutions_success_rate;
DROP INDEX IF EXISTS idx_resolutions_error_signature;

-- Error signatures indexes
DROP INDEX IF EXISTS idx_error_signatures_last_seen;
DROP INDEX IF EXISTS idx_error_signatures_type;
DROP INDEX IF EXISTS idx_error_signatures_tenant;
```

#### Step 4: Drop Tables

```sql
-- Drop in order respecting foreign keys
DROP TABLE IF EXISTS hindsight_notes CASCADE;
DROP TABLE IF EXISTS resolutions CASCADE;
DROP TABLE IF EXISTS error_signatures CASCADE;
```

#### Step 5: Update Migration Tracking

```sql
-- Remove migration 008 from tracking (if using sqlx)
DELETE FROM _sqlx_migrations WHERE version = 8;

-- Or if using custom tracking table
DELETE FROM schema_migrations WHERE version = '008';
```

### Rollback Migration 007 (Summary Schema)

#### Step 1: Backup Summary Data (Optional)

```sql
-- Export summaries
COPY (
    SELECT id, summaries, context_vector, summary_updated_at 
    FROM memory_entries 
    WHERE summaries != '{}' AND tenant_id = 'your-tenant'
) TO '/tmp/backup_memory_summaries.csv' WITH CSV HEADER;

-- Export layer_summary_cache
COPY (SELECT * FROM layer_summary_cache WHERE tenant_id = 'your-tenant')
TO '/tmp/backup_layer_summary_cache.csv' WITH CSV HEADER;

-- Export knowledge_items summaries
COPY (
    SELECT id, summaries 
    FROM knowledge_items 
    WHERE summaries != '{}' AND tenant_id = 'your-tenant'
) TO '/tmp/backup_knowledge_summaries.csv' WITH CSV HEADER;
```

#### Step 2: Drop Indexes

```sql
DROP INDEX IF EXISTS idx_layer_summary_cache_expiry;
DROP INDEX IF EXISTS idx_layer_summary_cache_lookup;
DROP INDEX IF EXISTS idx_memory_entries_summary_updated;
```

#### Step 3: Drop New Table

```sql
DROP TABLE IF EXISTS layer_summary_cache CASCADE;
```

#### Step 4: Remove Columns from Existing Tables

```sql
-- Remove summary columns from memory_entries
ALTER TABLE memory_entries DROP COLUMN IF EXISTS summaries;
ALTER TABLE memory_entries DROP COLUMN IF EXISTS context_vector;
ALTER TABLE memory_entries DROP COLUMN IF EXISTS summary_updated_at;

-- Remove summary column from knowledge_items
ALTER TABLE knowledge_items DROP COLUMN IF EXISTS summaries;
```

#### Step 5: Update Migration Tracking

```sql
-- Remove migration 007 from tracking (if using sqlx)
DELETE FROM _sqlx_migrations WHERE version = 7;

-- Or if using custom tracking table
DELETE FROM schema_migrations WHERE version = '007';
```

### Complete Rollback Script

For convenience, here's a complete script to rollback both migrations:

```sql
-- ============================================
-- CCA COMPLETE ROLLBACK SCRIPT
-- Run this to fully remove CCA schema changes
-- ============================================

BEGIN;

-- === Migration 008 Rollback ===

-- Drop RLS policies
DROP POLICY IF EXISTS hindsight_notes_delete_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_update_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_insert_policy ON hindsight_notes;
DROP POLICY IF EXISTS hindsight_notes_tenant_isolation ON hindsight_notes;
DROP POLICY IF EXISTS resolutions_delete_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_update_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_insert_policy ON resolutions;
DROP POLICY IF EXISTS resolutions_tenant_isolation ON resolutions;
DROP POLICY IF EXISTS error_signatures_delete_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_update_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_insert_policy ON error_signatures;
DROP POLICY IF EXISTS error_signatures_tenant_isolation ON error_signatures;

-- Drop hindsight indexes
DROP INDEX IF EXISTS idx_hindsight_notes_tags;
DROP INDEX IF EXISTS idx_hindsight_notes_quality;
DROP INDEX IF EXISTS idx_hindsight_notes_error_signature;
DROP INDEX IF EXISTS idx_hindsight_notes_tenant;
DROP INDEX IF EXISTS idx_resolutions_application_count;
DROP INDEX IF EXISTS idx_resolutions_success_rate;
DROP INDEX IF EXISTS idx_resolutions_error_signature;
DROP INDEX IF EXISTS idx_error_signatures_last_seen;
DROP INDEX IF EXISTS idx_error_signatures_type;
DROP INDEX IF EXISTS idx_error_signatures_tenant;

-- Drop hindsight tables
DROP TABLE IF EXISTS hindsight_notes CASCADE;
DROP TABLE IF EXISTS resolutions CASCADE;
DROP TABLE IF EXISTS error_signatures CASCADE;

-- === Migration 007 Rollback ===

-- Drop summary indexes
DROP INDEX IF EXISTS idx_layer_summary_cache_expiry;
DROP INDEX IF EXISTS idx_layer_summary_cache_lookup;
DROP INDEX IF EXISTS idx_memory_entries_summary_updated;

-- Drop summary cache table
DROP TABLE IF EXISTS layer_summary_cache CASCADE;

-- Remove summary columns
ALTER TABLE memory_entries DROP COLUMN IF EXISTS summaries;
ALTER TABLE memory_entries DROP COLUMN IF EXISTS context_vector;
ALTER TABLE memory_entries DROP COLUMN IF EXISTS summary_updated_at;
ALTER TABLE knowledge_items DROP COLUMN IF EXISTS summaries;

-- Update migration tracking
DELETE FROM _sqlx_migrations WHERE version IN (7, 8);

COMMIT;

-- Verify rollback
SELECT column_name FROM information_schema.columns 
WHERE table_name = 'memory_entries' 
AND column_name IN ('summaries', 'context_vector', 'summary_updated_at');
-- Should return 0 rows

SELECT table_name FROM information_schema.tables 
WHERE table_name IN ('layer_summary_cache', 'error_signatures', 'resolutions', 'hindsight_notes');
-- Should return 0 rows
```

### Redis Cleanup After Rollback

After rolling back PostgreSQL, clean up Redis CCA keys:

```bash
# Connect to Redis
redis-cli

# Delete all summary cache keys
SCAN 0 MATCH "summary:*" COUNT 1000
# Then DEL each key, or use redis-cli with pattern:
redis-cli --scan --pattern "summary:*" | xargs redis-cli DEL

# Delete extension state keys
redis-cli --scan --pattern "ext_state:*" | xargs redis-cli DEL

# Delete CCA-related lock keys
redis-cli --scan --pattern "cca:*" | xargs redis-cli DEL

# Delete summarization budget keys
redis-cli --scan --pattern "budget:summarization:*" | xargs redis-cli DEL
```

Or use this Lua script for atomic cleanup:

```lua
-- cleanup_cca_redis.lua
local patterns = {'summary:*', 'ext_state:*', 'cca:*', 'budget:summarization:*'}
local deleted = 0

for _, pattern in ipairs(patterns) do
    local cursor = '0'
    repeat
        local result = redis.call('SCAN', cursor, 'MATCH', pattern, 'COUNT', 1000)
        cursor = result[1]
        local keys = result[2]
        if #keys > 0 then
            redis.call('DEL', unpack(keys))
            deleted = deleted + #keys
        end
    until cursor == '0'
end

return deleted
```

Run with:
```bash
redis-cli --eval cleanup_cca_redis.lua
```

### Post-Rollback Verification

```sql
-- Verify no CCA tables remain
SELECT table_name FROM information_schema.tables 
WHERE table_schema = 'public' 
AND table_name IN ('layer_summary_cache', 'error_signatures', 'resolutions', 'hindsight_notes');
-- Expected: 0 rows

-- Verify no CCA columns remain on memory_entries
SELECT column_name FROM information_schema.columns 
WHERE table_name = 'memory_entries' 
AND column_name IN ('summaries', 'context_vector', 'summary_updated_at');
-- Expected: 0 rows

-- Verify no CCA columns remain on knowledge_items
SELECT column_name FROM information_schema.columns 
WHERE table_name = 'knowledge_items' 
AND column_name = 'summaries';
-- Expected: 0 rows

-- Verify migrations removed from tracking
SELECT * FROM _sqlx_migrations WHERE version IN (7, 8);
-- Expected: 0 rows
```

### Re-applying Migrations After Rollback

If you need to re-apply CCA migrations after fixing issues:

```bash
# Ensure migrations are in proper state
sqlx migrate info --source storage/migrations

# Re-run migrations
sqlx migrate run --source storage/migrations

# Verify
psql -d aeterna -c "\d+ memory_entries" | grep summaries
```

### Emergency Rollback (Minimal Downtime)

For production emergencies, use this faster approach:

```sql
-- Disable CCA at application level first (via config)
-- Then run minimal rollback:

BEGIN;
-- Just disable CCA functionality without data loss
ALTER TABLE memory_entries ALTER COLUMN summaries SET DEFAULT '{}';
ALTER TABLE memory_entries ALTER COLUMN context_vector DROP NOT NULL;

-- Disable triggers if any
-- ALTER TABLE memory_entries DISABLE TRIGGER cca_summary_trigger;

COMMIT;
```

This preserves CCA data but disables functionality, allowing quick recovery while you investigate.

## Next Steps

After successful migration:

1. Configure CCA in `config/aeterna.toml` (see [Configuration Guide](configuration.md))
2. Verify Redis schema is configured (see [Redis Schema](redis-schema.md))
3. Test CCA tools via MCP interface (see [API Reference](api-reference.md))
