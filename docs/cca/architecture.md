# CCA Architecture

## Hybrid Execution Model

CCA capabilities operate across two distinct execution contexts: client-side extensions (OpenCode Plugin) and server-side agents (Aeterna Core). This hybrid architecture enables low-latency local processing while leveraging centralized memory storage and compute.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          CLIENT SIDE (OpenCode Plugin)                       │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                         Extension System                                │ │
│  │                                                                          │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐│ │
│  │  │ Extension A  │  │ Extension B  │  │ Extension C  │  │ Extension N  ││ │
│  │  │              │  │              │  │              │  │              ││ │
│  │  │ • Callbacks  │  │ • Callbacks  │  │ • Callbacks  │  │ • Callbacks  ││ │
│  │  │ • State (1MB)│  │ • State (1MB)│  │ • State (1MB)│  │ • State (1MB)││ │
│  │  │ • Priority 1 │  │ • Priority 2 │  │ • Priority 5 │  │ • Priority 10││ │
│  │  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘│ │
│  │                                                                          │ │
│  │  Execution Order: Priority DESC (N → C → B → A)                         │ │
│  └──────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                    Extension Registry & Executor                        │ │
│  │                                                                          │ │
│  │  • ExtensionRegistration: id, callbacks, prompt_additions, tool_config │ │
│  │  • ExtensionExecutor: orchestrates callback chains                      │ │
│  │  • ExtensionStateStore: Redis-backed persistence (zstd compression)    │ │
│  │  • ExtensionStateLimiter: LRU eviction, 1MB per extension              │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                         Callback Hooks                                  │ │
│  │                                                                          │ │
│  │  1. on_input_messages(messages)  →  Transform user/system messages     │ │
│  │  2. on_plain_text(text)           →  Process text before parsing       │ │
│  │  3. on_tag(tag, content)          →  Handle custom tags                │ │
│  │  4. on_llm_output(output)         →  Post-process LLM responses        │ │
│  │                                                                          │ │
│  │  Each callback:                                                          │ │
│  │  • Receives ExtensionContext (tenant, session, tools, state)           │ │
│  │  • Timeout: 5s default (configurable)                                   │ │
│  │  • Chained in priority order                                            │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│                                      ▲                                       │
│                                      │ MCP Tools API                         │
│                                      ▼                                       │
└──────────────────────────────────────┼──────────────────────────────────────┘
                                       │
                                       │ HTTP/JSON-RPC
                                       │
