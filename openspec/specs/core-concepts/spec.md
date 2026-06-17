---
title: Core Concepts and Terminology
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 00-overview.md
  - 02-memory-system.md
  - 03-knowledge-repository.md
---

# Core Concepts and Terminology

This document establishes the foundational vocabulary and mental models for the Memory-Knowledge System. All subsequent specifications reference these definitions.

## Table of Contents

1. [Glossary](#glossary)
2. [Mental Model](#mental-model)
3. [Design Principles](#design-principles)
4. [System Boundaries](#system-boundaries)
5. [Data Flow Patterns](#data-flow-patterns)

---

## Glossary

### Core Entities

| Term | Definition |
|------|------------|
| **Memory** | A discrete unit of semantic information stored for retrieval. Contains content, metadata, and vector embedding. |
| **Knowledge** | A structured, versioned artifact representing organizational decisions, policies, patterns, or specifications. |
| **Memory Layer** | A hierarchical scope for memory isolation (agent, user, session, project, team, org, tenant). |
| **Knowledge Layer** | A hierarchical scope for knowledge federation (project, team, org, tenant). |
| **Provider** | A storage backend that persists memories (e.g., Mem0, Letta, Chroma, Pinecone). |
| **Ecosystem** | An AI agent framework that consumes this specification (e.g., LangChain, AutoGen, CrewAI, OpenCode). |
| **Adapter** | A bridge component that translates between this specification and a specific provider or ecosystem. |

### Memory System Terms

| Term | Definition |
|------|------------|
| **Memory Entry** | A single memory record with id, content, embedding, metadata, and layer assignment. |
| **Memory Search** | Vector-based semantic retrieval of memories matching a query. |
| **Layer Precedence** | Rules determining which layer's memory takes priority when conflicts exist. |
| **Memory Consolidation** | Process of merging similar memories to reduce redundancy. |
| **Memory Decay** | Optional mechanism to reduce relevance of old memories over time. |
| **Rehydration** | Loading memories into agent context at session start. |

### Knowledge Repository Terms

| Term | Definition |
|------|------------|
| **Knowledge Item** | A single versioned artifact (ADR, policy, pattern, spec). |
| **Knowledge Type** | Category of knowledge: `adr`, `policy`, `pattern`, `spec`. |
| **Knowledge Commit** | An immutable snapshot of a knowledge item at a point in time. |
| **Constraint** | A declarative rule attached to knowledge that guides agent behavior. |
| **Constraint DSL** | Domain-specific language for expressing constraints. |
| **Manifest** | Index file listing all knowledge items with metadata. |
| **Federation** | Multi-tenant knowledge sharing across organizational boundaries. |

### Sync System Terms

| Term | Definition |
|------|------------|
| **Sync Bridge** | Component that synchronizes memory and knowledge systems. |
| **Pointer** | Memory entry that references knowledge item (stores summary, links to full content). |
| **Delta Sync** | Efficient synchronization using content hashing to detect changes. |
| **Content Hash** | SHA-256 hash of knowledge item content for change detection. |
| **Sync State** | Persistent record of last sync timestamp and hashes. |
| **Conflict** | Situation where memory and knowledge have diverged. |

### Infrastructure Terms

| Term | Definition |
|------|------------|
| **Vector Store** | Database optimized for similarity search (Qdrant, Pinecone, Chroma). |
| **Embedding Model** | ML model that converts text to vector representations. |
| **Central Hub** | Git repository containing federated knowledge from multiple tenants. |
| **Upstream** | Parent layer in federation hierarchy (e.g., org is upstream of project). |
| **Downstream** | Child layer in federation hierarchy (e.g., project is downstream of org). |

---

## Mental Model

### The Two-System Architecture

Think of the Memory-Knowledge System as **two complementary databases**:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  ┌──────────────────────┐       ┌──────────────────────┐       │
│  │    MEMORY SYSTEM     │       │  KNOWLEDGE REPOSITORY │       │
│  │                      │       │                       │       │
│  │  "What I remember"   │       │  "What I should know" │       │
│  │                      │       │                       │       │
│  │  • Semantic search   │       │  • Versioned content  │       │
│  │  • Fast retrieval    │       │  • Full audit trail   │       │
│  │  • Fuzzy matching    │       │  • Constraint rules   │       │
│  │  • Session context   │       │  • Org-wide policies  │       │
│  │                      │       │                       │       │
│  │  Vector DB           │       │  Git Repository       │       │
│  └──────────┬───────────┘       └───────────┬───────────┘       │
│             │                               │                    │
│             └───────────┬───────────────────┘                    │
│                         │                                        │
│                         ▼                                        │
│                   SYNC BRIDGE                                    │
│             (Keeps them aligned)                                 │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Memory: The "Fast Brain"

Memory is like **working memory** in human cognition:

- **Quick access**: Optimized for fast semantic retrieval
- **Fuzzy matching**: "Find things similar to X"
- **Contextual**: Scoped to sessions, users, projects
- **Ephemeral-ish**: Can be consolidated, pruned, or decayed
- **Summaries**: Stores digestible chunks, not full documents

**Use memory when**: You need to quickly recall relevant context during agent execution.

### Knowledge: The "Reference Library"

Knowledge is like **long-term institutional memory**:

- **Authoritative**: Single source of truth
- **Versioned**: Full history of changes
- **Structured**: Typed artifacts with schemas
- **Enforced**: Constraints that guide behavior
- **Permanent**: Immutable commits, never lost

**Use knowledge when**: You need to establish, enforce, or reference organizational decisions.

### The Pointer Pattern

Memory and knowledge are connected via **pointers**:

```
┌─────────────────────────────────────────────────────────────────┐
│                        MEMORY ENTRY                              │
│                                                                  │
│  id: "mem_abc123"                                               │
│  content: "Use PostgreSQL for all new services (ADR-042)"       │
│  embedding: [0.12, -0.34, 0.56, ...]                            │
│  metadata:                                                       │
│    type: "knowledge_pointer"                                    │
│    sourceType: "adr"                                            │
│    sourceId: "adr-042-database-selection"       ─────┐          │
│    contentHash: "sha256:abc123..."                   │          │
│                                                      │          │
└──────────────────────────────────────────────────────│──────────┘
                                                       │
                                                       ▼
┌─────────────────────────────────────────────────────────────────┐
│                      KNOWLEDGE ITEM                              │
│                                                                  │
│  id: "adr-042-database-selection"                               │
│  type: "adr"                                                    │
│  title: "Database Selection for New Services"                   │
│  content: |                                                     │
│    ## Context                                                   │
│    We need to standardize on a database for new services...     │
│    (500+ lines of detailed rationale)                           │
│                                                                  │
│  constraints:                                                    │
│    - operator: must_use                                         │
│      target: dependency                                         │
│      pattern: "postgresql"                                      │
│      severity: block                                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Why pointers?**

1. **Memory stays lean**: Summaries fit in context windows
2. **Knowledge stays complete**: Full content always available
3. **Sync is efficient**: Compare hashes, not content
4. **Search is fast**: Vector search on summaries
5. **Audit is preserved**: Knowledge history intact

---

## Design Principles

### 1. Separation of Concerns

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   RETRIEVAL     │     │    STORAGE      │     │   ENFORCEMENT   │
│                 │     │                 │     │                 │
│ "Find relevant  │     │ "Persist and    │     │ "Apply rules    │
│  information"   │     │  version data"  │     │  to behavior"   │
│                 │     │                 │     │                 │
│ Memory System   │     │ Both Systems    │     │ Knowledge Repo  │
└─────────────────┘     └─────────────────┘     └─────────────────┘
```

Each system has a clear responsibility:
- **Memory**: Retrieval and context augmentation
- **Knowledge**: Storage, versioning, and constraint enforcement
- **Sync Bridge**: Consistency between systems

### 2. Provider Agnosticism

The specification defines **interfaces, not implementations**:

```
┌─────────────────────────────────────────────────────────────────┐
│                    APPLICATION CODE                              │
│                                                                  │
│  const memory = await memoryManager.search("database policies") │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    PROVIDER ADAPTER                              │
│                                                                  │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐            │
│  │  Mem0   │  │  Letta  │  │ Chroma  │  │Pinecone │            │
│  │ Adapter │  │ Adapter │  │ Adapter │  │ Adapter │            │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

Switch providers by changing configuration, not code.

### 3. Ecosystem Agnosticism

The specification works with **any AI agent framework**:

```
┌─────────────────────────────────────────────────────────────────┐
│                    ECOSYSTEM ADAPTERS                            │
│                                                                  │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐    │
│  │ LangChain │  │  AutoGen  │  │  CrewAI   │  │ OpenCode  │    │
│  │  Adapter  │  │  Adapter  │  │  Adapter  │  │  Adapter  │    │
│  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘    │
│        │              │              │              │            │
│        └──────────────┴──────────────┴──────────────┘            │
│                              │                                   │
│                              ▼                                   │
│               UNIFIED MEMORY-KNOWLEDGE API                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 4. Hierarchical Layering

Both memory and knowledge use **hierarchical layers** with precedence:

```
MEMORY LAYERS (7 levels)              KNOWLEDGE LAYERS (4 levels)
─────────────────────────             ──────────────────────────

agent    ◄── Most specific            project  ◄── Most specific
   │                                      │
user                                   team
   │                                      │
session                                org
   │                                      │
project                                tenant ◄── Least specific
   │
team
   │
org
   │
tenant  ◄── Least specific
```

**Precedence rule**: More specific layers override less specific layers.

### 5. Immutability Where It Matters

| System | Mutability | Rationale |
|--------|------------|-----------|
| **Memory** | Mutable | Consolidation, decay, updates needed |
| **Knowledge Commits** | Immutable | Audit trail, reproducibility |
| **Sync State** | Mutable | Tracks last sync position |

### 6. Offline-First

The system must work without network connectivity:

```
┌─────────────────────────────────────────────────────────────────┐
│                     OFFLINE OPERATION                            │
│                                                                  │
│  1. Local memory cache serves reads                             │
│  2. Local knowledge repo serves reads                           │
│  3. Writes queue for later sync                                 │
│  4. Conflict resolution on reconnect                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 7. Graceful Degradation

If one system is unavailable, the other continues:

| Scenario | Behavior |
|----------|----------|
| Memory unavailable | Knowledge still enforces constraints |
| Knowledge unavailable | Memory still provides context |
| Sync unavailable | Both operate independently, sync later |
| Provider unavailable | Fall back to local cache if configured |

---

## System Boundaries

### What This Specification Covers

```
┌─────────────────────────────────────────────────────────────────┐
│                      IN SCOPE                                    │
│                                                                  │
│  • Memory storage and retrieval interfaces                      │
│  • Knowledge repository structure and versioning                │
│  • Constraint DSL and enforcement rules                         │
│  • Sync bridge algorithm and state management                   │
│  • Provider adapter interface contracts                         │
│  • Ecosystem adapter interface contracts                        │
│  • Tool/function calling contracts                              │
│  • Configuration schema and validation                          │
│  • Data portability and migration                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### What This Specification Does NOT Cover

```
┌─────────────────────────────────────────────────────────────────┐
│                     OUT OF SCOPE                                 │
│                                                                  │
│  • Agent orchestration (how agents are scheduled)               │
│  • LLM selection (which models to use)                          │
│  • Embedding model selection (delegated to provider)            │
│  • User authentication (delegated to ecosystem)                 │
│  • Authorization/permissions (delegated to ecosystem)           │
│  • User interface design                                        │
│  • Billing and metering                                         │
│  • Network transport protocols                                  │
│  • Specific provider implementation details                     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Interface Boundaries

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  ECOSYSTEM                                                       │
│  (LangChain, AutoGen, etc.)                                     │
│                                                                  │
│       │                                                          │
│       │ Ecosystem Adapter Interface                              │
│       ▼                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                                                          │    │
│  │  MEMORY-KNOWLEDGE CORE (This Specification)              │    │
│  │                                                          │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│       │ Provider Adapter Interface                               │
│       ▼                                                          │
│                                                                  │
│  PROVIDER                                                        │
│  (Mem0, Letta, Chroma, etc.)                                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Flow Patterns

### Pattern 1: Memory Write

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Agent                                                           │
│    │                                                             │
│    │ 1. "Remember: User prefers TypeScript"                     │
│    ▼                                                             │
│  Ecosystem Adapter                                               │
│    │                                                             │
│    │ 2. Determine layer (user), format content                  │
│    ▼                                                             │
│  Memory Manager                                                  │
│    │                                                             │
│    │ 3. Generate embedding, validate metadata                   │
│    ▼                                                             │
│  Provider Adapter                                                │
│    │                                                             │
│    │ 4. Persist to vector store                                 │
│    ▼                                                             │
│  Vector Store (Qdrant, Pinecone, etc.)                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Pattern 2: Memory Search

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Agent                                                           │
│    │                                                             │
│    │ 1. "What are the user's preferences?"                      │
│    ▼                                                             │
│  Ecosystem Adapter                                               │
│    │                                                             │
│    │ 2. Determine layers to search, build query                 │
│    ▼                                                             │
│  Memory Manager                                                  │
│    │                                                             │
│    │ 3. Search each layer, apply precedence                     │
│    ▼                                                             │
│  Provider Adapter                                                │
│    │                                                             │
│    │ 4. Vector similarity search                                │
│    ▼                                                             │
│  Results                                                         │
│    │                                                             │
│    │ 5. Merge, rank, return                                     │
│    ▼                                                             │
│  Agent Context                                                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Pattern 3: Knowledge Query

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Agent                                                           │
│    │                                                             │
│    │ 1. "What policies apply to database selection?"            │
│    ▼                                                             │
│  Knowledge Manager                                               │
│    │                                                             │
│    │ 2. Search knowledge repository                             │
│    ▼                                                             │
│  Git Backend                                                     │
│    │                                                             │
│    │ 3. Find matching items across layers                       │
│    ▼                                                             │
│  Results + Constraints                                           │
│    │                                                             │
│    │ 4. Return items with applicable constraints                │
│    ▼                                                             │
│  Agent Context (with enforcement rules)                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Pattern 4: Constraint Check

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Agent                                                           │
│    │                                                             │
│    │ 1. "Adding MySQL dependency"                               │
│    ▼                                                             │
│  Constraint Engine                                               │
│    │                                                             │
│    │ 2. Load applicable constraints from knowledge              │
│    ▼                                                             │
│  Evaluation                                                      │
│    │                                                             │
│    │ 3. Check: must_use postgresql (severity: block)            │
│    ▼                                                             │
│  Violation Detected                                              │
│    │                                                             │
│    │ 4. Return: BLOCKED - "Use PostgreSQL per ADR-042"          │
│    ▼                                                             │
│  Agent (stops action, explains constraint)                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Pattern 5: Memory-Knowledge Sync

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Sync Trigger (schedule, manual, or event)                      │
│    │                                                             │
│    │ 1. Initiate sync                                           │
│    ▼                                                             │
│  Sync Bridge                                                     │
│    │                                                             │
│    │ 2. Load last sync state (timestamps, hashes)               │
│    ▼                                                             │
│  Delta Detection                                                 │
│    │                                                             │
│    │ 3. Compare knowledge hashes vs memory pointers             │
│    ▼                                                             │
│  Changes: [added: 2, updated: 1, deleted: 0]                    │
│    │                                                             │
│    │ 4. For each change:                                        │
│    │    - Added: Create memory pointer                          │
│    │    - Updated: Update memory content + hash                 │
│    │    - Deleted: Mark memory as orphaned                      │
│    ▼                                                             │
│  Commit Sync State                                               │
│    │                                                             │
│    │ 5. Persist new hashes and timestamp                        │
│    ▼                                                             │
│  Done                                                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Key Invariants

These invariants must **always** hold:

### Memory Invariants

1. Every memory entry has a unique `id`
2. Every memory entry belongs to exactly one layer
3. Memory content length does not exceed provider limits
4. Memory metadata is valid JSON
5. Embeddings are generated by consistent model

### Knowledge Invariants

1. Every knowledge item has a unique `id`
2. Knowledge commits are immutable once created
3. Every knowledge item has a `type` from allowed set
4. Constraints attached to knowledge are valid DSL
5. Manifest reflects current state of repository

### Sync Invariants

1. Pointers always reference existing knowledge items (or are marked orphaned)
2. Content hash in pointer matches knowledge item at sync time
3. Sync state accurately reflects last successful sync
4. Conflicts are detected and reported, never silently overwritten

### Layer Invariants

1. Layer precedence is deterministic (same input → same output)
2. More specific layers always override less specific
3. Layer assignment is explicit, never inferred

---

**Next**: [02-memory-system.md](./02-memory-system.md) - Memory System Specification
