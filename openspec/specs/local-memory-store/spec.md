# local-memory-store Specification

## Purpose
The Local Memory Store provides an embedded SQLite-based storage layer for personal-layer memories (agent, user, session) with offline resilience, sub-millisecond reads, and bidirectional sync to the remote Aeterna server.
## Requirements
### Requirement: Local Memory Store Initialization
The system SHALL initialize an embedded SQLite database compatible with the OpenCode runtime on plugin startup at the configured path (default `~/.aeterna/local.db`).

#### Scenario: First-time initialization
- **WHEN** the plugin starts and no local database file exists
- **THEN** the system SHALL create the database file with WAL journal mode
- **AND** the system SHALL create the `memories`, `sync_queue`, and `sync_cursors` tables
- **AND** the system SHALL complete initialization in under 50ms

#### Scenario: Existing database
- **WHEN** the plugin starts and a local database file already exists
- **THEN** the system SHALL open the database and verify schema version
- **AND** the system SHALL apply any pending schema migrations idempotently

#### Scenario: Disabled local store
- **WHEN** the plugin config sets `local.enabled = false`
- **THEN** the system SHALL NOT initialize the local database
- **AND** all memory operations SHALL route to the remote server exclusively

### Requirement: Local Memory Write
The system SHALL write agent, user, and session layer memories directly to the local SQLite store without requiring network connectivity.

#### Scenario: Add memory to local layer
- **WHEN** a memory is added with layer `agent`, `user`, or `session`
- **THEN** the system SHALL insert the memory into the local `memories` table with `ownership = 'local'`
- **AND** the system SHALL enqueue the operation in the `sync_queue` table
- **AND** the write SHALL complete in under 5ms

#### Scenario: Update local memory
- **WHEN** a local-owned memory is updated
- **THEN** the system SHALL update the memory in the local `memories` table
- **AND** the system SHALL update `updated_at` to the current timestamp
- **AND** the system SHALL enqueue an `upsert` operation in the `sync_queue`

#### Scenario: Delete local memory
- **WHEN** a local-owned memory is deleted
- **THEN** the system SHALL set `deleted_at` on the memory (soft delete)
- **AND** the system SHALL enqueue a `delete` operation in the `sync_queue`

### Requirement: Local Memory Read
The system SHALL serve reads for personal layers (agent, user, session) from the local store with sub-millisecond latency.

#### Scenario: Search local memories
- **WHEN** a memory search targets layer `agent`, `user`, or `session`
- **AND** embeddings are available for the query and stored memories
- **THEN** the system SHALL compute cosine similarity across local embeddings
- **AND** the system SHALL return results sorted by similarity score
- **AND** the search SHALL complete in under 10ms for up to 10,000 memories

#### Scenario: Search without embeddings
- **WHEN** a memory search targets a personal layer
- **AND** the query or stored memories lack embeddings
- **THEN** the system SHALL fall back to text `LIKE` matching on content
- **AND** the system SHALL return results sorted by `updated_at` descending

#### Scenario: Get memory by ID
- **WHEN** a memory is requested by ID and exists in the local store
- **THEN** the system SHALL return the memory from the local store
- **AND** the read SHALL complete in under 1ms

### Requirement: Shared Layer Cache
The system SHALL cache remote shared-layer memories (project, team, org, tenant) in the local store for offline access.

#### Scenario: Read cached shared memory
- **WHEN** a memory search targets layer `project`, `team`, `org`, or `tenant`
- **AND** cached entries exist in the local store with `ownership = 'cached'`
- **THEN** the system SHALL return cached results immediately when the cache is still fresh
- **AND** the returned results SHALL indicate that they came from cache

#### Scenario: Cache miss with server available
- **WHEN** a shared-layer search has no cached results
- **AND** the remote server is reachable
- **THEN** the system SHALL fetch from the remote server
- **AND** the system SHALL cache the results locally with `ownership = 'cached'`

#### Scenario: Cache miss with server unavailable
- **WHEN** a shared-layer search has no cached results
- **AND** the remote server is unreachable
- **THEN** the system SHALL return an empty result set (not an error)
- **AND** the system SHALL log a warning about offline state

#### Scenario: Stale cache fallback
- **WHEN** a shared-layer search has cached results that are older than the fresh-cache threshold
- **AND** the remote server request fails
- **THEN** the system SHALL return the stale cached results rather than failing the search
- **AND** the returned results SHALL indicate that they may be stale

#### Scenario: Cache eviction
- **WHEN** the number of cached shared-layer entries exceeds `local.max_cached_entries` (default 50,000)
- **THEN** the system SHALL evict the oldest entries by `cached_at` timestamp

### Requirement: Offline Resilience
The system SHALL function for personal layers without any network connectivity.

#### Scenario: Fully offline operation
- **WHEN** the remote Aeterna server is unreachable
- **THEN** memory add, search, get, update, and delete operations for agent/user/session layers SHALL succeed using the local store
- **AND** changes SHALL accumulate in the `sync_queue` for later push

#### Scenario: Network recovery
- **WHEN** the remote server becomes reachable after an offline period
- **THEN** the `SyncEngine` SHALL push all queued changes on the next sync cycle
- **AND** the system SHALL pull any shared layer updates missed during the offline period

#### Scenario: Offline search provenance is visible
- **WHEN** the plugin returns local, cached, or remote memory results
- **THEN** the returned results SHALL include source metadata identifying whether the result came from local storage, cache, or remote retrieval
- **AND** stale cached results SHALL be identifiable to the user-facing tool output

### Requirement: Session Memory Expiration
The system SHALL automatically expire session-layer memories from the local store after a configurable TTL.

#### Scenario: Expire old session memories
- **WHEN** the `SyncEngine` runs a maintenance cycle
- **AND** session memories exist with `created_at` older than `session.storage_ttl_hours` (default 24h)
- **THEN** the system SHALL delete expired session memories from the local store
- **AND** the system SHALL NOT delete them from the remote server (remote manages its own retention)

### Requirement: Local Store Configuration
The system SHALL support configuration via environment variables and `.aeterna/config.toml`.

#### Scenario: Default configuration
- **WHEN** no local store configuration is provided
- **THEN** the system SHALL use defaults: `enabled = true`, `db_path = ~/.aeterna/local.db`, `sync_push_interval_ms = 30000`, `sync_pull_interval_ms = 60000`, `max_cached_entries = 50000`

#### Scenario: Environment variable override
- **WHEN** `AETERNA_LOCAL_DB_PATH` is set
- **THEN** the system SHALL use that path for the local database file
- **AND** the system SHALL create parent directories if they do not exist