┌──────────────────────────────────────▼──────────────────────────────────────┐
│                         SERVER SIDE (Aeterna Core)                           │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                          Context Architect                              │ │
│  │                                                                          │ │
│  │  ┌──────────────────────────────────────────────────────────────────┐  │ │
│  │  │          Hierarchical Context Assembly Pipeline                  │  │ │
│  │  │                                                                    │  │ │
│  │  │   Query  →  Parallel Layer Retrieval  →  Relevance Scoring  →   │  │ │
│  │  │                                                                    │  │ │
│  │  │   Deduplication  →  Token Budget Fitting  →  Context Summary     │  │ │
│  │  └──────────────────────────────────────────────────────────────────┘  │ │
│  │                                                                          │ │
│  │  Memory Layers (queried in parallel if enabled):                        │ │
│  │  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐           │ │
│  │  │ Agent  │  │  User  │  │Session │  │Project │  │  Team  │ ...       │ │
│  │  └───┬────┘  └───┬────┘  └───┬────┘  └───┬────┘  └───┬────┘           │ │
│  │      │           │           │           │           │                  │ │
│  │      └───────────┴───────────┴───────────┴───────────┘                  │ │
│  │                              │                                           │ │
│  │                    ┌─────────▼──────────┐                               │ │
│  │                    │  Token Budget Mgr  │                               │ │
│  │                    │  • Min relevance   │                               │ │
│  │                    │  • Early term.     │                               │ │
│  │                    │  • Cache (300s TTL)│                               │ │
│  │                    └────────────────────┘                               │ │
│  │                                                                          │ │
│  │  Output: AssembledContext { total_tokens, content, layers_included }   │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                          Note-Taking Agent                              │ │
│  │                                                                          │ │
│  │  ┌──────────────────────────────────────────────────────────────────┐  │ │
│  │  │           Trajectory Capture & Distillation Pipeline              │  │ │
│  │  │                                                                    │  │ │
│  │  │   Tool Call  →  Event Capture  →  Async Queue  →  Batch Write  → │  │ │
│  │  │                                                                    │  │ │
│  │  │   Accumulate (Threshold)  →  Distill to Markdown  →  Store as    │  │ │
│  │  │                                                     Memory         │  │ │
│  │  └──────────────────────────────────────────────────────────────────┘  │ │
│  │                                                                          │ │
│  │  Capture Modes:                                                          │ │
│  │  • All: Capture every event                                             │ │
│  │  • Sampled: Capture 1 in N events (configurable rate)                  │ │
│  │  • ErrorsOnly: Capture only failures                                    │ │
│  │  • Disabled: No capture                                                 │ │
│  │                                                                          │ │
│  │  Queue Configuration:                                                    │ │
│  │  • Size: 1000 events (default)                                          │ │
│  │  • Batch: 10 events (default)                                           │ │
│  │  • Flush: 100ms (default)                                               │ │
│  │  • Overhead budget: 5ms per event                                       │ │
│  │                                                                          │ │
│  │  Output: Distilled notes stored in Project/Team/Org memory layers      │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                         Hindsight Learning                              │ │
│  │                                                                          │ │
│  │  ┌──────────────────────────────────────────────────────────────────┐  │ │
│  │  │             Error Pattern Recognition Pipeline                    │  │ │
│  │  │                                                                    │  │ │
│  │  │   Error  →  Signature Extract  →  Semantic Search  →  Match     │  │ │
│  │  │                                                                    │  │ │
│  │  │   Resolution Ranking  →  Promotion (if threshold met)  →  Store  │  │ │
│  │  └──────────────────────────────────────────────────────────────────┘  │ │
│  │                                                                          │ │
│  │  Error Signature:                                                        │ │
│  │  • error_type: "TypeError", "BuildError", etc.                          │ │
│  │  • message_pattern: Regex or substring                                  │ │
│  │  • context_patterns: Stack trace patterns, file paths                   │ │
│  │  • embedding: Vector for semantic matching                              │ │
│  │                                                                          │ │
│  │  Resolution Tracking:                                                    │ │
│  │  • description: What was done to fix                                    │ │
│  │  • success_rate: % of times this resolution worked                      │ │
│  │  • application_count: How many times applied                            │ │
│  │                                                                          │ │
│  │  Promotion: If success_rate >= 0.8, promote to broader layer           │ │
│  │  Output: Matched resolutions with scores                                │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                             Meta-Agent                                  │ │
│  │                                                                          │ │
│  │  ┌──────────────────────────────────────────────────────────────────┐  │ │
│  │  │              Build-Test-Improve Loop (BTI)                        │  │ │
│  │  │                                                                    │  │ │
│  │  │   Iteration 1:                                                     │  │ │
│  │  │   BUILD (120s timeout)  →  TEST (60s timeout)  →  IMPROVE        │  │ │
│  │  │                                                                    │  │ │
│  │  │   Iteration 2:                                                     │  │ │
│  │  │   BUILD (refined)  →  TEST  →  IMPROVE                            │  │ │
│  │  │                                                                    │  │ │
│  │  │   Iteration N (max 3):                                             │  │ │
│  │  │   BUILD  →  TEST  →  Success? → Exit : (Escalate or Continue)    │  │ │
│  │  └──────────────────────────────────────────────────────────────────┘  │ │
│  │                                                                          │ │
│  │  Loop State Tracking:                                                    │ │
│  │  • iterations: Current iteration count                                  │ │
│  │  • last_build: Output, notes, tokens used                               │ │
│  │  • last_test: Status, output, duration                                  │ │
│  │  • last_improve: Action taken (retry, refine, escalate)                │ │
│  │                                                                          │ │
│  │  Integration:                                                            │ │
│  │  • Queries Hindsight for error resolutions                              │ │
│  │  • Captures trajectory via Note-Taking                                  │ │
│  │  • Uses Context Architect for full context                              │ │
│  │                                                                          │ │
│  │  Output: Final build result or escalation signal                        │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                              Storage Layer                                    │
│                                                                              │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
│  │ PostgreSQL │  │   Qdrant   │  │   Redis    │  │  DuckDB    │            │
│  │            │  │            │  │            │  │            │            │
│  │ • Metadata │  │ • Vectors  │  │ • Cache    │  │ • Graph    │            │
│  │ • Policies │  │ • Semantic │  │ • State    │  │ • Relations│            │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘            │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Data Flow Examples

