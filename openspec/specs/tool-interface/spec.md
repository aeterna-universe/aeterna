---
title: Tool Interface Specification
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

# Tool Interface Specification

## Purpose
Specifies the standard tool contracts for memory and knowledge operations via MCP, designed to be ecosystem-agnostic and compatible with any AI agent framework.
## Requirements
### Requirement: Per-Tool Authorization in MCP Dispatcher
The MCP tool dispatcher SHALL map each tool invocation to its corresponding domain Cedar action and evaluate authorization before executing the tool.

#### Scenario: Memory tool maps to memory Cedar action
- **WHEN** the MCP dispatcher receives a `memory_add` tool call
- **THEN** the dispatcher SHALL evaluate the `CreateMemory` Cedar action against the authenticated principal
- **AND** the tool SHALL only execute if the Cedar evaluation permits the action

#### Scenario: Knowledge tool maps to knowledge Cedar action
- **WHEN** the MCP dispatcher receives a `knowledge_query` tool call
- **THEN** the dispatcher SHALL evaluate the `SearchKnowledge` Cedar action against the authenticated principal

#### Scenario: Governance tool maps to governance Cedar action
- **WHEN** the MCP dispatcher receives a `governance_role_assign` tool call
- **THEN** the dispatcher SHALL evaluate the `AssignRoles` Cedar action against the authenticated principal

#### Scenario: CCA tool maps to CCA Cedar action
- **WHEN** the MCP dispatcher receives any CCA tool call (`context_assemble`, `note_capture`, `hindsight_query`, `meta_loop_status`)
- **THEN** the dispatcher SHALL evaluate the `InvokeCCA` Cedar action against the authenticated principal

#### Scenario: Graph tool maps to graph Cedar action
- **WHEN** the MCP dispatcher receives a read-only graph tool call (`graph_query`, `graph_neighbors`, `graph_path`)
- **THEN** the dispatcher SHALL evaluate the `QueryGraph` Cedar action
- **AND** mutating graph tool calls (`graph_link`, `graph_unlink`) SHALL evaluate the `ModifyGraph` Cedar action

### Requirement: Unique MCP Tool Names
The MCP tool dispatcher SHALL ensure that every registered tool has a unique name, preventing name collisions that cause undefined dispatch behavior.

#### Scenario: No duplicate tool names at registration
- **WHEN** the MCP server registers all tools at startup
- **THEN** the registration SHALL detect and reject duplicate tool names
- **AND** the server SHALL log an error and fail to start if a name collision is found

#### Scenario: governance_role_assign collision resolved
- **WHEN** the MCP tool set includes the previously colliding `governance_role_assign` tools
- **THEN** one SHALL be renamed to `governance_role_grant` (or equivalent unique name)
- **AND** both tools SHALL have distinct names and distinct Cedar action mappings

## Table of Contents

