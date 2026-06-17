# Aeterna Charter

**The Universal Memory & Knowledge Framework for Enterprise AI Agent Systems**

---

## Mission Statement

> **Aeterna exists to make AI agents smarter, safer, and aligned with organizational knowledge.**

We believe that AI agents are only as good as the context they operate in. Without persistent memory, they forget. Without organizational knowledge, they make decisions in isolation. Without governance, they violate standards and drift from established patterns.

**Aeterna solves this by providing the infrastructure layer that connects AI agents to institutional memory and governed knowledge.**

---

## The Problem We Solve

Modern enterprises deploying AI agents face a critical paradox:

**AI agents are simultaneously powerful and forgetful.**

They can write code, answer questions, and automate workflows—but each session starts from zero. They don't remember that your team prefers PostgreSQL. They don't know your tenant's security policies. They can't learn from mistakes their colleagues made last week.

### The Symptoms

| Symptom | Real-World Impact |
|---------|-------------------|
| **Context Window Limits** | Agent forgets the beginning of a long conversation |
| **Session Isolation** | Every chat starts fresh, no cross-session learning |
| **Knowledge Fragmentation** | ADRs in Confluence, policies in wikis, patterns in Slack |
| **No Institutional Memory** | Team learnings stay locked in individual heads |
| **Policy Violations** | Agent suggests MySQL when PostgreSQL is mandated |
| **Duplicate Work** | Different agents solving the same problems differently |
| **Compliance Gaps** | No audit trail for AI-assisted decisions |

### The Root Cause

AI agent ecosystems lack a **shared memory and knowledge layer** that:
- Persists across sessions
- Spans organizational boundaries
- Enforces governance
- Learns from outcomes

**Aeterna is that layer.**

---

## Our Vision

### The World Without Aeterna

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         FRAGMENTED AI LANDSCAPE                              │
│                                                                              │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐              │
│   │ Agent A │ │ Agent B │ │ Agent C │ │ Agent D │ │ Agent E │              │
│   │         │ │         │ │         │ │         │ │         │              │
│   │ (knows  │ │ (knows  │ │ (knows  │ │ (knows  │ │ (knows  │              │
│   │ nothing)│ │ nothing)│ │ nothing)│ │ nothing)│ │ nothing)│              │
│   └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘              │
│        │           │           │           │           │                    │
│        ▼           ▼           ▼           ▼           ▼                    │
│   ┌─────────────────────────────────────────────────────────────────┐      │
│   │                    CHAOS                                         │      │
│   │  • Conflicting decisions                                         │      │
│   │  • Repeated mistakes                                             │      │
│   │  • Policy violations                                             │      │
│   │  • No shared learning                                            │      │
│   └─────────────────────────────────────────────────────────────────┘      │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### The World With Aeterna

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         UNIFIED AI ECOSYSTEM                                 │
│                                                                              │
│   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐              │
│   │ Agent A │ │ Agent B │ │ Agent C │ │ Agent D │ │ Agent E │              │
│   │         │ │         │ │         │ │         │ │         │              │
│   │ (smart) │ │ (smart) │ │ (smart) │ │ (smart) │ │ (smart) │              │
│   └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘              │
│        │           │           │           │           │                    │
│        └───────────┴───────────┼───────────┴───────────┘                    │
│                                │                                             │
│                                ▼                                             │
│   ┌─────────────────────────────────────────────────────────────────┐      │
│   │                        AETERNA                                   │      │
│   │                                                                  │      │
│   │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │      │
│   │   │    MEMORY    │  │  KNOWLEDGE   │  │  GOVERNANCE  │          │      │
│   │   │              │  │              │  │              │          │      │
│   │   │ • 7 layers   │  │ • ADRs       │  │ • RBAC       │          │      │
│   │   │ • Semantic   │  │ • Policies   │  │ • Constraints│          │      │
│   │   │ • Promotes   │  │ • Patterns   │  │ • Audit      │          │      │
│   │   └──────────────┘  └──────────────┘  └──────────────┘          │      │
│   │                                                                  │      │
│   │   Result: Aligned, Learning, Compliant AI Agents                │      │
│   └─────────────────────────────────────────────────────────────────┘      │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Core Principles