### 1. Context Assembly Request

```
User Request
    │
    ▼
[OpenCode Plugin]
    │
    ├─ Extension Chain (on_input_messages)
    │  └─ Transform/enrich request
    │
    ▼
MCP Tool: context_assemble({ query, tokenBudget, layers })
    │
    ▼
[Aeterna Server]
    │
    ├─ Context Architect
    │  │
    │  ├─ Query Layer: Agent (parallel)
    │  ├─ Query Layer: User (parallel)
    │  ├─ Query Layer: Session (parallel)
    │  ├─ Query Layer: Project (parallel)
    │  └─ Query Layer: Team (parallel)
    │
    ├─ Relevance Scoring
    │  └─ Filter < 0.3 threshold
    │
    ├─ Deduplication
    │  └─ Remove semantic duplicates
    │
    ├─ Token Budget Fitting
    │  └─ Prioritize by layer + recency
    │
    ▼
AssembledContext { totalTokens: 3847, content: "...", layers: [Agent, User, Session] }
    │
    ▼
[OpenCode Plugin]
    │
    └─ Extension Chain (on_llm_output)
       └─ Format/display to user
```

### 2. Trajectory Capture Flow

```
Agent executes tool_call("memory_search", {...})
    │
    ▼
[Note-Taking Agent]
    │
    ├─ Check capture_mode: All/Sampled/ErrorsOnly
    │  └─ If Sampled: Random(1-100) <= sampling_rate?
    │
    ├─ Create TrajectoryEvent {
    │     tool_name: "memory_search",
    │     description: "Search for auth patterns",
    │     success: true,
    │     duration_ms: 42,
    │     metadata: { tags: ["auth"] }
    │  }
    │
    ├─ Async Queue (non-blocking)
    │  └─ Buffer size: 1000 events
    │
    ▼
Batch Processor (every 100ms or 10 events)
    │
    ├─ Write to storage
    │
    ├─ Check auto_distill_threshold
    │  └─ If event_count >= 10:
    │
    ▼
Distill to Markdown
    │
    ├─ Group by tags/themes
    ├─ Extract patterns
    ├─ Format as sections
    │
    ▼
Store as Memory (Project layer)
    │
    └─ Memory content: "## Auth Search Patterns\n- Frequently search for JWT\n- ..."
```

### 3. Hindsight Error Resolution

```
Build fails with error
    │
    ▼
[Hindsight Learning]
    │
    ├─ Auto-capture if enabled
    │
    ├─ Extract ErrorSignature {
    │     error_type: "BuildError",
    │     message_pattern: "cannot find symbol: class JwtValidator",
    │     context_patterns: ["src/auth/", "Java"]
    │  }
    │
    ├─ Generate embedding (vector)
    │
    ▼
Query existing hindsight notes
    │
    ├─ Semantic search (threshold: 0.8)
    │
    ├─ Find matches:
    │  └─ Match { score: 0.92, note_id: "hs_42", resolution: {
    │       description: "Add import com.example.auth.JwtValidator",
    │       success_rate: 0.95,
    │       application_count: 12
    │     }}
    │
    ▼
Return ranked resolutions to user/agent
    │
    └─ Agent applies fix
       │
       ▼
       Success? → Update resolution (success_rate++)
       │
       └─ If success_rate >= 0.8: Promote to Team/Org layer
```

### 4. Meta-Agent Build-Test-Improve Loop

