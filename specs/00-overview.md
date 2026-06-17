---
title: Memory-Knowledge System Specification
subtitle: A Universal Framework for AI Agent Memory and Knowledge Management
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
license: Apache-2.0
---

# Memory-Knowledge System Specification

## Executive Summary

This specification defines a **universal framework** for managing memory and knowledge in AI agent systems. It is designed to be:

- **Implementation-agnostic**: Works with any AI agent framework
- **Provider-flexible**: Supports multiple storage backends via adapters
- **Ecosystem-extensible**: Integrates with LangChain, AutoGen, CrewAI, OpenCode, and others

The framework addresses a fundamental challenge: **AI agents need both short-term contextual memory and long-term structured knowledge**, and these two systems must work together seamlessly.

## Problem Statement

Modern AI agent systems face several memory and knowledge challenges:

| Challenge | Impact |
|-----------|--------|
| **Context window limits** | Agents forget previous interactions |
| **Knowledge fragmentation** | Organizational knowledge scattered across systems |
| **No memory hierarchy** | All information treated equally regardless of scope |
| **Vendor lock-in** | Switching memory providers requires rewriting code |
| **Knowledge drift** | No versioning or audit trail for decisions |
| **Siloed ecosystems** | Memory from one framework incompatible with others |

## Solution Overview

This specification defines two complementary systems:

### 1. Memory System
Semantic, searchable storage for agent experiences and learnings.

- **Hierarchical layers**: agent → user → session → project → team → org → tenant
- **Vector-based retrieval**: Natural language search across memories
- **Provider abstraction**: Mem0, Letta, Chroma, Pinecone, etc.

### 2. Knowledge Repository
Structured, versioned storage for organizational decisions and policies.

- **Git-based versioning**: Full audit trail, branching, merging
- **Constraint enforcement**: Policies that guide agent behavior
- **Multi-tenant federation**: Tenant → Org → Team → Project layers

### 3. Sync Bridge
Bidirectional synchronization keeping memory and knowledge aligned.