### 1. Memory is Hierarchical

Not all memories are equal. A tenant security policy matters more than a user's preference for tabs over spaces. Aeterna's 7-layer hierarchy ensures precedence:

```
tenant  ←── "No production deployments on Fridays"
   │
org      ←── "Platform teams use Kubernetes"
   │
team     ←── "API team prefers gRPC"
   │
project  ←── "This repo uses Rust 2024"
   │
session  ←── "Currently debugging auth issue"
   │
user     ←── "Alice prefers verbose explanations"
   │
agent    ←── "This agent instance learned X"
```

Higher layers override lower layers. Policies flow down. Learnings bubble up.

### 2. Knowledge is Governed

Organizational decisions shouldn't live in wikis that no one reads. They should be:
- **Version-controlled** (Git-based, immutable history)
- **Enforceable** (constraints that block violations)
- **Discoverable** (semantic search, not folder hunting)
- **Living** (proposals, approvals, deprecation)

Aeterna stores knowledge as first-class citizens:
- **ADRs** (Architecture Decision Records)
- **Policies** (rules that agents must follow)
- **Patterns** (reusable solutions)
- **Specs** (technical specifications)

### 3. Agents Learn from Outcomes

Static memory is just a database. Smart memory improves over time.

Aeterna's **Memory-R1** system (inspired by reinforcement learning):
- Tracks which memories led to successful outcomes
- Rewards useful memories with higher scores
- Promotes high-reward memories to broader scope
- Prunes memories that consistently fail

```
Individual Learning → Session Memory
        │
        │ High reward + approval
        ▼
Team Learning → Team Memory
        │
        │ Proven across teams
        ▼
Organizational Wisdom → Tenant Memory
```

### 4. Context is Adaptive

LLM context windows are finite. Aeterna's **Context Architect** (based on Confucius Code Agent research):
- Pre-computes summaries at multiple depths (sentence, paragraph, detailed)
- Assembles context based on relevance scores
- Adapts to token budgets dynamically
- Separates Agent Experience (AX), User Experience (UX), Developer Experience (DX)

### 5. Integration is Universal

Aeterna doesn't care which AI framework you use:
- **LangChain** → Aeterna adapter
- **AutoGen** → Aeterna adapter
- **CrewAI** → Aeterna adapter
- **OpenCode** → Native plugin + MCP server
- **Custom** → MCP protocol support

One memory. One knowledge base. Many agents.

---

## Value Proposition

### For Engineering Leaders

| Challenge | Without Aeterna | With Aeterna |
|-----------|-----------------|--------------|
| **AI Governance** | No visibility into AI decisions | Full audit trail, constraint enforcement |
| **Knowledge Drift** | Teams diverge from standards | Automatic drift detection + alerts |
| **Compliance** | Manual policy enforcement | Blocking constraints in real-time |
| **ROI** | Each AI session starts from scratch | Accumulated organizational intelligence |

### For Development Teams

| Challenge | Without Aeterna | With Aeterna |
|-----------|-----------------|--------------|
| **Onboarding** | "Read the wiki" (no one does) | AI knows team conventions instantly |
| **Consistency** | Different agents, different answers | Shared patterns + policies |
| **Learning** | Mistakes repeated across team | Error patterns captured + shared |
| **Context Switching** | Re-explain project every session | Agent remembers project history |

### For Individual Developers

| Challenge | Without Aeterna | With Aeterna |
|-----------|-----------------|--------------|
| **Repetition** | "I told you this yesterday" | Agent remembers preferences |
| **Discovery** | "Is there an ADR for this?" | Semantic search finds it |
| **Compliance** | "Oops, we can't use MySQL" | Agent knows before suggesting |
| **Attribution** | "Who decided this?" | Full history with author + rationale |

---

## Capability Roadmap

### Implemented (v1.0)

