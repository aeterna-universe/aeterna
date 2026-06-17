# CCA: Confucius Code Agent Capabilities

## Overview

The Confucius Code Agent (CCA) capabilities in Aeterna implement advanced agent intelligence based on research from Meta AI and Harvard University. CCA provides production-ready implementations of four specialized agents that work together to create a self-improving AI system with hierarchical memory, trajectory learning, error pattern recognition, and autonomous refinement loops.

## Research Foundation

CCA is based on the paper "Confucius: Iterative Tool Learning from Introspection Feedback by Easy-to-Difficult Curriculum" (arxiv.org/html/2512.10398v5). The research demonstrates that specialized agent capabilities working in concert can achieve significant performance improvements:

- **+7.6% improvement** from rich tool handling and semantic context compression
- **Hierarchical working memory** enables efficient context management within token budgets
- **Trajectory distillation** captures agent learnings as reusable knowledge
- **Error pattern recognition** prevents repeated failures across sessions

## The Four Core Components

### 1. Context Architect

The Context Architect compresses hierarchical memory into efficient context summaries that fit within token budgets. It queries multiple memory layers simultaneously and intelligently prioritizes the most relevant information.

**Key Features:**
- Hierarchical compression across 7 memory layers (Tenant → Org → Team → Project → Session → User → Agent)
- Token budget management with configurable limits (100-32,000 tokens)
- Relevance scoring with semantic deduplication
- Parallel layer queries with early termination
- Caching with staleness validation

**Use Cases:**
- Assembling context for LLM prompts without exceeding token limits
- Prioritizing recent session context over historical organizational knowledge
- Balancing breadth (many layers) vs depth (detailed single-layer information)

### 2. Note-Taking Agent

The Note-Taking Agent captures trajectory events during agent execution and distills them into Markdown documentation. This enables agents to learn from their own behavior and share successful patterns with other agents.

**Key Features:**
- Automatic trajectory capture for tool calls and decisions
- Configurable sampling rates to control overhead (1-100%)
- Batch processing with async queue (default: 1000 events, 10 batch size)
- Sensitive pattern detection and redaction
- Manual trigger support for critical events
- Distillation to structured Markdown notes

**Use Cases:**
- Recording successful problem-solving approaches
- Documenting tool usage patterns that worked
- Creating knowledge artifacts from agent sessions
- Building organizational memory from individual learnings

### 3. Hindsight Learning

Hindsight Learning captures errors and failed attempts, analyzes patterns semantically, and suggests resolutions based on past successful recoveries. This creates a growing knowledge base of how to handle errors.

**Key Features:**
- Semantic error signature matching (type, message, context patterns)
- Resolution tracking with success rates
- Auto-capture of all errors or manual selective capture
- Promotion threshold (default 0.8) for elevating working solutions
- Vector-based similarity search for related error patterns

**Use Cases:**
- Finding solutions to recurring error types
- Learning from failed build/test attempts
- Sharing error resolutions across teams
- Reducing repeated debugging of similar issues

### 4. Meta-Agent

The Meta-Agent implements build-test-improve loops for autonomous agent refinement. It runs multiple iterations of task execution, evaluates outcomes, and adjusts strategies based on feedback.

**Key Features:**
- Configurable iteration limits (default: 3 iterations)
- Per-phase timeouts (build: 120s, test: 60s, iteration: 300s)
- Automatic escalation on repeated failures
- Integration with Note-Taking and Hindsight for learning
- State tracking across iterations

**Use Cases:**
- Self-correcting code generation with automated testing
- Iterative improvement of tool call sequences
- Autonomous debugging with multiple attempted fixes
- Quality gates before promoting solutions to production

## Production Readiness Features

Unlike research prototypes, Aeterna's CCA implementation includes critical production requirements:

### Cost Control
- Token budget enforcement at every context assembly
- Configurable overhead budgets for Note-Taking (default: 5ms per event)
- Sampling modes to reduce capture volume in high-throughput scenarios

### Time Budgets
- Assembly timeout for Context Architect (default: 100ms)
- Per-phase timeouts for Meta-Agent loops
- Callback timeouts for extension execution (default: 5s)
- Early termination when token budgets are satisfied