- **Pointer architecture**: Memory stores summaries, knowledge stores full content
- **Delta sync**: Efficient change detection via content hashing
- **Conflict resolution**: Deterministic merge strategies

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        AI AGENT SYSTEM                                   │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │  LangChain  │  │   AutoGen   │  │   CrewAI    │  │  OpenCode   │    │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘    │
│         │                │                │                │            │
│         └────────────────┴────────────────┴────────────────┘            │
│                                   │                                      │
│                                   ▼                                      │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                    UNIFIED TOOL INTERFACE                        │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │   │
│  │  │ memory_add   │  │ memory_search│  │ knowledge_query      │   │   │
│  │  │ memory_get   │  │ memory_delete│  │ knowledge_check      │   │   │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      MEMORY-KNOWLEDGE CORE                               │
│                                                                          │
│  ┌─────────────────────────┐       ┌─────────────────────────┐         │
│  │     MEMORY SYSTEM       │       │   KNOWLEDGE REPOSITORY   │         │
│  │                         │       │                          │         │
│  │  ┌───────────────────┐  │       │  ┌────────────────────┐  │         │
│  │  │   Memory Manager  │  │◄─────►│  │  Knowledge Manager │  │         │
│  │  └─────────┬─────────┘  │ Sync  │  └─────────┬──────────┘  │         │
│  │            │            │ Bridge│            │             │         │
│  │  ┌─────────▼─────────┐  │       │  ┌─────────▼──────────┐  │         │
│  │  │  Layer Resolver   │  │       │  │ Constraint Engine  │  │         │
│  │  │  (7-layer hierarchy)│  │       │  │ (DSL evaluation)   │  │         │
│  │  └─────────┬─────────┘  │       │  └─────────┬──────────┘  │         │
│  │            │            │       │            │             │         │
│  │  ┌─────────▼─────────┐  │       │  ┌─────────▼──────────┐  │         │
│  │  │ Provider Adapter  │  │       │  │   Git Backend      │  │         │
│  │  └───────────────────┘  │       │  └────────────────────┘  │         │
│  └─────────────────────────┘       └─────────────────────────┘         │
└─────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                       PROVIDER ADAPTERS                                  │
│                                                                          │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐           │
│  │  Mem0   │ │  Letta  │ │ Chroma  │ │Pinecone │ │ Qdrant  │           │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘           │
│                                                                          │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐           │
│  │  Redis  │ │PostgreSQL│ │ SQLite  │ │   S3    │ │  Git    │           │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘           │
└─────────────────────────────────────────────────────────────────────────┘
```

## Design Goals

### Must Have (P0)

| Goal | Description |
|------|-------------|
| **Provider Agnostic** | Switch between Mem0, Letta, Chroma without code changes |
| **Ecosystem Agnostic** | Work with LangChain, AutoGen, CrewAI, OpenCode equally |
| **Hierarchical Memory** | 7-layer precedence (agent → tenant) |
| **Versioned Knowledge** | Git-based immutable commits |
| **Constraint DSL** | Declarative policy enforcement |
| **Offline-First** | Work without network, sync when available |

### Should Have (P1)

| Goal | Description |
|------|-------------|
| **Delta Sync** | Efficient change propagation |
| **Conflict Resolution** | Deterministic merge strategies |
| **Multi-Tenant** | Tenant → Org → Team → Project isolation |
| **Audit Trail** | Full history of all changes |

### Nice to Have (P2)

| Goal | Description |
|------|-------------|
| **Real-time Sync** | WebSocket-based live updates |
| **Semantic Deduplication** | Automatic memory consolidation |
| **Cross-Ecosystem Sync** | Share memory between frameworks |

## Non-Goals

This specification explicitly does **not** cover:

- **Agent orchestration**: How agents are scheduled or coordinated
- **LLM selection**: Which models to use for embeddings or generation
- **UI/UX**: User interfaces for managing memory or knowledge
- **Authentication**: Identity provider integration (delegated to ecosystem; Aeterna uses Okta-backed oauth2-proxy for browser users and API keys for service clients)
- **Billing**: Metering or cost allocation for storage

## Specification Index

| # | Document | Description |
|---|----------|-------------|
| 00 | [Overview](./00-overview.md) | This document |
| 01 | [Core Concepts](./01-core-concepts.md) | Glossary, mental model, design principles |
| 02 | [Memory System](./02-memory-system.md) | Layer hierarchy, operations, interfaces |
| 03 | [Knowledge Repository](./03-knowledge-repository.md) | Schema, constraints DSL, versioning |
| 04 | [Memory-Knowledge Sync](./04-memory-knowledge-sync.md) | Pointer architecture, delta sync |
| 05 | [Adapter Architecture](./05-adapter-architecture.md) | Provider abstraction, ecosystem adapters |
| 06 | [Tool Interface](./06-tool-interface.md) | Generic tool contracts |
| 07 | [Configuration](./07-configuration.md) | Config schema, environment abstraction |
| 08 | [Deployment](./08-deployment.md) | Self-hosted vs cloud patterns |
| 09 | [Migration](./09-migration.md) | Data portability, import/export |

## Ecosystem Adapters

Adapters bridge this specification to specific AI agent frameworks:

| Ecosystem | Adapter | Status |
|-----------|---------|--------|
| OpenCode | `@aeterna-org/opencode-plugin` | Released |
| LangChain | `adapters/langchain/` | Planned |
| AutoGen | `adapters/autogen/` | Planned |
| CrewAI | `adapters/crewai/` | Planned |

## Quick Start

### For Specification Readers

1. Start with [Core Concepts](./core-concepts) for terminology
2. Read [Memory System](./memory-system) for the memory model
3. Read [Knowledge Repository](./knowledge-repository) for knowledge management
4. See [Adapter Architecture](./adapter-architecture) for integration patterns

### For Implementers

1. Review [Tool Interface](./tool-interface) for required tool contracts
2. Check [Configuration](./configuration) for configuration schema
3. Reference [Examples](../examples/strangler-fig-migration) for implementation examples

### For Ecosystem Integrators

1. Read [Adapter Architecture](./adapter-architecture) for adapter interface
2. Check the [OpenCode Integration](../integrations/opencode-integration) as a reference
3. Implement the `EcosystemAdapter` interface for your framework

## Versioning

This specification follows [Semantic Versioning](https://semver.org/):

- **MAJOR**: Breaking changes to core interfaces
- **MINOR**: New features, backward compatible
- **PATCH**: Clarifications, typo fixes

Current version: **0.1.0** (Draft)

## Contributing

Contributions welcome via:

1. Issues for clarifications or suggestions
2. Pull requests for specification improvements
3. Adapter implementations for new ecosystems

## License

Apache License 2.0 — See the [GitHub repository](https://github.com/kikokikok/aeterna) for details.

---

**Next**: [Core Concepts](./core-concepts) - Core Concepts and Terminology