- [x] **7-Layer Memory Hierarchy** - agent → user → session → project → team → org → tenant
- [x] **Git-Based Knowledge Repository** - ADRs, policies, patterns, specs with version control
- [x] **Constraint DSL** - 6 operators, 5 targets, 3 severity levels
- [x] **Multi-Tenant Governance** - RBAC with Cedar policies
- [x] **Memory-Knowledge Sync Bridge** - Pointer architecture, delta sync
- [x] **MCP Tool Interface** - 8 unified tools for universal compatibility
- [x] **Provider Abstraction** - Mem0, Letta, Qdrant, Pinecone, PostgreSQL
- [x] **Memory-R1 Optimization** - Reward-based pruning and promotion

### In Progress

- [ ] **OpenCode Plugin** - NPM plugin with deep hook integration
- [ ] **Helm Chart** - Production Kubernetes deployment
- [ ] **Multi-Tenant Governance** - Full ReBAC with semantic drift detection

### Planned

- [ ] **Confucius Code Agent (CCA) Capabilities**
  - Context Architect (hierarchical compression)
  - Note-Taking Agent (trajectory distillation)
  - Hindsight Learning (error pattern capture)
  - Meta-Agent (build-test-improve loops)
  - Extension System (typed callbacks)

- [ ] **Reflective Memory Reasoning (MemR³)**
  - Pre-retrieval reasoning
  - Multi-hop memory queries
  - Query refinement

- [ ] **Dynamic Knowledge Graph**
  - Entity extraction
  - Relationship traversal
  - Graph-based reasoning

- [ ] **Radkit A2A Integration**
  - Agent-to-Agent protocol
  - Skill-based discovery
  - Conversation threads

---

## Architecture Philosophy

### Hybrid Deployment

Aeterna supports both local development and centralized enterprise deployment:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         HYBRID DEPLOYMENT MODEL                              │
│                                                                              │
│  LOCAL DEVELOPMENT                         ENTERPRISE SHARED                 │
│  ──────────────────                        ──────────────────                │
│                                                                              │
│  ┌─────────────────┐                      ┌─────────────────┐               │
│  │ Developer       │                      │ Central Aeterna │               │
│  │ Workstation     │                      │ Cluster         │               │
│  │                 │                      │                 │               │
│  │ ┌─────────────┐ │    Sync              │ ┌─────────────┐ │               │
│  │ │ Aeterna     │ │◄──────────────────────►│ │ Knowledge   │ │               │
│  │ │ Local       │ │                      │ │ Repository  │ │               │
│  │ └─────────────┘ │                      │ └─────────────┘ │               │
│  │                 │                      │                 │               │
│  │ • Session mem   │                      │ • Tenant ADRs  │               │
│  │ • User prefs    │                      │ • Org policies  │               │
│  │ • Project ctx   │                      │ • Team patterns │               │
│  └─────────────────┘                      └─────────────────┘               │
│                                                                              │
│  Fast reads, offline capable              Central truth, governed           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Provider Agnostic

Lock-in is the enemy. Aeterna abstracts storage providers:

| Layer | Recommended | Alternatives |
|-------|-------------|--------------|
| Vector Search | Qdrant | Pinecone, Chroma, Weaviate |
| Structured Data | PostgreSQL | MySQL, SQLite (dev only) |
| Cache | Redis | Dragonfly, Valkey |
| Embeddings | OpenAI | Anthropic, Cohere, local models |
| Knowledge Store | Git | S3 (planned), Custom MCP |

### Standards-Based

Aeterna embraces open standards:
- **MCP** (Model Context Protocol) for tool interfaces
- **OpenSpec** for specification-driven development
- **Cedar** for authorization policies
- **OpenTelemetry** for observability
- **Prometheus** for metrics

---

## Target Use Cases

### 1. Enterprise Platform Transformation (Flagship)

**Scenario**: 300 engineers migrating a monolith to microservices over 2-3 years.

**Aeterna provides**:
- ADRs for migration strategy (Strangler Fig, technology selections)
- Policies blocking legacy patterns from spreading
- Patterns for common solutions (Anti-Corruption Layer, Bricks)
- Memory for team learnings (gotchas, workarounds)
- Governance for cross-team alignment

📖 [Full Example: Strangler Fig Migration Guide](docs/examples/strangler-fig-migration.md)

### 2. AI Coding Assistant Fleet

**Scenario**: Tenant deploys AI coding assistants to all engineers.

