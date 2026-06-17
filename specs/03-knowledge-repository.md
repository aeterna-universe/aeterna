---
title: Knowledge Repository Specification
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

# Knowledge Repository Specification

This document specifies the Knowledge Repository component: a versioned, Git-based store for organizational decisions, policies, patterns, and specifications.

## Table of Contents

1. [Overview](#overview)
2. [Knowledge Types](#knowledge-types)
3. [Knowledge Item Schema](#knowledge-item-schema)
4. [Constraint DSL](#constraint-dsl)
5. [Repository Structure](#repository-structure)
6. [Versioning Model](#versioning-model)
7. [Multi-Tenant Federation](#multi-tenant-federation)
8. [Core Operations](#core-operations)
9. [Error Handling](#error-handling)

---

## Overview

The Knowledge Repository provides:

- **Structured storage**: Typed artifacts (ADRs, policies, patterns, specs)
- **Git-based versioning**: Full audit trail, immutable commits
- **Constraint enforcement**: Declarative rules guiding agent behavior
- **Multi-tenant federation**: Tenant → Org → Team → Project layers

```
┌─────────────────────────────────────────────────────────────────┐
│                   KNOWLEDGE REPOSITORY                           │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                 Knowledge Manager                        │    │
│  │  • Coordinates all knowledge operations                  │    │
│  │  • Enforces schema validation                            │    │
│  │  • Routes to appropriate layer                           │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                 Constraint Engine                        │    │
│  │  • Parses constraint DSL                                 │    │
│  │  • Evaluates constraints against context                 │    │
│  │  • Reports violations by severity                        │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  Version Manager                         │    │
│  │  • Creates immutable commits                             │    │
│  │  • Manages manifest index                                │    │
│  │  • Handles federation sync                               │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   Git Backend                            │    │
│  │  • Persists to Git repository                            │    │
│  │  • Handles branching and merging                         │    │
│  │  • Supports local and remote repos                       │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Knowledge Types

### The Four Knowledge Types

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  TYPE        PURPOSE                       EXAMPLES              │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  adr         Architecture Decision         "Use PostgreSQL"     │
│              Records                        "Adopt microservices"│
│                                                                  │
│  policy      Organizational rules           "No console.log"     │
│              and constraints               "PR reviews required" │
│                                                                  │
│  pattern     Reusable solutions            "Error handling"      │
│              and best practices            "API response format" │
│                                                                  │
│  spec        Technical specifications       "API contract"       │
│              and contracts                  "Data schema"        │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Type Characteristics

| Type | Immutable | Has Constraints | Supersedes | Typical Size |
|------|-----------|-----------------|------------|--------------|
| `adr` | Yes (decisions are final) | Often | Previous ADRs | Medium |
| `policy` | No (can be updated) | Always | - | Small |
| `pattern` | No (evolves) | Sometimes | - | Large |
| `spec` | Yes (versioned) | Sometimes | Previous versions | Large |

### Type Definitions

```typescript
type KnowledgeType = 'adr' | 'policy' | 'pattern' | 'spec';

interface KnowledgeTypeConfig {
  /** Human-readable name */
  displayName: string;
  
  /** Description of this type */
  description: string;
  
  /** Whether items of this type can be updated (vs superseded) */
  allowUpdates: boolean;
  
  /** Required fields for this type */
  requiredFields: string[];
  
  /** File extension for this type */
  fileExtension: string;
}

const knowledgeTypeConfigs: Record<KnowledgeType, KnowledgeTypeConfig> = {
  adr: {
    displayName: 'Architecture Decision Record',
    description: 'Documents significant architectural decisions',
    allowUpdates: false, // Supersede instead
    requiredFields: ['context', 'decision', 'consequences'],
    fileExtension: '.md'
  },
  policy: {
    displayName: 'Policy',
    description: 'Organizational rules and guidelines',
    allowUpdates: true,
    requiredFields: ['scope', 'rules'],
    fileExtension: '.md'
  },
  pattern: {
    displayName: 'Pattern',
    description: 'Reusable solutions and best practices',
    allowUpdates: true,
    requiredFields: ['problem', 'solution'],
    fileExtension: '.md'
  },
  spec: {
    displayName: 'Specification',
    description: 'Technical specifications and contracts',
    allowUpdates: false, // Version instead
    requiredFields: ['version'],
    fileExtension: '.md'
  }
};
```

---

## Knowledge Item Schema

### Core Schema

```typescript
/**
 * A single knowledge item in the repository.
 */
interface KnowledgeItem {
  /** Unique identifier (e.g., "adr-042-database-selection") */
  id: string;
  
  /** Knowledge type */
  type: KnowledgeType;
  
  /** Layer this item belongs to */
  layer: KnowledgeLayer;
  
  /** Human-readable title */
  title: string;
  
  /** Brief summary (for memory pointer) */
  summary: string;
  
  /** Full content (Markdown) */
  content: string;
  
  /** SHA-256 hash of content */
  contentHash: string;
  
  /** Status in lifecycle */
  status: KnowledgeStatus;
  
  /** Severity for constraint enforcement */
  severity: ConstraintSeverity;
  
  /** Attached constraints */
  constraints: Constraint[];
  
  /** Tags for categorization */
  tags: string[];
  
  /** Metadata */
  metadata: KnowledgeMetadata;
  
  /** Creation timestamp */
  createdAt: string;
  
  /** Last update timestamp */
  updatedAt: string;
  
  /** Version number (for specs) */
  version?: string;
  
  /** ID of superseded item (for ADRs) */
  supersedes?: string;
  
  /** IDs of items that supersede this one */
  supersededBy?: string[];
}

type KnowledgeLayer = 'tenant' | 'org' | 'team' | 'project';

type KnowledgeStatus = 
  | 'draft'      // Work in progress
  | 'proposed'   // Ready for review
  | 'accepted'   // Approved and active
  | 'deprecated' // No longer recommended
  | 'superseded' // Replaced by another item
  | 'rejected';  // Not accepted

type ConstraintSeverity = 
  | 'info'   // Informational, no enforcement
  | 'warn'   // Warning, can be overridden
  | 'block'; // Blocking, must be followed

interface KnowledgeMetadata {
  /** Authors */
  authors?: string[];
  
  /** Reviewers who approved */
  reviewers?: string[];
  
  /** Related items */
  relatedItems?: string[];
  
  /** External references */
  references?: string[];
  
  /** Custom fields */
  [key: string]: unknown;
}
```

### Knowledge Layer Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  LAYER       SCOPE                  EXAMPLES                     │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  tenant     Entire tenant         "All code must be typed"    │
│     │        (least specific)       "Security policy"           │
│     │                                                            │
│  org         Business unit          "Use React for frontend"    │
│     │                               "API versioning scheme"     │
│     │                                                            │
│  team        Team scope             "Code review checklist"     │
│     │                               "Sprint ceremonies"         │
│     │                                                            │
│  project     Single project         "Project-specific ADRs"     │
│              (most specific)        "Local conventions"         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Example Knowledge Items

#### ADR Example

```yaml
---
id: adr-042-database-selection
type: adr
layer: org
title: Database Selection for New Services
summary: Use PostgreSQL for all new services requiring relational data storage
status: accepted
severity: block
tags: [database, infrastructure, architecture]
metadata:
  authors: [alice@example.com]
  reviewers: [bob@example.com, charlie@example.com]
  relatedItems: [adr-015-data-layer]
createdAt: "2025-01-07T10:00:00Z"
updatedAt: "2025-01-07T10:00:00Z"
---

## Context

We need to standardize on a relational database for new services. Currently, teams use MySQL, PostgreSQL, and SQLite inconsistently.

## Decision

We will use **PostgreSQL** for all new services requiring relational data storage.

## Rationale

1. Superior JSON support for semi-structured data
2. Better performance for complex queries
3. Strong community and ecosystem
4. Existing operational expertise

## Consequences

### Positive
- Consistent tooling and operations
- Easier cross-service data integration
- Reduced training burden

### Negative
- Teams with MySQL expertise need to learn PostgreSQL
- Some existing services will remain on MySQL

## Constraints

```yaml
constraints:
  - operator: must_use
    target: dependency
    pattern: "postgresql|pg|postgres"
    severity: block
    message: "Use PostgreSQL for relational data per ADR-042"
  - operator: must_not_use
    target: dependency
    pattern: "mysql|mysql2|mariadb"
    severity: block
    message: "MySQL not allowed for new services per ADR-042"
```
```

#### Policy Example

```yaml
---
id: policy-no-console-log
type: policy
layer: tenant
title: No Console.log in Production Code
summary: Console.log statements must be removed before merging to main
status: accepted
severity: warn
tags: [logging, code-quality]
metadata:
  authors: [security-team@example.com]
createdAt: "2025-01-01T00:00:00Z"
updatedAt: "2025-01-07T00:00:00Z"
---

## Scope

All production code in any language that has console/print debugging.

## Rules

1. No `console.log()` in JavaScript/TypeScript production code
2. No `print()` debugging in Python production code
3. Use proper logging frameworks with log levels

## Exceptions

- Test files (*.test.ts, *_test.py)
- Development-only scripts
- CLI tools intended for local use

## Constraints

```yaml
constraints:
  - operator: must_not_match
    target: code
    pattern: "console\\.log\\("
    appliesTo: ["*.ts", "*.js"]
    severity: warn
    message: "Remove console.log before merging (use logger instead)"
```

---

## Constraint DSL

### Constraint Schema

```typescript
/**
 * A constraint attached to a knowledge item.
 */
interface Constraint {
  /** Constraint operator */
  operator: ConstraintOperator;
  
  /** What the constraint targets */
  target: ConstraintTarget;
  
  /** Pattern to match (regex or glob depending on target) */
  pattern: string;
  
  /** File patterns this applies to (glob) */
  appliesTo?: string[];
  
  /** Severity of violation */
  severity: ConstraintSeverity;
  
  /** Human-readable message on violation */
  message?: string;
}

type ConstraintOperator =
  | 'must_use'       // Pattern MUST be present
  | 'must_not_use'   // Pattern MUST NOT be present
  | 'must_match'     // Content MUST match pattern
  | 'must_not_match' // Content MUST NOT match pattern
  | 'must_exist'     // File/path MUST exist
  | 'must_not_exist';// File/path MUST NOT exist

type ConstraintTarget =
  | 'file'       // File paths
  | 'code'       // Code content
  | 'dependency' // Package dependencies
  | 'import'     // Import statements
  | 'config';    // Configuration files
```

### Operator Semantics

| Operator | Target | Pattern | Passes When |
|----------|--------|---------|-------------|
| `must_use` | dependency | `postgresql` | `postgresql` is in dependencies |
| `must_not_use` | dependency | `mysql` | `mysql` is NOT in dependencies |
| `must_match` | code | `^import.*from ['"]@tenant/` | All imports from @tenant/* |
| `must_not_match` | code | `console\.log\(` | No console.log statements |
| `must_exist` | file | `README.md` | README.md exists |
| `must_not_exist` | file | `.env.local` | .env.local not committed |

### Constraint Evaluation

```typescript
interface ConstraintContext {
  /** Files being checked */
  files: FileInfo[];
  
  /** Dependencies from package.json/requirements.txt/etc */
  dependencies: DependencyInfo[];
  
  /** Current layer identifiers */
  layerIdentifiers: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface FileInfo {
  path: string;
  content: string;
}

interface DependencyInfo {
  name: string;
  version: string;
  type: 'production' | 'development';
}

interface ConstraintViolation {
  /** The constraint that was violated */
  constraint: Constraint;
  
  /** Knowledge item the constraint came from */
  knowledgeItemId: string;
  
  /** Where the violation occurred */
  location?: {
    file: string;
    line?: number;
    column?: number;
  };
  
  /** Severity of this violation */
  severity: ConstraintSeverity;
  
  /** Human-readable message */
  message: string;
}

interface ConstraintCheckResult {
  /** Whether all constraints passed */
  passed: boolean;
  
  /** All violations found */
  violations: ConstraintViolation[];
  
  /** Violations by severity */
  summary: {
    info: number;
    warn: number;
    block: number;
  };
}
```

### Evaluation Algorithm

```typescript
function evaluateConstraints(
  constraints: Constraint[],
  context: ConstraintContext
): ConstraintCheckResult {
  const violations: ConstraintViolation[] = [];
  
  for (const constraint of constraints) {
    const applicableFiles = constraint.appliesTo
      ? context.files.filter(f => matchGlob(f.path, constraint.appliesTo!))
      : context.files;
    
    switch (constraint.target) {
      case 'dependency':
        evaluateDependencyConstraint(constraint, context.dependencies, violations);
        break;
      case 'code':
        evaluateCodeConstraint(constraint, applicableFiles, violations);
        break;
      case 'file':
        evaluateFileConstraint(constraint, context.files, violations);
        break;
      case 'import':
        evaluateImportConstraint(constraint, applicableFiles, violations);
        break;
      case 'config':
        evaluateConfigConstraint(constraint, context.files, violations);
        break;
    }
  }
  
  return {
    passed: violations.filter(v => v.severity === 'block').length === 0,
    violations,
    summary: {
      info: violations.filter(v => v.severity === 'info').length,
      warn: violations.filter(v => v.severity === 'warn').length,
      block: violations.filter(v => v.severity === 'block').length
    }
  };
}
```

### Severity Behavior

| Severity | On Violation | Agent Behavior |
|----------|--------------|----------------|
| `info` | Log message | Continue normally |
| `warn` | Show warning | Continue with caution, may prompt user |
| `block` | Raise error | Stop action, explain constraint, suggest fix |

---

## Repository Structure

### Directory Layout

```
knowledge-repo/
├── manifest.json           # Index of all items
├── tenant/                # Tenant-wide knowledge
│   ├── adrs/
│   │   └── adr-001-*.md
│   ├── policies/
│   │   └── policy-*.md
│   ├── patterns/
│   │   └── pattern-*.md
│   └── specs/
│       └── spec-*.md
├── orgs/                   # Organization-specific
│   └── {orgId}/
│       ├── adrs/
│       ├── policies/
│       ├── patterns/
│       └── specs/
├── teams/                  # Team-specific
│   └── {teamId}/
│       ├── adrs/
│       ├── policies/
│       ├── patterns/
│       └── specs/
└── projects/               # Project-specific
    └── {projectId}/
        ├── adrs/
        ├── policies/
        ├── patterns/
        └── specs/
```

### Manifest Schema

```typescript
interface KnowledgeManifest {
  /** Manifest version */
  version: '1.0';
  
  /** Generation timestamp */
  generatedAt: string;
  
  /** Git commit hash */
  commitHash: string;
  
  /** All items by ID */
  items: Record<string, ManifestEntry>;
  
  /** Items grouped by layer */
  byLayer: Record<KnowledgeLayer, string[]>;
  
  /** Items grouped by type */
  byType: Record<KnowledgeType, string[]>;
  
  /** Items grouped by status */
  byStatus: Record<KnowledgeStatus, string[]>;
}

interface ManifestEntry {
  /** Item ID */
  id: string;
  
  /** Item type */
  type: KnowledgeType;
  
  /** Item layer */
  layer: KnowledgeLayer;
  
  /** File path relative to repo root */
  path: string;
  
  /** Item title */
  title: string;
  
  /** Item summary */
  summary: string;
  
  /** Current status */
  status: KnowledgeStatus;
  
  /** Content hash for change detection */
  contentHash: string;
  
  /** Whether item has constraints */
  hasConstraints: boolean;
  
  /** Constraint count by severity */
  constraintSeverity?: {
    info: number;
    warn: number;
    block: number;
  };
  
  /** Tags */
  tags: string[];
  
  /** Last modified timestamp */
  updatedAt: string;
}
```

### Example Manifest

```json
{
  "version": "1.0",
  "generatedAt": "2025-01-07T12:00:00Z",
  "commitHash": "abc123def456",
  "items": {
    "adr-042-database-selection": {
      "id": "adr-042-database-selection",
      "type": "adr",
      "layer": "org",
      "path": "orgs/engineering/adrs/adr-042-database-selection.md",
      "title": "Database Selection for New Services",
      "summary": "Use PostgreSQL for all new services",
      "status": "accepted",
      "contentHash": "sha256:abc123...",
      "hasConstraints": true,
      "constraintSeverity": {
        "info": 0,
        "warn": 0,
        "block": 2
      },
      "tags": ["database", "infrastructure"],
      "updatedAt": "2025-01-07T10:00:00Z"
    }
  },
  "byLayer": {
    "tenant": ["policy-security-baseline"],
    "org": ["adr-042-database-selection"],
    "team": [],
    "project": []
  },
  "byType": {
    "adr": ["adr-042-database-selection"],
    "policy": ["policy-security-baseline"],
    "pattern": [],
    "spec": []
  },
  "byStatus": {
    "accepted": ["adr-042-database-selection", "policy-security-baseline"],
    "draft": [],
    "proposed": [],
    "deprecated": [],
    "superseded": [],
    "rejected": []
  }
}
```

---

## Versioning Model

### Commit Schema

```typescript
/**
 * An immutable commit in the knowledge repository.
 */
interface KnowledgeCommit {
  /** Commit hash */
  hash: string;
  
  /** Parent commit hash (null for initial) */
  parent: string | null;
  
  /** Commit timestamp */
  timestamp: string;
  
  /** Author identifier */
  author: string;
  
  /** Commit message */
  message: string;
  
  /** Type of change */
  changeType: CommitChangeType;
  
  /** Items affected */
  affectedItems: string[];
  
  /** Manifest snapshot at this commit */
  manifest: KnowledgeManifest;
}

type CommitChangeType =
  | 'create'     // New item added
  | 'update'     // Existing item modified
  | 'delete'     // Item removed
  | 'supersede'  // Item superseded by another
  | 'status'     // Status change only
  | 'federation';// Sync from upstream
```

### Version History

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Commit: abc123                                                  │
│  Parent: def456                                                  │
│  Author: alice@example.com                                       │
│  Date:   2025-01-07T10:00:00Z                                   │
│  Type:   create                                                  │
│                                                                  │
│  Message: Add ADR-042: Database Selection                        │
│                                                                  │
│  Affected Items:                                                 │
│    + adr-042-database-selection                                 │
│                                                                  │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  Commit: def456                                                  │
│  Parent: ghi789                                                  │
│  Author: bob@example.com                                         │
│  Date:   2025-01-06T15:30:00Z                                   │
│  Type:   update                                                  │
│                                                                  │
│  Message: Update security policy with new requirements           │
│                                                                  │
│  Affected Items:                                                 │
│    ~ policy-security-baseline                                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Immutability Rules

1. **Commits are immutable**: Once created, never modified
2. **Items can be superseded**: New item references old one
3. **History is append-only**: No rebasing or force-push
4. **Manifest reflects current state**: Regenerated on each commit

---

## Multi-Tenant Federation

### Federation Model

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│                     CENTRAL HUB                                  │
│                  (tenant-wide repo)                             │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  tenant/                                                │    │
│  │    └── policies/                                         │    │
│  │    └── patterns/                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│              ┌───────────────┼───────────────┐                   │
│              │               │               │                   │
│              ▼               ▼               ▼                   │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐          │
│  │   ORG REPO    │ │   ORG REPO    │ │   ORG REPO    │          │
│  │  (Engineering)│ │   (Product)   │ │   (Platform)  │          │
│  │               │ │               │ │               │          │
│  │ orgs/eng/     │ │ orgs/prod/    │ │ orgs/plat/    │          │
│  │   └── adrs/   │ │   └── adrs/   │ │   └── adrs/   │          │
│  └───────┬───────┘ └───────────────┘ └───────────────┘          │
│          │                                                       │
│          ▼                                                       │
│  ┌───────────────┐                                               │
│  │  PROJECT REPO │                                               │
│  │  (Backend API)│                                               │
│  │               │                                               │
│  │ projects/api/ │                                               │
│  │   └── adrs/   │                                               │
│  └───────────────┘                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Federation Config

```typescript
interface FederationConfig {
  /** Central hub repository */
  centralHub?: {
    url: string;
    branch: string;
    syncInterval: string; // e.g., "1h", "6h", "1d"
  };
  
  /** Upstream repositories to sync from */
  upstreams: UpstreamConfig[];
  
  /** Layers to accept from each upstream */
  layerMapping: Record<string, KnowledgeLayer[]>;
}

interface UpstreamConfig {
  /** Unique identifier */
  id: string;
  
  /** Repository URL */
  url: string;
  
  /** Branch to sync */
  branch: string;
  
  /** Layers to pull */
  layers: KnowledgeLayer[];
  
  /** Auto-sync enabled */
  autoSync: boolean;
}
```

### Sync Algorithm

```typescript
async function syncFromUpstream(
  upstream: UpstreamConfig,
  localRepo: KnowledgeRepository
): Promise<SyncResult> {
  // 1. Fetch upstream manifest
  const upstreamManifest = await fetchManifest(upstream);
  
  // 2. Compare with local
  const localManifest = await localRepo.getManifest();
  
  // 3. Compute delta
  const delta = computeDelta(localManifest, upstreamManifest, upstream.layers);
  
  // 4. Apply changes
  const applied: string[] = [];
  const conflicts: ConflictInfo[] = [];
  
  for (const item of delta.added) {
    await localRepo.createItem(item);
    applied.push(item.id);
  }
  
  for (const item of delta.updated) {
    const local = await localRepo.getItem(item.id);
    if (local && local.contentHash !== item.contentHash) {
      // Conflict: both modified
      conflicts.push({ itemId: item.id, local, upstream: item });
    } else {
      await localRepo.updateItem(item);
      applied.push(item.id);
    }
  }
  
  for (const itemId of delta.deleted) {
    await localRepo.markSuperseded(itemId, 'upstream-deleted');
    applied.push(itemId);
  }
  
  // 5. Create sync commit
  await localRepo.commit({
    message: `Sync from upstream: ${upstream.id}`,
    changeType: 'federation',
    affectedItems: applied
  });
  
  return { applied, conflicts };
}
```

### Layer Precedence in Federation

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  When same item exists at multiple layers:                      │
│                                                                  │
│    project  ◄── Highest precedence (most specific)              │
│       │                                                          │
│    team                                                          │
│       │                                                          │
│    org                                                           │
│       │                                                          │
│    tenant ◄── Lowest precedence (least specific)               │
│                                                                  │
│  Project-level items override tenant-level items               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Core Operations

### Operation: Query Knowledge

Search for knowledge items matching criteria.

```typescript
interface QueryKnowledgeInput {
  /** Text search query */
  query?: string;
  
  /** Filter by type */
  type?: KnowledgeType;
  
  /** Filter by layer */
  layer?: KnowledgeLayer;
  
  /** Filter by status */
  status?: KnowledgeStatus | KnowledgeStatus[];
  
  /** Filter by tags */
  tags?: string[];
  
  /** Filter by severity */
  severity?: ConstraintSeverity;
  
  /** Maximum results */
  limit?: number;
  
  /** Layer identifiers for scoping */
  identifiers?: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface QueryKnowledgeOutput {
  /** Matching items (summary only, not full content) */
  items: KnowledgeItemSummary[];
  
  /** Total count */
  totalCount: number;
}

interface KnowledgeItemSummary {
  id: string;
  type: KnowledgeType;
  layer: KnowledgeLayer;
  title: string;
  summary: string;
  status: KnowledgeStatus;
  tags: string[];
  hasConstraints: boolean;
  updatedAt: string;
}
```

### Operation: Get Knowledge Item

Retrieve full content of a knowledge item.

```typescript
interface GetKnowledgeInput {
  /** Item ID */
  id: string;
  
  /** Include constraint details */
  includeConstraints?: boolean;
  
  /** Include version history */
  includeHistory?: boolean;
}

interface GetKnowledgeOutput {
  /** Full knowledge item */
  item: KnowledgeItem | null;
  
  /** Version history (if requested) */
  history?: KnowledgeCommit[];
}
```

### Operation: Check Constraints

Evaluate constraints against a context.

```typescript
interface CheckConstraintsInput {
  /** Files to check */
  files?: FileInfo[];
  
  /** Dependencies to check */
  dependencies?: DependencyInfo[];
  
  /** Specific knowledge items to check (or all if empty) */
  knowledgeItemIds?: string[];
  
  /** Only check certain severity levels */
  minSeverity?: ConstraintSeverity;
  
  /** Layer identifiers */
  identifiers: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface CheckConstraintsOutput {
  /** Check result */
  result: ConstraintCheckResult;
  
  /** Knowledge items that were checked */
  checkedItems: string[];
}
```

### Operation: Propose Knowledge

Create a new knowledge item proposal.

```typescript
interface ProposeKnowledgeInput {
  /** Item type */
  type: KnowledgeType;
  
  /** Title */
  title: string;
  
  /** Summary */
  summary: string;
  
  /** Full content (Markdown) */
  content: string;
  
  /** Target layer */
  layer: KnowledgeLayer;
  
  /** Severity */
  severity?: ConstraintSeverity;
  
  /** Constraints */
  constraints?: Constraint[];
  
  /** Tags */
  tags?: string[];
  
  /** If superseding existing item */
  supersedes?: string;
}

interface ProposeKnowledgeOutput {
  /** Created item (in draft status) */
  item: KnowledgeItem;
  
  /** Generated ID */
  id: string;
  
  /** Path in repository */
  path: string;
}
```

### Operation: Update Knowledge Status

Change the status of a knowledge item.

```typescript
interface UpdateStatusInput {
  /** Item ID */
  id: string;
  
  /** New status */
  status: KnowledgeStatus;
  
  /** Reason for status change */
  reason?: string;
}

interface UpdateStatusOutput {
  /** Updated item */
  item: KnowledgeItem;
  
  /** Commit hash */
  commitHash: string;
}
```

---

## Error Handling

### Error Response Format

```typescript
interface KnowledgeError {
  /** Error code */
  code: KnowledgeErrorCode;
  
  /** Human-readable message */
  message: string;
  
  /** Operation that failed */
  operation: string;
  
  /** Additional context */
  details?: Record<string, unknown>;
}

type KnowledgeErrorCode =
  | 'ITEM_NOT_FOUND'
  | 'INVALID_TYPE'
  | 'INVALID_LAYER'
  | 'INVALID_STATUS_TRANSITION'
  | 'INVALID_CONSTRAINT'
  | 'DUPLICATE_ID'
  | 'MANIFEST_CORRUPTED'
  | 'GIT_ERROR'
  | 'FEDERATION_ERROR'
  | 'VALIDATION_ERROR';
```

### Valid Status Transitions

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  draft ──────────────► proposed                                 │
│    │                      │                                      │
│    │                      ├──────► accepted                     │
│    │                      │           │                          │
│    │                      │           ├──────► deprecated       │
│    │                      │           │                          │
│    │                      │           └──────► superseded       │
│    │                      │                                      │
│    │                      └──────► rejected                     │
│    │                                                             │
│    └─────────────────────► rejected                             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

**Next**: [04-memory-knowledge-sync.md](./04-memory-knowledge-sync.md) - Memory-Knowledge Sync Specification
