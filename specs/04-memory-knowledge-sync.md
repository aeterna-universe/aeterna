---
title: Memory-Knowledge Sync Specification
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 02-memory-system.md
  - 03-knowledge-repository.md
  - 05-adapter-architecture.md
---

# Memory-Knowledge Sync Specification

This document specifies the Sync Bridge component: the mechanism that keeps Memory and Knowledge systems aligned through pointer architecture and delta synchronization.

## Table of Contents

1. [Overview](#overview)
2. [Pointer Architecture](#pointer-architecture)
3. [Sync State Management](#sync-state-management)
4. [Delta Detection](#delta-detection)
5. [Sync Operations](#sync-operations)
6. [Conflict Resolution](#conflict-resolution)
7. [Sync Triggers](#sync-triggers)
8. [Error Handling](#error-handling)

---

## Overview

The Sync Bridge maintains consistency between Memory and Knowledge systems:

```
┌─────────────────────────────────────────────────────────────────┐
│                       SYNC BRIDGE                                │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   Sync Coordinator                       │    │
│  │  • Orchestrates sync operations                          │    │
│  │  • Manages sync state persistence                        │    │
│  │  • Handles conflict resolution                           │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│         ┌────────────────────┼────────────────────┐              │
│         │                    │                    │              │
│         ▼                    ▼                    ▼              │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐        │
│  │   Delta     │     │  Pointer    │     │   State     │        │
│  │  Detector   │     │  Manager    │     │  Persister  │        │
│  └─────────────┘     └─────────────┘     └─────────────┘        │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
         │                                         │
         ▼                                         ▼
┌─────────────────┐                     ┌─────────────────┐
│ KNOWLEDGE REPO  │                     │  MEMORY SYSTEM  │
└─────────────────┘                     └─────────────────┘
```

### Design Goals

| Goal | Description |
|------|-------------|
| **Efficiency** | Only sync changed items (delta sync) |
| **Consistency** | Memory pointers always reference valid knowledge |
| **Resilience** | Recover from partial failures |
| **Observability** | Clear audit trail of sync operations |

---

## Pointer Architecture

### The Pointer Pattern

Memory stores **lightweight pointers** to knowledge items:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  MEMORY ENTRY (Pointer)              KNOWLEDGE ITEM (Source)    │
│  ───────────────────────             ──────────────────────     │
│                                                                  │
│  ┌─────────────────────┐             ┌─────────────────────┐    │
│  │ id: mem_ptr_001     │             │ id: adr-042         │    │
│  │                     │             │                     │    │
│  │ content:            │             │ title: Database     │    │
│  │   "Use PostgreSQL   │             │        Selection    │    │
│  │    per ADR-042"     │             │                     │    │
│  │                     │             │ content:            │    │
│  │ metadata:           │             │   ## Context        │    │
│  │   knowledgePointer: │────────────►│   We need to...     │    │
│  │     sourceType: adr │             │   (500+ lines)      │    │
│  │     sourceId: adr042│             │                     │    │
│  │     contentHash:    │             │ constraints:        │    │
│  │       sha256:abc... │             │   - must_use:       │    │
│  │     syncedAt:       │             │       postgresql    │    │
│  │       2025-01-07    │             │                     │    │
│  └─────────────────────┘             └─────────────────────┘    │
│                                                                  │
│  Memory is LEAN                      Knowledge is COMPLETE      │
│  (fits in context)                   (full audit trail)         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Pointer Schema

```typescript
/**
 * Pointer from memory entry to knowledge item.
 */
interface KnowledgePointer {
  /** Type of knowledge item */
  sourceType: KnowledgeType;
  
  /** ID of knowledge item */
  sourceId: string;
  
  /** SHA-256 hash of content at sync time */
  contentHash: string;
  
  /** When this pointer was last synced */
  syncedAt: string;
  
  /** Knowledge layer */
  sourceLayer: KnowledgeLayer;
  
  /** Whether source still exists */
  isOrphaned: boolean;
}

/**
 * Memory entry with knowledge pointer.
 */
interface KnowledgePointerMemory {
  /** Memory ID */
  id: string;
  
  /** Summary content (from knowledge) */
  content: string;
  
  /** Memory layer */
  layer: MemoryLayer;
  
  /** Layer identifiers */
  identifiers: LayerIdentifiers;
  
  /** Pointer metadata */
  metadata: {
    /** Marker that this is a knowledge pointer */
    type: 'knowledge_pointer';
    
    /** The pointer itself */
    knowledgePointer: KnowledgePointer;
    
    /** Tags from knowledge item */
    tags?: string[];
  };
  
  /** Timestamps */
  createdAt: string;
  updatedAt: string;
}
```

### Content Generation

When syncing knowledge to memory, generate searchable content:

```typescript
function generatePointerContent(knowledge: KnowledgeItem): string {
  const parts: string[] = [];
  
  // Title
  parts.push(knowledge.title);
  
  // Summary
  parts.push(knowledge.summary);
  
  // Type indicator
  parts.push(`[${knowledge.type.toUpperCase()}]`);
  
  // Key constraints (if any blocking)
  const blockingConstraints = knowledge.constraints
    .filter(c => c.severity === 'block')
    .slice(0, 3);
  
  if (blockingConstraints.length > 0) {
    parts.push('Constraints:');
    for (const c of blockingConstraints) {
      parts.push(`- ${c.message || formatConstraint(c)}`);
    }
  }
  
  // Reference
  parts.push(`(${knowledge.id})`);
  
  return parts.join('\n');
}

function formatConstraint(c: Constraint): string {
  return `${c.operator}: ${c.pattern} [${c.target}]`;
}
```

### Layer Mapping

Knowledge layers map to memory layers for pointer storage:

| Knowledge Layer | Default Memory Layer | Rationale |
|-----------------|---------------------|-----------|
| `tenant` | `tenant` | 1:1 mapping |
| `org` | `org` | 1:1 mapping |
| `team` | `team` | 1:1 mapping |
| `project` | `project` | 1:1 mapping |

---

## Sync State Management

### Sync State Schema

```typescript
/**
 * Persistent state of sync operations.
 */
interface SyncState {
  /** Sync state version */
  version: '1.0';
  
  /** Last successful sync timestamp */
  lastSyncAt: string | null;
  
  /** Last knowledge commit synced */
  lastKnowledgeCommit: string | null;
  
  /** Hash map: knowledge ID → content hash at last sync */
  knowledgeHashes: Record<string, string>;
  
  /** Hash map: memory pointer ID → knowledge ID */
  pointerMapping: Record<string, string>;
  
  /** Items that failed to sync */
  failedItems: SyncFailure[];
  
  /** Sync statistics */
  stats: SyncStats;
}

interface SyncFailure {
  /** Knowledge item ID */
  knowledgeId: string;
  
  /** Error message */
  error: string;
  
  /** Failure timestamp */
  failedAt: string;
  
  /** Retry count */
  retryCount: number;
}

interface SyncStats {
  /** Total successful syncs */
  totalSyncs: number;
  
  /** Total items synced */
  totalItemsSynced: number;
  
  /** Total conflicts resolved */
  totalConflicts: number;
  
  /** Average sync duration (ms) */
  avgSyncDurationMs: number;
}
```

### State Persistence

```typescript
interface SyncStatePersister {
  /** Load sync state (or return default if none) */
  load(): Promise<SyncState>;
  
  /** Save sync state */
  save(state: SyncState): Promise<void>;
  
  /** Create checkpoint for rollback */
  checkpoint(): Promise<string>;
  
  /** Rollback to checkpoint */
  rollback(checkpointId: string): Promise<void>;
}
```

### Default Storage Locations

| Environment | Storage Location |
|-------------|------------------|
| Local development | `~/.config/memory-knowledge/sync-state.json` |
| CI/CD | Environment variable or secrets manager |
| Production | Database or object storage |

---

## Delta Detection

### Change Detection Algorithm

```typescript
interface DeltaResult {
  /** New items in knowledge not in memory */
  added: KnowledgeItem[];
  
  /** Items whose content hash changed */
  updated: KnowledgeItem[];
  
  /** Items in memory but not in knowledge */
  deleted: string[];
  
  /** Items unchanged */
  unchanged: string[];
}

async function detectDelta(
  knowledgeManifest: KnowledgeManifest,
  syncState: SyncState
): Promise<DeltaResult> {
  const delta: DeltaResult = {
    added: [],
    updated: [],
    deleted: [],
    unchanged: []
  };
  
  const currentKnowledgeIds = new Set(Object.keys(knowledgeManifest.items));
  const previousKnowledgeIds = new Set(Object.keys(syncState.knowledgeHashes));
  
  // Find added items
  for (const [id, entry] of Object.entries(knowledgeManifest.items)) {
    if (!previousKnowledgeIds.has(id)) {
      const item = await loadKnowledgeItem(id);
      delta.added.push(item);
    }
  }
  
  // Find updated items
  for (const [id, entry] of Object.entries(knowledgeManifest.items)) {
    if (previousKnowledgeIds.has(id)) {
      const previousHash = syncState.knowledgeHashes[id];
      if (previousHash !== entry.contentHash) {
        const item = await loadKnowledgeItem(id);
        delta.updated.push(item);
      } else {
        delta.unchanged.push(id);
      }
    }
  }
  
  // Find deleted items
  for (const id of previousKnowledgeIds) {
    if (!currentKnowledgeIds.has(id)) {
      delta.deleted.push(id);
    }
  }
  
  return delta;
}
```

### Content Hashing

```typescript
import { createHash } from 'crypto';

function computeContentHash(content: string): string {
  return createHash('sha256')
    .update(content, 'utf8')
    .digest('hex');
}

function computeKnowledgeHash(item: KnowledgeItem): string {
  // Hash includes content + constraints (structural changes)
  const hashInput = JSON.stringify({
    content: item.content,
    constraints: item.constraints,
    status: item.status
  });
  return computeContentHash(hashInput);
}
```

---

## Sync Operations

### Full Sync

Synchronize all knowledge items to memory:

```typescript
interface FullSyncInput {
  /** Force re-sync even if unchanged */
  force?: boolean;
  
  /** Layer identifiers for scoping */
  identifiers: LayerIdentifiers;
  
  /** Only sync specific knowledge types */
  types?: KnowledgeType[];
  
  /** Only sync specific layers */
  layers?: KnowledgeLayer[];
}

interface FullSyncOutput {
  /** Sync result */
  result: SyncResult;
  
  /** New sync state */
  newState: SyncState;
}

interface SyncResult {
  /** Whether sync completed successfully */
  success: boolean;
  
  /** Items added to memory */
  added: number;
  
  /** Items updated in memory */
  updated: number;
  
  /** Items removed from memory */
  deleted: number;
  
  /** Items unchanged */
  unchanged: number;
  
  /** Failures */
  failures: SyncFailure[];
  
  /** Sync duration in milliseconds */
  durationMs: number;
}
```

### Sync Algorithm

```typescript
async function executeFullSync(
  input: FullSyncInput,
  knowledgeRepo: KnowledgeRepository,
  memoryManager: MemoryManager,
  syncState: SyncState
): Promise<FullSyncOutput> {
  const startTime = Date.now();
  const result: SyncResult = {
    success: true,
    added: 0,
    updated: 0,
    deleted: 0,
    unchanged: 0,
    failures: [],
    durationMs: 0
  };
  
  // 1. Create checkpoint for rollback
  const checkpoint = await syncStatePersister.checkpoint();
  
  try {
    // 2. Get knowledge manifest
    const manifest = await knowledgeRepo.getManifest();
    
    // 3. Filter by types/layers if specified
    const filteredItems = filterManifest(manifest, input.types, input.layers);
    
    // 4. Detect delta
    const delta = input.force
      ? { added: Object.values(filteredItems), updated: [], deleted: [], unchanged: [] }
      : await detectDelta(filteredItems, syncState);
    
    // 5. Process additions
    for (const item of delta.added) {
      try {
        const memoryId = await createPointerMemory(item, input.identifiers, memoryManager);
        syncState.knowledgeHashes[item.id] = item.contentHash;
        syncState.pointerMapping[memoryId] = item.id;
        result.added++;
      } catch (error) {
        result.failures.push({
          knowledgeId: item.id,
          error: error.message,
          failedAt: new Date().toISOString(),
          retryCount: 0
        });
      }
    }
    
    // 6. Process updates
    for (const item of delta.updated) {
      try {
        const memoryId = findPointerMemoryId(item.id, syncState);
        await updatePointerMemory(memoryId, item, memoryManager);
        syncState.knowledgeHashes[item.id] = item.contentHash;
        result.updated++;
      } catch (error) {
        result.failures.push({
          knowledgeId: item.id,
          error: error.message,
          failedAt: new Date().toISOString(),
          retryCount: 0
        });
      }
    }
    
    // 7. Process deletions
    for (const knowledgeId of delta.deleted) {
      try {
        const memoryId = findPointerMemoryId(knowledgeId, syncState);
        await markPointerOrphaned(memoryId, memoryManager);
        delete syncState.knowledgeHashes[knowledgeId];
        result.deleted++;
      } catch (error) {
        result.failures.push({
          knowledgeId,
          error: error.message,
          failedAt: new Date().toISOString(),
          retryCount: 0
        });
      }
    }
    
    result.unchanged = delta.unchanged.length;
    
    // 8. Update sync state
    syncState.lastSyncAt = new Date().toISOString();
    syncState.lastKnowledgeCommit = manifest.commitHash;
    syncState.failedItems = result.failures;
    syncState.stats.totalSyncs++;
    syncState.stats.totalItemsSynced += result.added + result.updated;
    
    await syncStatePersister.save(syncState);
    
    result.success = result.failures.length === 0;
    
  } catch (error) {
    // Rollback on catastrophic failure
    await syncStatePersister.rollback(checkpoint);
    throw error;
  }
  
  result.durationMs = Date.now() - startTime;
  
  return { result, newState: syncState };
}
```

### Incremental Sync

Sync only items changed since last sync:

```typescript
interface IncrementalSyncInput {
  /** Layer identifiers */
  identifiers: LayerIdentifiers;
  
  /** Maximum items to sync (for rate limiting) */
  maxItems?: number;
}

async function executeIncrementalSync(
  input: IncrementalSyncInput,
  knowledgeRepo: KnowledgeRepository,
  memoryManager: MemoryManager,
  syncState: SyncState
): Promise<FullSyncOutput> {
  // Similar to full sync but:
  // 1. Uses lastKnowledgeCommit to fetch only new commits
  // 2. Processes changes incrementally
  // 3. Respects maxItems limit
  
  const commits = await knowledgeRepo.getCommitsSince(syncState.lastKnowledgeCommit);
  
  const affectedItems = new Set<string>();
  for (const commit of commits) {
    for (const itemId of commit.affectedItems) {
      affectedItems.add(itemId);
    }
  }
  
  // Process only affected items (up to maxItems)
  const itemsToProcess = Array.from(affectedItems).slice(0, input.maxItems ?? Infinity);
  
  // ... rest similar to full sync
}
```

### Single Item Sync

Sync a single knowledge item immediately:

```typescript
interface SingleItemSyncInput {
  /** Knowledge item ID */
  knowledgeId: string;
  
  /** Layer identifiers */
  identifiers: LayerIdentifiers;
}

async function syncSingleItem(
  input: SingleItemSyncInput,
  knowledgeRepo: KnowledgeRepository,
  memoryManager: MemoryManager,
  syncState: SyncState
): Promise<{ success: boolean; memoryId?: string; error?: string }> {
  const item = await knowledgeRepo.getItem(input.knowledgeId);
  
  if (!item) {
    // Item deleted - mark pointer orphaned
    const memoryId = findPointerMemoryId(input.knowledgeId, syncState);
    if (memoryId) {
      await markPointerOrphaned(memoryId, memoryManager);
      delete syncState.knowledgeHashes[input.knowledgeId];
      delete syncState.pointerMapping[memoryId];
    }
    return { success: true };
  }
  
  const existingMemoryId = findPointerMemoryId(input.knowledgeId, syncState);
  
  if (existingMemoryId) {
    // Update existing
    await updatePointerMemory(existingMemoryId, item, memoryManager);
    syncState.knowledgeHashes[item.id] = item.contentHash;
    return { success: true, memoryId: existingMemoryId };
  } else {
    // Create new
    const memoryId = await createPointerMemory(item, input.identifiers, memoryManager);
    syncState.knowledgeHashes[item.id] = item.contentHash;
    syncState.pointerMapping[memoryId] = item.id;
    return { success: true, memoryId };
  }
}
```

---

## Conflict Resolution

### Conflict Types

```typescript
type ConflictType =
  | 'hash_mismatch'      // Memory hash differs from knowledge
  | 'orphaned_pointer'   // Memory points to deleted knowledge
  | 'duplicate_pointer'  // Multiple memories point to same knowledge
  | 'layer_mismatch'     // Memory layer doesn't match knowledge layer
  | 'status_change';     // Knowledge status changed (deprecated/superseded)

interface Conflict {
  /** Conflict type */
  type: ConflictType;
  
  /** Memory entry involved */
  memoryId: string;
  
  /** Knowledge item involved */
  knowledgeId: string;
  
  /** Details */
  details: Record<string, unknown>;
  
  /** Suggested resolution */
  suggestedResolution: ConflictResolution;
}

type ConflictResolution =
  | 'update_memory'      // Update memory from knowledge
  | 'delete_memory'      // Remove orphaned memory
  | 'keep_memory'        // Keep memory, ignore knowledge change
  | 'merge'              // Merge changes
  | 'manual';            // Requires manual intervention
```

### Resolution Strategy

```typescript
interface ConflictResolutionConfig {
  /** Default resolution per conflict type */
  defaults: Record<ConflictType, ConflictResolution>;
  
  /** Custom resolver function */
  customResolver?: (conflict: Conflict) => ConflictResolution;
}

const defaultResolutionConfig: ConflictResolutionConfig = {
  defaults: {
    hash_mismatch: 'update_memory',      // Knowledge is authoritative
    orphaned_pointer: 'delete_memory',   // Clean up stale pointers
    duplicate_pointer: 'delete_memory',  // Keep newest, delete duplicates
    layer_mismatch: 'update_memory',     // Correct layer assignment
    status_change: 'update_memory'       // Reflect status in memory
  }
};

async function resolveConflict(
  conflict: Conflict,
  config: ConflictResolutionConfig,
  memoryManager: MemoryManager,
  knowledgeRepo: KnowledgeRepository
): Promise<void> {
  const resolution = config.customResolver?.(conflict) 
    ?? config.defaults[conflict.type];
  
  switch (resolution) {
    case 'update_memory':
      const item = await knowledgeRepo.getItem(conflict.knowledgeId);
      if (item) {
        await updatePointerMemory(conflict.memoryId, item, memoryManager);
      }
      break;
      
    case 'delete_memory':
      await memoryManager.delete({ id: conflict.memoryId });
      break;
      
    case 'keep_memory':
      // No action - log for audit
      break;
      
    case 'merge':
      // Complex merge logic - implementation specific
      break;
      
    case 'manual':
      throw new Error(`Conflict requires manual resolution: ${conflict.type}`);
  }
}
```

### Conflict Detection

```typescript
async function detectConflicts(
  syncState: SyncState,
  knowledgeRepo: KnowledgeRepository,
  memoryManager: MemoryManager
): Promise<Conflict[]> {
  const conflicts: Conflict[] = [];
  
  // Check each pointer for conflicts
  for (const [memoryId, knowledgeId] of Object.entries(syncState.pointerMapping)) {
    const memory = await memoryManager.get({ id: memoryId });
    const knowledge = await knowledgeRepo.getItem(knowledgeId);
    
    if (!memory) {
      // Memory was deleted externally
      conflicts.push({
        type: 'orphaned_pointer',
        memoryId,
        knowledgeId,
        details: { reason: 'memory_deleted' },
        suggestedResolution: 'delete_memory'
      });
      continue;
    }
    
    if (!knowledge) {
      // Knowledge was deleted
      conflicts.push({
        type: 'orphaned_pointer',
        memoryId,
        knowledgeId,
        details: { reason: 'knowledge_deleted' },
        suggestedResolution: 'delete_memory'
      });
      continue;
    }
    
    const pointer = memory.metadata.knowledgePointer as KnowledgePointer;
    
    // Check hash mismatch
    if (pointer.contentHash !== knowledge.contentHash) {
      conflicts.push({
        type: 'hash_mismatch',
        memoryId,
        knowledgeId,
        details: {
          memoryHash: pointer.contentHash,
          knowledgeHash: knowledge.contentHash
        },
        suggestedResolution: 'update_memory'
      });
    }
    
    // Check status change
    if (knowledge.status === 'deprecated' || knowledge.status === 'superseded') {
      conflicts.push({
        type: 'status_change',
        memoryId,
        knowledgeId,
        details: { newStatus: knowledge.status },
        suggestedResolution: 'update_memory'
      });
    }
  }
  
  return conflicts;
}
```

---

## Sync Triggers

### Trigger Types

```typescript
type SyncTrigger =
  | 'manual'           // User-initiated
  | 'scheduled'        // Cron/interval based
  | 'event'            // On knowledge change
  | 'session_start'    // On agent session start
  | 'threshold';       // After N sessions or staleness

interface SyncTriggerConfig {
  /** Enable automatic sync */
  autoSync: boolean;
  
  /** Scheduled sync interval (e.g., "1h", "6h", "1d") */
  scheduleInterval?: string;
  
  /** Sync on every N sessions */
  sessionThreshold?: number;
  
  /** Sync if state older than duration */
  stalenessThreshold?: string;
  
  /** Sync on knowledge commit webhook */
  webhookEnabled?: boolean;
}
```

### Trigger Evaluation

```typescript
function shouldTriggerSync(
  config: SyncTriggerConfig,
  syncState: SyncState,
  context: { sessionCount: number }
): { shouldSync: boolean; reason: SyncTrigger } {
  // Check staleness
  if (config.stalenessThreshold && syncState.lastSyncAt) {
    const lastSync = new Date(syncState.lastSyncAt);
    const threshold = parseDuration(config.stalenessThreshold);
    if (Date.now() - lastSync.getTime() > threshold) {
      return { shouldSync: true, reason: 'threshold' };
    }
  }
  
  // Check session threshold
  if (config.sessionThreshold) {
    const sessionsSinceSync = context.sessionCount - (syncState.stats.totalSyncs ?? 0);
    if (sessionsSinceSync >= config.sessionThreshold) {
      return { shouldSync: true, reason: 'threshold' };
    }
  }
  
  return { shouldSync: false, reason: 'manual' };
}
```

### Session-Based Sync

```typescript
interface SessionSyncConfig {
  /** Sync at session start */
  syncOnStart: boolean;
  
  /** Sync at session end */
  syncOnEnd: boolean;
  
  /** Only sync if stale */
  stalenessCheck: boolean;
  
  /** Staleness threshold */
  stalenessThreshold: string;
}

async function handleSessionStart(
  config: SessionSyncConfig,
  syncState: SyncState,
  identifiers: LayerIdentifiers
): Promise<void> {
  if (!config.syncOnStart) return;
  
  if (config.stalenessCheck) {
    const isStale = checkStaleness(syncState, config.stalenessThreshold);
    if (!isStale) return;
  }
  
  await executeIncrementalSync({ identifiers }, knowledgeRepo, memoryManager, syncState);
}
```

---

## Error Handling

### Error Types

```typescript
type SyncErrorCode =
  | 'KNOWLEDGE_UNAVAILABLE'  // Cannot reach knowledge repo
  | 'MEMORY_UNAVAILABLE'     // Cannot reach memory system
  | 'STATE_CORRUPTED'        // Sync state is invalid
  | 'CHECKPOINT_FAILED'      // Cannot create checkpoint
  | 'ROLLBACK_FAILED'        // Cannot rollback
  | 'PARTIAL_FAILURE'        // Some items failed to sync
  | 'CONFLICT_UNRESOLVED'    // Manual conflict resolution needed
  | 'TIMEOUT';               // Sync operation timed out

interface SyncError {
  code: SyncErrorCode;
  message: string;
  details?: Record<string, unknown>;
  retryable: boolean;
}
```

### Retry Strategy

```typescript
interface SyncRetryConfig {
  /** Maximum retry attempts */
  maxAttempts: number;
  
  /** Initial delay (ms) */
  initialDelayMs: number;
  
  /** Maximum delay (ms) */
  maxDelayMs: number;
  
  /** Backoff multiplier */
  backoffMultiplier: number;
  
  /** Error codes to retry */
  retryableCodes: SyncErrorCode[];
}

const defaultSyncRetryConfig: SyncRetryConfig = {
  maxAttempts: 3,
  initialDelayMs: 1000,
  maxDelayMs: 30000,
  backoffMultiplier: 2,
  retryableCodes: [
    'KNOWLEDGE_UNAVAILABLE',
    'MEMORY_UNAVAILABLE',
    'TIMEOUT'
  ]
};
```

### Failure Recovery

```typescript
async function recoverFromFailure(
  syncState: SyncState,
  lastGoodCheckpoint: string
): Promise<void> {
  // 1. Rollback sync state
  await syncStatePersister.rollback(lastGoodCheckpoint);
  
  // 2. Retry failed items
  const retryableFailures = syncState.failedItems.filter(
    f => f.retryCount < defaultSyncRetryConfig.maxAttempts
  );
  
  for (const failure of retryableFailures) {
    try {
      await syncSingleItem({
        knowledgeId: failure.knowledgeId,
        identifiers: getCurrentIdentifiers()
      }, knowledgeRepo, memoryManager, syncState);
      
      // Remove from failures
      syncState.failedItems = syncState.failedItems.filter(
        f => f.knowledgeId !== failure.knowledgeId
      );
    } catch (error) {
      failure.retryCount++;
      failure.failedAt = new Date().toISOString();
      failure.error = error.message;
    }
  }
  
  await syncStatePersister.save(syncState);
}
```

---

## Observability

### Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `sync.operations.total` | Counter | Total sync operations |
| `sync.operations.duration` | Histogram | Sync duration (ms) |
| `sync.items.added` | Counter | Items added to memory |
| `sync.items.updated` | Counter | Items updated in memory |
| `sync.items.deleted` | Counter | Items deleted from memory |
| `sync.conflicts.total` | Counter | Total conflicts detected |
| `sync.failures.total` | Counter | Failed sync operations |
| `sync.state.age` | Gauge | Time since last sync (s) |

### Logging

```typescript
interface SyncLogEntry {
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  operation: string;
  trigger: SyncTrigger;
  duration_ms: number;
  items_processed: number;
  failures: number;
  conflicts: number;
  details?: Record<string, unknown>;
}
```

---

**Next**: [05-adapter-architecture.md](./05-adapter-architecture.md) - Adapter Architecture Specification