**Aeterna provides**:
- Shared coding standards enforced across all AI sessions
- User preference persistence (style, verbosity)
- Project context that survives session boundaries
- Audit trail for AI-assisted code changes

### 3. Multi-Agent Orchestration

**Scenario**: Autonomous agents collaborating on complex tasks.

**Aeterna provides**:
- Isolated memory per agent
- Shared knowledge preventing conflicting decisions
- Policy constraints preventing dangerous actions
- A2A communication via Radkit integration

### 4. Compliance-Heavy Industries

**Scenario**: Financial services, healthcare, or government AI deployment.

**Aeterna provides**:
- Immutable audit trail for all AI decisions
- Policy enforcement at query time
- Role-based access to sensitive knowledge
- Drift detection when practices diverge from standards

---

## Technical Excellence

### Performance Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| Working Memory Latency | < 10ms | In-memory Redis |
| Session Memory Latency | < 50ms | Redis with TTL |
| Semantic Search Latency | < 200ms | Qdrant vectors |
| Knowledge Query Latency | < 100ms | PostgreSQL + cache |
| Throughput | > 100 QPS | Horizontal scaling |

### Quality Targets

| Metric | Target | Enforcement |
|--------|--------|-------------|
| Test Coverage | > 80% | CI/CD gate |
| Mutation Score | > 90% | cargo-mutants |
| Documentation | 100% public API | clippy lints |
| Type Safety | Zero `unsafe` | Code review |

### Observability

- **Distributed Tracing**: OpenTelemetry spans for every operation
- **Metrics**: Prometheus counters for memory/knowledge operations
- **Logging**: Structured logs with correlation IDs
- **Dashboards**: Grafana templates for governance visibility

---

## Community & Contribution

### Open Source Commitment

Aeterna is Apache 2.0 licensed. We believe that:
- Memory and knowledge infrastructure should be open
- Vendor lock-in hurts the AI ecosystem
- Community contributions make software better
- Standards emerge from open collaboration

### Contribution Areas

| Area | Description | Skill Level |
|------|-------------|-------------|
| **Adapters** | New AI framework integrations | Intermediate |
| **Providers** | New storage backend support | Intermediate |
| **Tools** | New MCP tools for specific use cases | Beginner |
| **Documentation** | Guides, examples, translations | Beginner |
| **Core** | Memory system, governance engine | Advanced |
| **Research** | CCA, MemR³, Graph Memory | Advanced |

### Getting Involved

1. **Star the repo**: [github.com/kikokikok/aeterna](https://github.com/kikokikok/aeterna)
2. **Read the specs**: Start with [00-overview.md](specs/00-overview.md)
3. **Follow OpenSpec**: All changes require proposals
4. **Join discussions**: GitHub Issues and Discussions
5. **Sign the CLA**: Before your first PR

---

## The Name

**Aeterna** is Latin for "eternal" or "everlasting."

We chose this name because:
- Memory should persist beyond sessions
- Knowledge should outlive individual engineers
- Learnings should accumulate across generations of AI agents
- Organizations should build institutional intelligence that endures

**Aeterna: Where AI agents become wise.**

---

## Acknowledgments

Aeterna builds on the shoulders of giants:

| Project | Contribution |
|---------|--------------|
| [Mem0](https://mem0.ai) | Memory layer concepts, API design |
| [Letta](https://letta.com) | Agent memory patterns, persistence |
| [Cedar](https://cedarpolicy.com) | Authorization language |
| [OpenCode](https://opencode.ai) | AI coding assistant integration |
| [Confucius Code Agent](https://arxiv.org/html/2512.10398v5) | Hierarchical context, note-taking |
| [MemR³](https://arxiv.org) | Reflective memory reasoning |
| [Radkit](https://radkit.rs) | A2A protocol, Rust SDK |

---

## Contact

- **GitHub**: [github.com/kikokikok/aeterna](https://github.com/kikokikok/aeterna)
- **Documentation**: [docs/](docs/)
- **Specifications**: [specs/](specs/)
- **Change Proposals**: [openspec/changes/](openspec/changes/)

---

*"The best time to plant a tree was 20 years ago. The second best time is now."*

*The best time to build organizational AI memory was when you deployed your first agent. The second best time is now.*

**Start with Aeterna today.**