1. [Overview](#overview)
2. [Tool Design Principles](#tool-design-principles)
3. [Memory Tools](#memory-tools)
4. [Knowledge Tools](#knowledge-tools)
5. [Sync Tools](#sync-tools)
6. [Tool Schemas (JSON Schema)](#tool-schemas-json-schema)
7. [Error Responses](#error-responses)

---

## Overview

Tools are the primary interface for AI agents to interact with the Memory-Knowledge system:

```
┌─────────────────────────────────────────────────────────────────┐
│                         AI AGENT                                 │
│                                                                  │
│  "I need to remember this for later"                            │
│  "What policies apply to database selection?"                   │
│  "Check if adding MySQL violates any constraints"               │
│                                                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             │ Tool Invocation
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                       TOOL LAYER                                 │
│                                                                  │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐    │
│  │  memory_add     │ │ knowledge_query │ │    sync_now     │    │
│  │  memory_search  │ │ knowledge_check │ │  sync_status    │    │
│  │  memory_delete  │ │ knowledge_show  │ │                 │    │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘    │
│                                                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             │ Unified API
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                  MEMORY-KNOWLEDGE CORE                           │
└─────────────────────────────────────────────────────────────────┘
```

### Tool Categories

| Category | Tools | Purpose |
|----------|-------|---------|
| **Memory** | `memory_add`, `memory_search`, `memory_delete` | Store and retrieve semantic memories |
| **Knowledge** | `knowledge_query`, `knowledge_check`, `knowledge_show` | Query organizational knowledge and enforce constraints |
| **Sync** | `sync_now`, `sync_status` | Manual sync control and monitoring |

---

## Tool Design Principles

### 1. Semantic Clarity

Tool names and descriptions must be self-explanatory:

```
✓ memory_add      - "Store information for future reference"
✓ knowledge_check - "Verify actions comply with organizational policies"

✗ mem_w           - Unclear abbreviation
✗ validate        - Too generic
```

### 2. Minimal Required Parameters

Only require what's necessary. Use sensible defaults:

```typescript
// Good: layer defaults to 'user'
memory_add({ content: "User prefers TypeScript" })

// Bad: requiring layer for every call
memory_add({ content: "...", layer: "user", identifiers: {...} })
```

### 3. Rich Optional Parameters

Allow power users to customize behavior:

```typescript
// Basic usage
memory_search({ query: "database preferences" })

// Advanced usage
memory_search({
  query: "database preferences",
  layers: ["project", "org"],
  limit: 20,
  threshold: 0.8,
  filter: { tags: ["infrastructure"] }
})
```

### 4. Structured Responses

Return structured data, not just strings:

```typescript
// Good: structured response
{
  success: true,
  results: [...],
  metadata: { searchedLayers: [...], totalCount: 15 }
}

// Bad: unstructured string
"Found 3 results: 1. PostgreSQL is preferred..."
```

### 5. Graceful Degradation

Tools should handle missing context gracefully:

```typescript
// If identifiers not provided, use context from ecosystem adapter
memory_search({ query: "..." }) // Uses session context automatically
```

---

## Memory Tools

### memory_add

Store information in long-term memory for future retrieval.

#### Description

```
Store a piece of information in memory for future reference. 
Use this to remember user preferences, project context, decisions made, 
or any information that should persist across sessions.
```

#### Input Schema

```typescript
interface MemoryAddInput {
  /**
   * The content to remember.
   * Should be a clear, self-contained statement.
   * @required
   */
  content: string;
  
  /**
   * Memory scope/layer.
   * - agent: This agent instance only
   * - user: This user across all sessions
   * - session: This conversation only
   * - project: This project/repository
   * - team: This team
   * - org: This organization
   * - tenant: Tenant-wide
   * @default "user"
   */
  layer?: 'agent' | 'user' | 'session' | 'project' | 'team' | 'org' | 'tenant';
  
  /**
   * Tags for categorization and filtering.
   */
  tags?: string[];
  
  /**
   * Additional metadata.
   */
  metadata?: Record<string, unknown>;
}
```

#### Output Schema

```typescript
interface MemoryAddOutput {
  /** Whether the operation succeeded */
  success: boolean;
  
  /** ID of the created memory */
  memoryId: string;
  
  /** Confirmation message */
  message: string;
}
```

#### Example

```json
// Input
{
  "content": "User prefers functional programming patterns over OOP",
  "layer": "user",
  "tags": ["preferences", "coding-style"]
}

// Output
{
  "success": true,
  "memoryId": "mem_abc123",
  "message": "Memory stored successfully"
}
```

---

### memory_search

Search memories for relevant information.

#### Description

```
Search your memory for relevant past information. Use this to recall 
user preferences, project context, previous decisions, or any stored knowledge.
Results are ranked by relevance and filtered by layer precedence.
```

#### Input Schema

```typescript
interface MemorySearchInput {
  /**
   * Search query in natural language.
   * @required
   */
  query: string;
  
  /**
   * Layers to search. If not specified, searches all accessible layers.
   */
  layers?: MemoryLayer[];
  
  /**
   * Maximum number of results.
   * @default 10
   */
  limit?: number;
  
  /**
   * Minimum similarity threshold (0.0 - 1.0).
   * @default 0.7
   */
  threshold?: number;
  
  /**
   * Filter by tags.
   */
  tags?: string[];
}
```

#### Output Schema

```typescript
interface MemorySearchOutput {
  /** Whether the operation succeeded */
  success: boolean;
  
  /** Search results */
  results: Array<{
    /** Memory content */
    content: string;
    
    /** Layer this memory came from */
    layer: MemoryLayer;
    
    /** Relevance score (0.0 - 1.0) */
    score: number;
    
    /** Memory ID */
    memoryId: string;
    
    /** Tags */
    tags?: string[];
  }>;
  
  /** Total results found (before limit) */
  totalCount: number;
  
  /** Layers that were searched */
  searchedLayers: MemoryLayer[];
}
```

#### Example

```json
// Input
{
  "query": "What are the user's coding preferences?",
  "layers": ["user", "project"],
  "limit": 5
}

// Output
{
  "success": true,
  "results": [
    {
      "content": "User prefers functional programming patterns over OOP",
      "layer": "user",
      "score": 0.92,
      "memoryId": "mem_abc123",
      "tags": ["preferences", "coding-style"]
    },
    {
      "content": "Project uses TypeScript with strict mode enabled",
      "layer": "project",
      "score": 0.85,
      "memoryId": "mem_def456",
      "tags": ["typescript", "configuration"]
    }
  ],
  "totalCount": 2,
  "searchedLayers": ["user", "project"]
}
```

---

### memory_delete

Remove a specific memory.

#### Description

```
Delete a memory that is no longer relevant or was stored incorrectly.
Use this sparingly - memories usually remain valuable for context.
```

#### Input Schema

```typescript
interface MemoryDeleteInput {
  /**
   * ID of the memory to delete.
   * @required
   */
  memoryId: string;
}
```

#### Output Schema

```typescript
interface MemoryDeleteOutput {
  /** Whether the operation succeeded */
  success: boolean;
  
  /** Confirmation message */
  message: string;
}
```

---

## Knowledge Tools

### knowledge_query

Search organizational knowledge (ADRs, policies, patterns, specs).

#### Description

```
Search organizational knowledge for relevant decisions, policies, patterns, 
or specifications. Use this to find guidance on architectural decisions, 
coding standards, or established patterns before making changes.
```

#### Input Schema

```typescript
interface KnowledgeQueryInput {
  /**
   * Search query in natural language.
   */
  query?: string;
  
  /**
   * Filter by knowledge type.
   */
  type?: 'adr' | 'policy' | 'pattern' | 'spec';
  
  /**
   * Filter by layer.
   */
  layer?: 'tenant' | 'org' | 'team' | 'project';
  
  /**
   * Filter by tags.
   */
  tags?: string[];
  
  /**
   * Filter by status.
   * @default ["accepted"]
   */
  status?: Array<'draft' | 'proposed' | 'accepted' | 'deprecated' | 'superseded'>;
  
  /**
   * Maximum results.
   * @default 10
   */
  limit?: number;
}
```

#### Output Schema

```typescript
interface KnowledgeQueryOutput {
  /** Whether the operation succeeded */
  success: boolean;
  
  /** Matching knowledge items (summaries only) */
  items: Array<{
    /** Item ID */
    id: string;
    
    /** Item type */
    type: KnowledgeType;
    
    /** Layer */
    layer: KnowledgeLayer;
    
    /** Title */
    title: string;
    
    /** Summary */
    summary: string;
    
    /** Status */
    status: KnowledgeStatus;
    
    /** Tags */
    tags: string[];
    
    /** Whether item has constraints */
    hasConstraints: boolean;
  }>;
  
  /** Total count */
  totalCount: number;
}
```

#### Example

```json
// Input
{
  "query": "database selection",
  "type": "adr",
  "status": ["accepted"]
}

// Output
{
  "success": true,
  "items": [
    {
      "id": "adr-042-database-selection",
      "type": "adr",
      "layer": "org",
      "title": "Database Selection for New Services",
      "summary": "Use PostgreSQL for all new services requiring relational data",
      "status": "accepted",
      "tags": ["database", "infrastructure"],
      "hasConstraints": true
    }
  ],
  "totalCount": 1
}
```

---

### knowledge_check

Verify that an action complies with organizational policies.

#### Description

```
Check if planned changes comply with organizational constraints and policies.
Use this before adding dependencies, creating files, or making architectural changes
to ensure compliance with established standards.
```

#### Input Schema

```typescript
interface KnowledgeCheckInput {
  /**
   * Files to check (path and content).
   */
  files?: Array<{
    path: string;
    content: string;
  }>;
  
  /**
   * Dependencies to check.
   */
  dependencies?: Array<{
    name: string;
    version?: string;
  }>;
  
  /**
   * Minimum severity to report.
   * @default "warn"
   */
  minSeverity?: 'info' | 'warn' | 'block';
  
  /**
   * Specific knowledge items to check against.
   * If not specified, checks all applicable constraints.
   */
  knowledgeItemIds?: string[];
}
```

#### Output Schema

```typescript
interface KnowledgeCheckOutput {
  /** Whether all blocking constraints pass */
  passed: boolean;
  
  /** Violations found */
  violations: Array<{
    /** Knowledge item that defines this constraint */
    knowledgeItemId: string;
    
    /** Knowledge item title */
    knowledgeItemTitle: string;
    
    /** Constraint that was violated */
    constraint: {
      operator: string;
      target: string;
      pattern: string;
    };
    
    /** Severity */
    severity: 'info' | 'warn' | 'block';
    
    /** Human-readable message */
    message: string;
    
    /** Where the violation occurred */
    location?: {
      file: string;
      line?: number;
    };
  }>;
  
  /** Summary counts */
  summary: {
    info: number;
    warn: number;
    block: number;
  };
}
```

#### Example

```json
// Input
{
  "dependencies": [
    { "name": "mysql2", "version": "3.0.0" }
  ]
}

// Output
{
  "passed": false,
  "violations": [
    {
      "knowledgeItemId": "adr-042-database-selection",
      "knowledgeItemTitle": "Database Selection for New Services",
      "constraint": {
        "operator": "must_not_use",
        "target": "dependency",
        "pattern": "mysql|mysql2|mariadb"
      },
      "severity": "block",
      "message": "MySQL not allowed for new services per ADR-042. Use PostgreSQL instead."
    }
  ],
  "summary": {
    "info": 0,
    "warn": 0,
    "block": 1
  }
}
```

---

### knowledge_show

Get full details of a specific knowledge item.

#### Description

```
Retrieve the complete content of a knowledge item including full text,
constraints, and metadata. Use this when you need detailed information
about a specific decision, policy, or pattern.
```

#### Input Schema

```typescript
interface KnowledgeShowInput {
  /**
   * Knowledge item ID.
   * @required
   */
  id: string;
  
  /**
   * Include constraint definitions.
   * @default true
   */
  includeConstraints?: boolean;
  
  /**
   * Include version history.
   * @default false
   */
  includeHistory?: boolean;
}
```

#### Output Schema

```typescript
interface KnowledgeShowOutput {
  /** Whether the operation succeeded */
  success: boolean;
  
  /** The knowledge item (null if not found) */
  item: {
    id: string;
    type: KnowledgeType;
    layer: KnowledgeLayer;
    title: string;
    summary: string;
    content: string;  // Full markdown content
    status: KnowledgeStatus;
    severity: ConstraintSeverity;
    tags: string[];
    constraints?: Constraint[];
    metadata: Record<string, unknown>;
    createdAt: string;
    updatedAt: string;
    supersedes?: string;
    supersededBy?: string[];
  } | null;
  
  /** Version history (if requested) */
  history?: Array<{
    version: string;
    timestamp: string;
    author: string;
    message: string;
  }>;
}
```

---

## Sync Tools

### sync_now

Trigger manual synchronization between memory and knowledge.

#### Description

```
Manually trigger synchronization between memory and knowledge systems.
Use this after significant knowledge updates or if you suspect memories
are out of date with the latest organizational policies.
```

#### Input Schema

```typescript
interface SyncNowInput {
  /**
   * Force full sync (ignore delta detection).
   * @default false
   */
  force?: boolean;
  
  /**
   * Only sync specific knowledge types.
   */
  types?: KnowledgeType[];
  
  /**
   * Only sync specific layers.
   */
  layers?: KnowledgeLayer[];
}
```

#### Output Schema

```typescript
interface SyncNowOutput {
  /** Whether sync completed successfully */
  success: boolean;
  
  /** Sync results */
  result: {
    added: number;
    updated: number;
    deleted: number;
    unchanged: number;
    failures: number;
  };
  
  /** Sync duration in milliseconds */
  durationMs: number;
  
  /** Message */
  message: string;
}
```

---

### sync_status

Check the current sync status.

#### Description

```
Check the status of memory-knowledge synchronization, including
last sync time, pending changes, and any sync failures.
```

#### Input Schema

```typescript
interface SyncStatusInput {
  /** No input required */
}
```

#### Output Schema

```typescript
interface SyncStatusOutput {
  /** Whether sync is healthy */
  healthy: boolean;
  
  /** Last sync timestamp */
  lastSyncAt: string | null;
  
  /** Time since last sync (human readable) */
  timeSinceSync: string;
  
  /** Number of failed items */
  failedItems: number;
  
  /** Sync statistics */
  stats: {
    totalSyncs: number;
    totalItemsSynced: number;
    avgSyncDurationMs: number;
  };
}
```

---

## Tool Schemas (JSON Schema)

### memory_add Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "content": {
      "type": "string",
      "description": "The content to remember"
    },
    "layer": {
      "type": "string",
      "enum": ["agent", "user", "session", "project", "team", "org", "tenant"],
      "default": "user",
      "description": "Memory scope"
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Tags for categorization"
    },
    "metadata": {
      "type": "object",
      "additionalProperties": true,
      "description": "Additional metadata"
    }
  },
  "required": ["content"]
}
```

### memory_search Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "layers": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": ["agent", "user", "session", "project", "team", "org", "tenant"]
      },
      "description": "Layers to search"
    },
    "limit": {
      "type": "integer",
      "minimum": 1,
      "maximum": 100,
      "default": 10,
      "description": "Maximum results"
    },
    "threshold": {
      "type": "number",
      "minimum": 0,
      "maximum": 1,
      "default": 0.7,
      "description": "Minimum similarity threshold"
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Filter by tags"
    }
  },
  "required": ["query"]
}
```

### knowledge_query Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "type": {
      "type": "string",
      "enum": ["adr", "policy", "pattern", "spec"],
      "description": "Filter by type"
    },
    "layer": {
      "type": "string",
      "enum": ["tenant", "org", "team", "project"],
      "description": "Filter by layer"
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Filter by tags"
    },
    "status": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": ["draft", "proposed", "accepted", "deprecated", "superseded"]
      },
      "default": ["accepted"],
      "description": "Filter by status"
    },
    "limit": {
      "type": "integer",
      "minimum": 1,
      "maximum": 100,
      "default": 10,
      "description": "Maximum results"
    }
  }
}
```

### knowledge_check Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "files": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "path": { "type": "string" },
          "content": { "type": "string" }
        },
        "required": ["path", "content"]
      },
      "description": "Files to check"
    },
    "dependencies": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "version": { "type": "string" }
        },
        "required": ["name"]
      },
      "description": "Dependencies to check"
    },
    "minSeverity": {
      "type": "string",
      "enum": ["info", "warn", "block"],
      "default": "warn",
      "description": "Minimum severity to report"
    },
    "knowledgeItemIds": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Specific items to check"
    }
  }
}
```

---

## Error Responses

### Standard Error Format

```typescript
interface ToolError {
  /** Error indicator */
  success: false;
  
  /** Error code */
  errorCode: string;
  
  /** Human-readable message */
  message: string;
  
  /** Additional details */
  details?: Record<string, unknown>;
  
  /** Whether the operation can be retried */
  retryable: boolean;
}
```

### Common Error Codes

| Code | Description | Retryable |
|------|-------------|-----------|
| `INVALID_INPUT` | Input validation failed | No |
| `NOT_FOUND` | Resource not found | No |
| `PROVIDER_ERROR` | Storage provider error | Yes |
| `RATE_LIMITED` | Too many requests | Yes |
| `UNAUTHORIZED` | Not authorized | No |
| `TIMEOUT` | Operation timed out | Yes |
| `CONFLICT` | Concurrent modification | Yes |

### Error Example

```json
{
  "success": false,
  "errorCode": "NOT_FOUND",
  "message": "Knowledge item 'adr-999' not found",
  "details": {
    "requestedId": "adr-999",
    "searchedLayers": ["tenant", "org", "project"]
  },
  "retryable": false
}
```

---

**Next**: [07-configuration.md](./07-configuration.md) - Configuration Specification