### Staleness Validation
- Cache TTL enforcement (default: 300 seconds)
- Three staleness policies: ServeStaleWarn, RegenerateBlocking, RegenerateAsync
- Automatic invalidation on memory updates

### Deduplication
- Semantic similarity detection during context assembly
- Relevance threshold filtering (default: 0.3)
- Error signature matching to avoid duplicate capture

### Latency Control
- Parallel layer queries by default (configurable)
- Batch flush intervals for Note-Taking (default: 100ms)
- Queue-based async processing to avoid blocking
- LRU eviction for extension state (default: 1MB limit)

## Hybrid Execution Model

CCA capabilities operate across two execution contexts:

### Client-Side (OpenCode Plugin)
- Extension system for customizing agent behavior
- Prompt additions and tool configuration
- State management with compression (zstd)
- Callback hooks for input/output transformation

### Server-Side (Aeterna Core)
- Memory layer storage and retrieval
- Context assembly and compression logic
- Trajectory capture and distillation
- Hindsight query engine and Meta-Agent loops

This hybrid model enables:
- Low-latency client-side processing for real-time needs
- Heavy computation on server for memory operations
- Flexible deployment (client-only, server-only, or both)
- Clear separation of concerns for testing and scaling

## Integration with Aeterna Memory System

CCA capabilities are deeply integrated with Aeterna's 7-layer memory hierarchy:

1. **Context Architect** queries all layers and respects layer priorities
2. **Note-Taking** stores distilled notes as Project/Team/Org layer memories
3. **Hindsight** promotes successful resolutions from Agent layer to Team/Org layers
4. **Meta-Agent** leverages full memory context for each iteration

This integration means CCA benefits from:
- Multi-tenant isolation at the Tenant level
- Memory-R1 reward signals for promoting valuable learnings
- Graph layer for discovering related memories and error patterns
- Policy enforcement to prevent invalid knowledge capture

## Performance Characteristics

Based on the research and Aeterna's implementation:

| Component | Operation | Typical Latency | Scalability |
|-----------|-----------|-----------------|-------------|
| Context Architect | Assemble 4000 tokens | \<100ms | Parallel queries across layers |
| Note-Taking | Capture event | \<5ms | Async queue, 1000 events/buffer |
| Hindsight | Query errors | \<50ms | Vector search, cached signatures |
| Meta-Agent | Single iteration | 60-300s | Per-loop isolation |

Performance tuning options:
- Adjust token budgets based on model context windows
- Configure sampling rates for high-traffic scenarios
- Enable/disable parallel queries based on infrastructure
- Tune batch sizes and flush intervals for throughput vs latency

## Comparison to Research Prototype

Aeterna's CCA implementation extends the original research:

| Feature | Research Prototype | Aeterna Implementation |
|---------|-------------------|------------------------|
| Context Compression | Yes | Yes, with hierarchical layers |
| Note-Taking | Yes | Yes, with async batching |
| Hindsight Learning | Yes | Yes, with semantic promotion |
| Meta-Agent | Yes | Yes, with timeout controls |
| Production Controls | No | Cost, time, staleness, latency |
| Multi-Tenancy | No | Tenant/Org/Team isolation |
| Extension System | No | Client-side customization |
| State Management | No | LRU caching with compression |
| Policy Integration | No | Cedar/Permit.io enforcement |

## Getting Started

To enable CCA capabilities in your Aeterna deployment:

1. Configure CCA in `config/aeterna.toml` (see [Configuration Guide](configuration.md))
2. Use the 4 MCP tools to interact with CCA components (see [API Reference](api-reference.md))
3. Optionally create custom extensions (see [Extension Guide](extension-guide.md))
4. Review the architecture for deployment considerations (see [Architecture](architecture.md))

## Next Steps

- [Architecture](architecture.md) - Understand the hybrid execution model and data flows
- [Configuration](configuration.md) - Learn all configuration options with examples
- [API Reference](api-reference.md) - Explore the 4 MCP tools for CCA
- [Extension Guide](extension-guide.md) - Build custom extensions for specialized behavior