```
User: "Generate and test a user authentication module"
    │
    ▼
[Meta-Agent]
    │
    ├─ Iteration 1
    │  │
    │  ├─ BUILD (timeout: 120s)
    │  │  ├─ Query Context Architect for patterns
    │  │  ├─ Generate code
    │  │  └─ Output: auth.rs (1247 tokens)
    │  │
    │  ├─ TEST (timeout: 60s)
    │  │  ├─ Run cargo test
    │  │  └─ Status: FAILED (3 tests failed)
    │  │
    │  └─ IMPROVE
    │     ├─ Query Hindsight for similar errors
    │     ├─ Capture trajectory via Note-Taking
    │     └─ Action: REFINE
    │
    ├─ Iteration 2
    │  │
    │  ├─ BUILD (refined)
    │  │  └─ Apply hindsight resolutions
    │  │
    │  ├─ TEST
    │  │  └─ Status: FAILED (1 test failed)
    │  │
    │  └─ IMPROVE
    │     └─ Action: RETRY with different approach
    │
    ├─ Iteration 3
    │  │
    │  ├─ BUILD
    │  ├─ TEST
    │  │  └─ Status: SUCCESS
    │  │
    │  └─ IMPROVE
    │     └─ Action: EXIT
    │
    ▼
Return: { iterations: 3, status: "success", output: "All tests passed" }
    │
    └─ Store successful trajectory as knowledge
```

## Component Interactions

### Context Architect + Memory Layers

The Context Architect queries memory layers based on configured priorities:

1. Default layer order: Session → Project → Team → Org → Tenant
2. Parallel queries if `enable_parallel_queries: true`
3. Early termination if `enable_early_termination: true` and budget satisfied
4. Cached results with TTL enforcement

### Note-Taking + Context Architect

Note-Taking stores distilled trajectories as memories, which Context Architect later retrieves:

1. Trajectory events accumulate (threshold: 10 events)
2. Distillation creates structured Markdown
3. Stored as Memory in Project/Team layer
4. Future context assemblies include these learnings

### Hindsight + Note-Taking

Hindsight captures errors, Note-Taking documents the resolution process:

1. Error occurs, Hindsight creates signature
2. Agent attempts fix, Note-Taking captures steps
3. Success triggers resolution creation
4. Resolution stored and ranked by success rate

### Meta-Agent + All Components

Meta-Agent orchestrates all other components in a feedback loop:

1. BUILD: Uses Context Architect for full context
2. TEST: Captures results via Note-Taking
3. IMPROVE: Queries Hindsight for error resolutions
4. Next iteration: Enhanced context from previous learnings

## Deployment Considerations

### Client-Side Extensions

- Deployed as part of OpenCode plugin (npm package: `@aeterna-org/opencode-plugin`)
- State stored in Redis, compressed with zstd
- LRU eviction when state exceeds 1MB per extension
- Callback timeouts prevent hanging (default: 5s)

### Server-Side Agents

- Deployed as Aeterna services (Rust binaries)
- Horizontal scaling via shared PostgreSQL/Qdrant/Redis
- Context Architect can be scaled independently (stateless queries)
- Meta-Agent requires sticky sessions for loop state

### Performance Tuning

1. Enable parallel queries for low-latency context assembly
2. Adjust sampling rates for Note-Taking in high-throughput scenarios
3. Configure cache TTL based on memory update frequency
4. Set appropriate token budgets based on LLM context windows (e.g., 4000 for GPT-3.5, 16000 for GPT-4)

### Monitoring

Key metrics to track:

- Context assembly latency (target: <100ms)
- Note capture overhead (target: <5ms per event)
- Hindsight query latency (target: <50ms)
- Meta-Agent iteration counts (anomaly: >3 frequently)
- Extension state size (alert: approaching 1MB)
- Token budget utilization (track over/under budget scenarios)

## Security Considerations

1. Extension state may contain sensitive data - ensure Redis encryption at rest
2. Hindsight notes may leak error details - enable sensitive pattern detection
3. Context assembly should respect multi-tenant isolation (enforced at TenantContext level)
4. Meta-Agent iterations should have cumulative timeout to prevent runaway loops
5. Extension callbacks run in client context - validate all inputs from server responses
