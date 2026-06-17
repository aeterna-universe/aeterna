# Project Context

## Purpose

OpenSpec-compliant Memory-Knowledge System specification implementation. Provides a universal framework for AI agent memory and knowledge management, supporting multiple ecosystems (LangChain, AutoGen, CrewAI, OpenCode) and pluggable storage backends.

## Tech Stack

- **Primary Language**: Rust (for implementation references)
- **Memory Storage**:
    - Redis 7+ (Working/Session cache)
    - PostgreSQL 16+ (Episodic, Procedural, User, Org metadata)
    - Qdrant 1.12+ (Semantic, Archival vectors)
- **Embedding**: rust-genai 0.4+ (multi-provider: OpenAI, Anthropic, Gemini, Z.AI)
- **API**: OpenSpec v1.0.0 protocol
- **Testing**: TDD/BDD with tarpaulin, proptest, cargo-mutants (MANDATORY 80% MINIMUM COVERAGE)

## Project Conventions

### Code Style

- Use SHALL/MUST for normative requirements
- Scenario format: `#### Scenario: [Name]` with **WHEN**... **THEN**... **AND**...
- Every requirement MUST have at least one scenario
- Follow Rust style guide with `cargo fmt`
- Document public APIs with comprehensive examples

### Architecture Patterns

- **8-Layer Memory Hierarchy**:
    - Working (µs, in-memory Redis)
    - Session (ms, Redis with TTL)
    - Episodic (h, PostgreSQL + Qdrant)
    - Semantic (d, Qdrant vector search)
    - Procedural (w, PostgreSQL facts)
    - User Personal (mo, PostgreSQL + Qdrant)
    - Organization (mo, PostgreSQL + Qdrant)
    - Archival (yr, Qdrant long-term storage)

- **Cross-Layer Queries**: Concurrent queries across multiple memory tiers
- **Knowledge Sources**: Pluggable providers (Git, NotebookLM, Dust.tt, Perplexity, Custom MCP)
- **OpenSpec Protocol**: Standardized endpoints for universal compatibility

### Testing Strategy (CRITICAL)

- **Test-Driven Development**: Write tests before code (RED-GREEN-REFACTOR)
- **Coverage Thresholds**: 80%+ overall (MANDATORY), 85%+ core logic
- **Property-Based Testing**: Critical algorithms (promotion scoring, similarity metrics, confidence aggregation)
- **Mutation Testing**: 90%+ mutants killed for critical code paths
- **Testability**: All external dependencies behind trait abstractions for easy mocking
- **Test Fixtures**: All external service responses have deterministic fixtures
- **BDD Scenarios**: All critical workflows have Gherkin-style scenarios

### Git Workflow

- Feature branches from `main`
- PR reviews required before merge
- Conventional commits format
- Automated CI/CD pipeline

## Domain Context

### Memory System

7-layer hierarchical storage for agent experiences, with vector-based semantic search and provider abstraction (Mem0, Letta, OpenMemory, etc.).

### Knowledge Repository

Git-based versioned storage for organizational decisions (ADRs, Policies, Patterns, Specs) with enforceable constraints and multi-tenant federation (Tenant → Org → Team → Project).

### Sync Bridge

Bidirectional synchronization keeping memory and knowledge aligned via pointer architecture and delta sync.

### Tool Interface

MCP-compatible tool contracts enabling universal compatibility with AI agent frameworks.

## Important Constraints

### Testing Requirements (Non-Negotiable)

- **Minimum 80% test coverage** enforced in CI/CD (STRICT REQUIREMENT)
- **Property-based tests** for all critical algorithms (promotion score, similarity metrics)
- **Mutation testing** with 90%+ mutants killed
- **Test fixtures** for all external API responses (deterministic, versioned)
- **TDD/BDD** from Day 1 - no code without failing tests

### Performance Requirements

- **Query latency**: Working <10ms, Session <50ms, Semantic <200ms
- **Throughput**: >100 QPS, >50 CPS (creates per second)
- **Resource utilization**: CPU <70%, Memory <80%, DB connections <80%

### Knowledge API Compatibility

All implementations MUST provide canonical knowledge endpoints at `/api/v1/knowledge/*` and SHOULD preserve `/openspec/v1/knowledge/*` as a compatibility alias during migration:

1. Discovery endpoint (`GET /api/v1/knowledge`)
2. Query endpoint (`POST /api/v1/knowledge/query`)
3. Create endpoint (`POST /api/v1/knowledge/create`)
4. Update endpoint (`PUT /api/v1/knowledge/{id}`)
5. Delete endpoint (`DELETE /api/v1/knowledge/{id}`)
6. Batch operations endpoint (`POST /api/v1/knowledge/batch`)
7. Streaming endpoint (`GET /api/v1/knowledge/stream`)
8. Metadata operations (`GET /api/v1/knowledge/{id}/metadata`)

Compatibility alias: `/openspec/v1/knowledge/*`

## External Dependencies

### Knowledge Source Integrations

- **Git Repository Provider**: Full version tracking, commit/diff extraction
- **NotebookLM Provider**: .ipynb parsing, cell content extraction
- **External AI Providers**: Dust.tt, Perplexity (API integration)
- **Local File System**: Recursive file indexing
- **Custom MCP Provider Framework**: Universal MCP server integration

### LLM/Embedding Providers (via rust-genai)

- OpenAI, Anthropic, Gemini, xAI, Ollama, Groq, DeepSeek, Cohere, Mimo
- Custom endpoints: Z.AI, AWS Bedrock, Vertex AI

### Data Storage

- PostgreSQL 16+ (metadata, facts, user/org data)
- Qdrant 1.12+ (vector embeddings, semantic search)
- Redis 7+ (working/session cache, message bus)

### Tooling

- OpenSpec CLI 0.16.0+ (spec validation, archiving)
- Cargo workspace management
- Docker Compose (development environment)
- OpenTelemetry (distributed tracing)
- Prometheus (metrics collection)
