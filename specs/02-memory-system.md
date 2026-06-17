---
title: Memory System Specification
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 01-core-concepts.md
  - 04-memory-knowledge-sync.md
  - 05-adapter-architecture.md
---

# Memory System Specification

This document specifies the Memory System component: a hierarchical, provider-agnostic semantic memory store for AI agents.

## Table of Contents

1. [Overview](#overview)
2. [Layer Hierarchy](#layer-hierarchy)
3. [Memory Entry Schema](#memory-entry-schema)
4. [Core Operations](#core-operations)
5. [Layer Resolution](#layer-resolution)
6. [Provider Adapter Interface](#provider-adapter-interface)
7. [Memory Lifecycle](#memory-lifecycle)
8. [Error Handling](#error-handling)

---

## Overview

The Memory System provides:

- **Semantic storage**: Vector-based content for similarity search
- **Hierarchical scoping**: 7 layers from agent-specific to organization-wide
- **Provider abstraction**: Swap backends without code changes
- **Flexible retrieval**: Query across layers with precedence rules

```
┌─────────────────────────────────────────────────────────────────┐
│                      MEMORY SYSTEM                               │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   Memory Manager                         │    │
│  │  • Coordinates all memory operations                     │    │
│  │  • Enforces layer rules                                  │    │
│  │  • Routes to provider adapter                            │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   Layer Resolver                         │    │
│  │  • Determines target layers for operations               │    │
│  │  • Applies precedence rules                              │    │
│  │  • Merges results from multiple layers                   │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  Provider Adapter                        │    │
│  │  • Translates to provider-specific API                   │    │
│  │  • Handles connection, auth, retries                     │    │
│  │  • Manages embedding generation                          │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │              Provider (Mem0, Letta, etc.)                │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Layer Hierarchy

### The Seven Layers

Memory is organized into seven hierarchical layers, from most specific to least specific:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  LAYER          SCOPE                    EXAMPLES                │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  agent    ←── Per-agent instance         Agent-specific learnings│
│    │          (most specific)            Tool preferences        │
│    │                                                             │
│  user          Per-user                  User preferences        │
│    │                                     Communication style     │
│    │                                                             │
│  session       Per-session               Current task context    │
│    │           (conversation)            Recent decisions        │
│    │                                                             │
│  project       Per-project/repo          Project conventions     │
│    │                                     Tech stack choices      │
│    │                                                             │
│  team          Per-team                  Team standards          │
│    │                                     Shared knowledge        │
│    │                                                             │
│  org           Per-organization          Org-wide policies       │
│    │                                     Compliance rules        │
│    │                                                             │
│  tenant  ←── Per-tenant         Tenant standards       │
│               (least specific)           Global policies         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Layer Identifiers

Each layer requires specific identifiers to scope memory:

```typescript
interface LayerIdentifiers {
  /** Required for agent layer */
  agentId?: string;
  
  /** Required for user layer and below */
  userId?: string;
  
  /** Required for session layer and below */
  sessionId?: string;
  
  /** Required for project layer and below */
  projectId?: string;
  
  /** Required for team layer and below */
  teamId?: string;
  
  /** Required for org layer and below */
  orgId?: string;
  
  /** Required for tenant layer */
  companyId?: string;
}
```

### Layer Requirements Matrix

| Layer | agentId | userId | sessionId | projectId | teamId | orgId | companyId |
|-------|---------|--------|-----------|-----------|--------|-------|-----------|
| agent | ✓ | ✓ | - | - | - | - | - |
| user | - | ✓ | - | - | - | - | - |
| session | - | ✓ | ✓ | - | - | - | - |
| project | - | - | - | ✓ | - | - | - |
| team | - | - | - | - | ✓ | - | - |
| org | - | - | - | - | - | ✓ | - |
| tenant | - | - | - | - | - | - | ✓ |

---

## Memory Entry Schema

### Core Schema

```typescript
/**
 * A single memory entry in the system.
 */
interface MemoryEntry {
  /** Unique identifier (provider-generated or UUID) */
  id: string;
  
  /** The memory content (human-readable text) */
  content: string;
  
  /** Layer this memory belongs to */
  layer: MemoryLayer;
  
  /** Layer-specific identifiers */
  identifiers: LayerIdentifiers;
  
  /** Arbitrary metadata */
  metadata: MemoryMetadata;
  
  /** Creation timestamp (ISO 8601) */
  createdAt: string;
  
  /** Last update timestamp (ISO 8601) */
  updatedAt: string;
  
  /** Vector embedding (provider-specific format) */
  embedding?: number[];
}

type MemoryLayer = 
  | 'agent'
  | 'user'
  | 'session'
  | 'project'
  | 'team'
  | 'org'
  | 'tenant';
```

### Metadata Schema

```typescript
/**
 * Flexible metadata attached to memories.
 */
interface MemoryMetadata {
  /** Optional: Tags for categorization */
  tags?: string[];
  
  /** Optional: Source of this memory */
  source?: MemorySource;
  
  /** Optional: Pointer to knowledge item (see 04-memory-knowledge-sync.md) */
  knowledgePointer?: KnowledgePointer;
  
  /** Optional: Relevance score (0.0 - 1.0) */
  relevance?: number;
  
  /** Optional: Decay factor for aging */
  decayFactor?: number;
  
  /** Custom fields (string keys, JSON-serializable values) */
  [key: string]: unknown;
}

interface MemorySource {
  /** Source type */
  type: 'conversation' | 'tool_result' | 'knowledge_sync' | 'manual' | 'import';
  
  /** Optional: Reference to source (message ID, tool call ID, etc.) */
  reference?: string;
}

interface KnowledgePointer {
  /** Type of knowledge item */
  sourceType: 'adr' | 'policy' | 'pattern' | 'spec';
  
  /** ID of knowledge item */
  sourceId: string;
  
  /** Content hash at sync time */
  contentHash: string;
  
  /** Sync timestamp */
  syncedAt: string;
}
```

### Example Memory Entries

#### Agent-Level Memory

```json
{
  "id": "mem_agent_001",
  "content": "When debugging TypeScript, always check tsconfig.json first",
  "layer": "agent",
  "identifiers": {
    "agentId": "agent_debugger",
    "userId": "user_123"
  },
  "metadata": {
    "tags": ["debugging", "typescript"],
    "source": {
      "type": "conversation",
      "reference": "msg_abc123"
    }
  },
  "createdAt": "2025-01-07T10:30:00Z",
  "updatedAt": "2025-01-07T10:30:00Z"
}
```

#### Project-Level Memory (Knowledge Pointer)

```json
{
  "id": "mem_proj_042",
  "content": "Use PostgreSQL for all new services per ADR-042",
  "layer": "project",
  "identifiers": {
    "projectId": "proj_backend_api"
  },
  "metadata": {
    "tags": ["database", "architecture"],
    "source": {
      "type": "knowledge_sync"
    },
    "knowledgePointer": {
      "sourceType": "adr",
      "sourceId": "adr-042-database-selection",
      "contentHash": "sha256:abc123def456...",
      "syncedAt": "2025-01-07T09:00:00Z"
    }
  },
  "createdAt": "2025-01-07T09:00:00Z",
  "updatedAt": "2025-01-07T09:00:00Z"
}
```

---

## Core Operations

### Operation: Add Memory

Add a new memory entry to a specific layer.

```typescript
interface AddMemoryInput {
  /** Memory content (required) */
  content: string;
  
  /** Target layer (required) */
  layer: MemoryLayer;
  
  /** Layer identifiers (required fields depend on layer) */
  identifiers: LayerIdentifiers;
  
  /** Optional metadata */
  metadata?: Partial<MemoryMetadata>;
}

interface AddMemoryOutput {
  /** Created memory entry */
  memory: MemoryEntry;
  
  /** Whether embedding was generated */
  embeddingGenerated: boolean;
}
```

**Behavior:**

1. Validate `identifiers` contains required fields for `layer`
2. Generate embedding from `content` via provider
3. Persist to provider with layer isolation
4. Return created entry with generated `id`

**Errors:**

| Error | Condition |
|-------|-----------|
| `INVALID_LAYER` | Unknown layer value |
| `MISSING_IDENTIFIER` | Required identifier not provided |
| `CONTENT_TOO_LONG` | Content exceeds provider limit |
| `EMBEDDING_FAILED` | Embedding generation failed |
| `PROVIDER_ERROR` | Provider-specific error |

### Operation: Search Memory

Search for memories semantically matching a query.

```typescript
interface SearchMemoryInput {
  /** Search query (natural language) */
  query: string;
  
  /** Layers to search (default: all accessible layers) */
  layers?: MemoryLayer[];
  
  /** Layer identifiers for scoping */
  identifiers: LayerIdentifiers;
  
  /** Maximum results per layer (default: 10) */
  limit?: number;
  
  /** Minimum similarity threshold (0.0 - 1.0, default: 0.7) */
  threshold?: number;
  
  /** Optional: Filter by metadata */
  filter?: MetadataFilter;
}

interface MetadataFilter {
  /** Match any of these tags */
  tags?: string[];
  
  /** Match source type */
  sourceType?: MemorySource['type'];
  
  /** Only knowledge pointers */
  hasKnowledgePointer?: boolean;
  
  /** Custom field filters */
  custom?: Record<string, unknown>;
}

interface SearchMemoryOutput {
  /** Search results, ordered by relevance */
  results: MemorySearchResult[];
  
  /** Total results before limit */
  totalCount: number;
  
  /** Layers that were searched */
  searchedLayers: MemoryLayer[];
}

interface MemorySearchResult {
  /** The memory entry */
  memory: MemoryEntry;
  
  /** Similarity score (0.0 - 1.0) */
  score: number;
  
  /** Layer this result came from */
  layer: MemoryLayer;
}
```

**Behavior:**

1. Generate embedding for `query`
2. For each layer in `layers`:
   a. Verify `identifiers` provides required fields
   b. Execute vector similarity search
   c. Apply `threshold` filter
   d. Apply `filter` if provided
3. Merge results using layer precedence (see [Layer Resolution](#layer-resolution))
4. Return top `limit` results

**Errors:**

| Error | Condition |
|-------|-----------|
| `INVALID_LAYER` | Unknown layer in `layers` array |
| `MISSING_IDENTIFIER` | Required identifier for layer not provided |
| `QUERY_TOO_LONG` | Query exceeds embedding limit |
| `PROVIDER_ERROR` | Provider-specific error |

### Operation: Get Memory

Retrieve a specific memory by ID.

```typescript
interface GetMemoryInput {
  /** Memory ID */
  id: string;
}

interface GetMemoryOutput {
  /** The memory entry, or null if not found */
  memory: MemoryEntry | null;
}
```

**Behavior:**

1. Look up memory by `id` in provider
2. Return entry or null

### Operation: Update Memory

Update an existing memory's content or metadata.

```typescript
interface UpdateMemoryInput {
  /** Memory ID */
  id: string;
  
  /** New content (optional, triggers re-embedding) */
  content?: string;
  
  /** Metadata updates (merged with existing) */
  metadata?: Partial<MemoryMetadata>;
}

interface UpdateMemoryOutput {
  /** Updated memory entry */
  memory: MemoryEntry;
  
  /** Whether embedding was regenerated */
  embeddingRegenerated: boolean;
}
```

**Behavior:**

1. Fetch existing memory by `id`
2. If `content` changed, regenerate embedding
3. Merge `metadata` with existing (shallow merge)
4. Update `updatedAt` timestamp
5. Persist to provider

**Errors:**

| Error | Condition |
|-------|-----------|
| `MEMORY_NOT_FOUND` | No memory with given ID |
| `CONTENT_TOO_LONG` | New content exceeds limit |
| `EMBEDDING_FAILED` | Re-embedding failed |

### Operation: Delete Memory

Remove a memory from the system.

```typescript
interface DeleteMemoryInput {
  /** Memory ID */
  id: string;
}

interface DeleteMemoryOutput {
  /** Whether deletion succeeded */
  success: boolean;
}
```

**Behavior:**

1. Remove memory from provider
2. Return success status

### Operation: List Memories

List memories in a specific layer with pagination.

```typescript
interface ListMemoriesInput {
  /** Target layer */
  layer: MemoryLayer;
  
  /** Layer identifiers */
  identifiers: LayerIdentifiers;
  
  /** Pagination cursor */
  cursor?: string;
  
  /** Page size (default: 50, max: 100) */
  limit?: number;
  
  /** Optional: Filter by metadata */
  filter?: MetadataFilter;
}

interface ListMemoriesOutput {
  /** Memories in this page */
  memories: MemoryEntry[];
  
  /** Cursor for next page (null if no more) */
  nextCursor: string | null;
  
  /** Total count in layer */
  totalCount: number;
}
```

---

## Layer Resolution

### Precedence Rules

When searching across multiple layers, results are merged using these rules:

```
┌─────────────────────────────────────────────────────────────────┐
│                   LAYER PRECEDENCE                               │
│                                                                  │
│  1. agent    (highest priority - most specific)                 │
│  2. user                                                        │
│  3. session                                                     │
│  4. project                                                     │
│  5. team                                                        │
│  6. org                                                         │
│  7. tenant  (lowest priority - least specific)                 │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Merge Algorithm

```typescript
function mergeSearchResults(
  resultsByLayer: Map<MemoryLayer, MemorySearchResult[]>,
  limit: number
): MemorySearchResult[] {
  // 1. Flatten all results
  const allResults: MemorySearchResult[] = [];
  for (const [layer, results] of resultsByLayer) {
    allResults.push(...results);
  }
  
  // 2. Sort by: layer precedence (primary), score (secondary)
  allResults.sort((a, b) => {
    const layerDiff = getLayerPrecedence(a.layer) - getLayerPrecedence(b.layer);
    if (layerDiff !== 0) return layerDiff;
    return b.score - a.score; // Higher score first
  });
  
  // 3. Deduplicate by content similarity (optional)
  const deduped = deduplicateBySimilarity(allResults, 0.95);
  
  // 4. Return top N
  return deduped.slice(0, limit);
}

function getLayerPrecedence(layer: MemoryLayer): number {
  const precedence: Record<MemoryLayer, number> = {
    agent: 1,
    user: 2,
    session: 3,
    project: 4,
    team: 5,
    org: 6,
    tenant: 7
  };
  return precedence[layer];
}
```

### Override Behavior

More specific layers **override** less specific layers when content conflicts:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  tenant layer: "Use spaces for indentation"                    │
│                              │                                   │
│                              ▼                                   │
│  project layer: "Use tabs for indentation"  ◄── WINS            │
│                                                                  │
│  Result: Agent uses tabs (project overrides tenant)            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Layer Access Control

Layers are only searchable if appropriate identifiers are provided:

```typescript
function getAccessibleLayers(identifiers: LayerIdentifiers): MemoryLayer[] {
  const layers: MemoryLayer[] = [];
  
  // Always accessible with tenant ID
  if (identifiers.companyId) layers.push('tenant');
  
  // Org requires org ID
  if (identifiers.orgId) layers.push('org');
  
  // Team requires team ID
  if (identifiers.teamId) layers.push('team');
  
  // Project requires project ID
  if (identifiers.projectId) layers.push('project');
  
  // Session requires user + session ID
  if (identifiers.userId && identifiers.sessionId) layers.push('session');
  
  // User requires user ID
  if (identifiers.userId) layers.push('user');
  
  // Agent requires agent + user ID
  if (identifiers.agentId && identifiers.userId) layers.push('agent');
  
  return layers;
}
```

---

## Provider Adapter Interface

### Interface Definition

All memory providers must implement this interface:

```typescript
/**
 * Memory provider adapter interface.
 * Implement this to add support for a new storage backend.
 */
interface MemoryProviderAdapter {
  /** Provider name (e.g., "mem0", "letta", "chroma") */
  readonly name: string;
  
  /** Provider version */
  readonly version: string;
  
  /** Initialize the provider connection */
  initialize(config: ProviderConfig): Promise<void>;
  
  /** Clean up resources */
  shutdown(): Promise<void>;
  
  /** Health check */
  healthCheck(): Promise<HealthCheckResult>;
  
  // Core operations
  add(input: AddMemoryInput): Promise<AddMemoryOutput>;
  search(input: SearchMemoryInput): Promise<SearchMemoryOutput>;
  get(input: GetMemoryInput): Promise<GetMemoryOutput>;
  update(input: UpdateMemoryInput): Promise<UpdateMemoryOutput>;
  delete(input: DeleteMemoryInput): Promise<DeleteMemoryOutput>;
  list(input: ListMemoriesInput): Promise<ListMemoriesOutput>;
  
  // Embedding operations
  generateEmbedding(content: string): Promise<number[]>;
  
  // Bulk operations (optional)
  bulkAdd?(inputs: AddMemoryInput[]): Promise<AddMemoryOutput[]>;
  bulkDelete?(ids: string[]): Promise<{ deleted: number; failed: string[] }>;
}

interface ProviderConfig {
  /** Provider-specific configuration */
  [key: string]: unknown;
}

interface HealthCheckResult {
  /** Overall health status */
  status: 'healthy' | 'degraded' | 'unhealthy';
  
  /** Latency in milliseconds */
  latencyMs: number;
  
  /** Optional: Detailed component health */
  components?: Record<string, {
    status: 'healthy' | 'degraded' | 'unhealthy';
    message?: string;
  }>;
}
```

### Layer Isolation

Providers MUST ensure layer isolation. Implementation strategies:

#### Strategy 1: Namespace by Layer

```
Collection: memories_agent_{agentId}_{userId}
Collection: memories_user_{userId}
Collection: memories_session_{userId}_{sessionId}
Collection: memories_project_{projectId}
...
```

#### Strategy 2: Metadata Filtering

```json
{
  "content": "...",
  "metadata": {
    "_layer": "project",
    "_projectId": "proj_123"
  }
}
```

Query with filter: `metadata._layer == "project" AND metadata._projectId == "proj_123"`

#### Strategy 3: Tenant Partitioning

Use provider's native multi-tenancy:
- Qdrant: Separate collections per layer
- Pinecone: Namespaces per layer
- Chroma: Collections per layer

---

## Memory Lifecycle

### State Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│                        ┌─────────┐                              │
│                        │ CREATED │                              │
│                        └────┬────┘                              │
│                             │                                    │
│                             ▼                                    │
│                        ┌─────────┐                              │
│              ┌────────►│ ACTIVE  │◄────────┐                    │
│              │         └────┬────┘         │                    │
│              │              │              │                    │
│              │    ┌─────────┴─────────┐    │                    │
│              │    │                   │    │                    │
│              │    ▼                   ▼    │                    │
│         ┌────┴────┐             ┌─────────┐                     │
│         │ UPDATED │             │ DECAYED │                     │
│         └─────────┘             └────┬────┘                     │
│                                      │                          │
│                                      ▼                          │
│                                ┌──────────┐                     │
│                                │ ARCHIVED │                     │
│                                └────┬─────┘                     │
│                                     │                           │
│                                     ▼                           │
│                                ┌─────────┐                      │
│                                │ DELETED │                      │
│                                └─────────┘                      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Memory Decay (Optional)

Providers MAY support memory decay to reduce old memory relevance:

```typescript
interface DecayConfig {
  /** Enable decay */
  enabled: boolean;
  
  /** Decay rate per day (0.0 - 1.0) */
  ratePerDay: number;
  
  /** Minimum relevance before archival */
  archiveThreshold: number;
  
  /** Layers exempt from decay */
  exemptLayers: MemoryLayer[];
}
```

**Decay Formula:**

```
relevance(t) = initial_relevance * (1 - rate)^days_since_creation
```

### Memory Consolidation (Optional)

Providers MAY support consolidation to merge similar memories:

```typescript
interface ConsolidationConfig {
  /** Enable consolidation */
  enabled: boolean;
  
  /** Similarity threshold for merging (0.0 - 1.0) */
  similarityThreshold: number;
  
  /** Maximum memories before triggering consolidation */
  maxMemoriesBeforeTrigger: number;
  
  /** Layers to consolidate */
  targetLayers: MemoryLayer[];
}
```

### Session Memory Cleanup

Session-layer memories have special lifecycle:

```typescript
interface SessionCleanupConfig {
  /** Auto-delete session memories after session ends */
  autoDelete: boolean;
  
  /** Retention period after session end (e.g., "7d", "30d") */
  retentionPeriod?: string;
  
  /** Promote important memories to user layer */
  promoteImportant: boolean;
  
  /** Threshold for promotion (0.0 - 1.0) */
  promotionThreshold?: number;
}
```

---

## Error Handling

### Error Response Format

```typescript
interface MemoryError {
  /** Error code */
  code: MemoryErrorCode;
  
  /** Human-readable message */
  message: string;
  
  /** Operation that failed */
  operation: string;
  
  /** Additional context */
  details?: Record<string, unknown>;
  
  /** Whether operation can be retried */
  retryable: boolean;
}

type MemoryErrorCode =
  | 'INVALID_LAYER'
  | 'MISSING_IDENTIFIER'
  | 'MEMORY_NOT_FOUND'
  | 'CONTENT_TOO_LONG'
  | 'QUERY_TOO_LONG'
  | 'EMBEDDING_FAILED'
  | 'PROVIDER_ERROR'
  | 'RATE_LIMITED'
  | 'UNAUTHORIZED'
  | 'CONFIGURATION_ERROR';
```

### Error Handling Guidelines

| Error Code | Recommended Action |
|------------|-------------------|
| `INVALID_LAYER` | Fix input, do not retry |
| `MISSING_IDENTIFIER` | Add required identifier, do not retry |
| `MEMORY_NOT_FOUND` | Check ID, may be deleted |
| `CONTENT_TOO_LONG` | Truncate or split content |
| `QUERY_TOO_LONG` | Shorten query |
| `EMBEDDING_FAILED` | Retry with backoff |
| `PROVIDER_ERROR` | Retry with backoff, check provider status |
| `RATE_LIMITED` | Retry after delay (use `Retry-After` if provided) |
| `UNAUTHORIZED` | Check credentials, do not retry |
| `CONFIGURATION_ERROR` | Fix configuration, do not retry |

### Retry Strategy

```typescript
interface RetryConfig {
  /** Maximum retry attempts */
  maxAttempts: number;
  
  /** Initial delay in milliseconds */
  initialDelayMs: number;
  
  /** Maximum delay in milliseconds */
  maxDelayMs: number;
  
  /** Backoff multiplier */
  backoffMultiplier: number;
  
  /** Error codes to retry */
  retryableCodes: MemoryErrorCode[];
}

const defaultRetryConfig: RetryConfig = {
  maxAttempts: 3,
  initialDelayMs: 1000,
  maxDelayMs: 30000,
  backoffMultiplier: 2,
  retryableCodes: ['EMBEDDING_FAILED', 'PROVIDER_ERROR', 'RATE_LIMITED']
};
```

---

## Implementation Notes

### Thread Safety

Memory operations MUST be thread-safe. Implementations should:

1. Use atomic operations where possible
2. Implement optimistic locking for updates
3. Handle concurrent access to same memory gracefully

### Caching

Implementations MAY cache:

- Embeddings for frequently accessed content
- Layer resolution results
- Provider connection metadata

Implementations MUST NOT cache:

- Memory content (may be stale)
- Search results (query-dependent)

### Observability

Implementations SHOULD emit metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `memory.operations.total` | Counter | Total operations by type |
| `memory.operations.errors` | Counter | Failed operations by error code |
| `memory.operations.latency` | Histogram | Operation latency in ms |
| `memory.search.results` | Histogram | Number of results per search |
| `memory.storage.size` | Gauge | Total memories by layer |

---

**Next**: [03-knowledge-repository.md](./03-knowledge-repository.md) - Knowledge Repository Specification
