# OpenCode Integration Guide

Comprehensive guide for integrating Aeterna with [OpenCode](https://opencode.ai), the AI-powered coding assistant.

For a practical operator workflow, see the [OpenCode Daily Usage Playbook](../guides/opencode-daily-usage-playbook).

---

## Table of Contents

1. [Overview](#overview)
2. [Quick Start](#quick-start)
3. [Integration Methods](#integration-methods)
4. [NPM Plugin (Recommended)](#npm-plugin-recommended)
5. [All 14 Aeterna Tools](#all-14-aeterna-tools)
6. [Hook System](#hook-system)
7. [Automatic Session Capture](#automatic-session-capture)
8. [Knowledge Context Injection](#knowledge-context-injection)
9. [Significance Detection](#significance-detection)
10. [Governance Integration](#governance-integration)
11. [Configuration Reference](#configuration-reference)
12. [Troubleshooting](#troubleshooting)

---

## Overview

Aeterna integrates with OpenCode to provide:

- **Persistent Memory**: Remember context across sessions
- **Organizational Knowledge**: ADRs, policies, patterns injected into chat
- **Automatic Capture**: Tool executions saved as working memory
- **Significance Detection**: Important learnings auto-promoted
- **Governance**: Policy enforcement and constraint checking

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           OPENCODE + AETERNA                                 │
│                                                                              │
│   ┌─────────────────┐                    ┌─────────────────┐                │
│   │   User Message  │ ───────────────────▶│   AI Response   │                │
│   └────────┬────────┘                    └────────┬────────┘                │
│            │                                      │                          │
│            ▼                                      ▼                          │
│   ┌────────────────────────────────────────────────────────────────┐        │
│   │                     AETERNA PLUGIN HOOKS                        │        │
│   │                                                                 │        │
│   │   chat.message          system.transform      tool.execute      │        │
│   │   ┌──────────┐          ┌──────────┐          ┌──────────┐     │        │
│   │   │ Inject   │          │ Add      │          │ Capture  │     │        │
│   │   │ Knowledge│          │ Context  │          │ Memory   │     │        │
│   │   │ + Memory │          │ + Hints  │          │ + Detect │     │        │
│   │   └──────────┘          └──────────┘          └──────────┘     │        │
│   │                                                                 │        │
│   └────────────────────────────────────────────────────────────────┘        │
│                              │                                               │
│                              ▼                                               │
│   ┌─────────────────────────────────────────────────────────────────┐       │
│   │                      AETERNA BACKEND                             │       │
│   │                                                                  │       │
│   │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │       │
│   │   │    Memory    │  │  Knowledge   │  │  Governance  │          │       │
│   │   │   (Qdrant)   │  │    (Git)     │  │   (Cedar)    │          │       │
│   │   └──────────────┘  └──────────────┘  └──────────────┘          │       │
│   │                                                                  │       │
│   └─────────────────────────────────────────────────────────────────┘       │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Quick Start

### 1. Configure OpenCode

Add to your `opencode.jsonc`:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["@aeterna-org/opencode-plugin"]
}
```

OpenCode installs configured npm plugins automatically at startup. No separate `npm install -D` step is required for normal use.

### 2. Configure Aeterna auth and target

Static-token path:

```bash
export AETERNA_SERVER_URL=http://localhost:8080
export AETERNA_TOKEN=<your-token>
```

Interactive device-flow path:

```bash
export AETERNA_SERVER_URL=http://localhost:8080
export AETERNA_PLUGIN_AUTH_ENABLED=true
export AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID=<your-oauth-app-client-id>
```

Optional target context:

```bash
export AETERNA_TEAM=platform
export AETERNA_ORG=engineering
export AETERNA_USER_ID=you@example.com
```

### 3. Start Using

Launch OpenCode and all Aeterna tools are now available:

```
You: Search my memory for database preferences

AI: [Uses aeterna_memory_search]
Found 2 relevant memories:
- "User prefers PostgreSQL for relational data" (user layer, 0.92 score)
- "Project uses TimescaleDB for time-series" (project layer, 0.85 score)
```

When interactive auth is enabled, the plugin presents a GitHub device-flow verification URL and user code through the normal OpenCode-visible startup path. Complete that one-time sign-in in your browser, then continue once the plugin reports success.

---

## Integration Methods

Aeterna provides **two integration methods**:

| Method | Best For | Features |
|--------|----------|----------|
| **NPM Plugin** | Local development, full features | All hooks, deep integration |
| **MCP Server** | Remote deployments, enterprise | Tool access, limited hooks |

### Feature Comparison

| Feature | NPM Plugin | MCP Server |
|---------|------------|------------|
| All 14 tools | Yes | Yes |
| Chat message hook | Yes | No |
| System prompt injection | Yes | No |
| Tool execution capture | Yes | Limited |
| Permission hooks | Yes | No |
| Session lifecycle | Yes | Limited |
| Remote deployment | No | Yes |
| Enterprise auth | Via backend | Native |

**Recommendation**: Use **NPM Plugin** for development. Use **MCP Server** for centralized/remote deployments.

---

## NPM Plugin (Recommended)

### Architecture

The current plugin implementation provides:

- a default OpenCode plugin entrypoint
- a local-first memory router for personal layers
- a background sync engine when `AETERNA_SERVER_URL` is configured
- tool registration for the supported Aeterna tool surface
- hook-based context injection, capture, governance, and session lifecycle handling

### Package Structure

```
@aeterna-org/opencode-plugin/
├── package.json
├── src/
│   ├── index.ts          # Plugin entry point
│   ├── client.ts         # Aeterna client wrapper
│   ├── tools/
│   │   ├── memory.ts     # Memory tools (add, search, get, promote)
│   │   ├── knowledge.ts  # Knowledge tools (query, propose)
│   │   └── governance.ts # Governance tools (sync, status)
│   ├── hooks/
│   │   ├── chat.ts       # chat.message, system.transform
│   │   ├── tool.ts       # tool.execute.before/after
│   │   └── session.ts    # Session lifecycle (event hook)
│   └── utils/
│       ├── format.ts     # Output formatting
│       └── detect.ts     # Significance detection
└── README.md
```

---

## All 14 Aeterna Tools

### Memory Tools

#### `aeterna_memory_add`

Store information for future reference.

```typescript
const memoryAddTool = tool({
  description: "Add a memory entry to Aeterna. Use this to capture learnings, solutions, or important context.",
  args: {
    content: z.string().describe("The content to remember"),
    layer: z.enum(["working", "session", "episodic"]).optional()
      .describe("Memory layer (default: working)"),
    tags: z.array(z.string()).optional()
      .describe("Tags for categorization"),
    importance: z.number().min(0).max(1).optional()
      .describe("Importance score 0-1 (default: auto-calculated)"),
  },
  async execute(args, context) {
    const result = await client.memoryAdd({
      content: args.content,
      layer: args.layer ?? "working",
      tags: args.tags,
      importance: args.importance,
      sessionId: context.sessionID,
    })
    return `Memory added: ${result.id} (layer: ${result.layer}, importance: ${result.importance})`
  },
})
```

**Example Usage:**

```
You: Remember that this project uses PostgreSQL with read replicas

AI: [Uses aeterna_memory_add]
Memory added: mem_abc123 (layer: project, importance: 0.75)
I've stored this as a project-level memory for future reference.
```

---

#### `aeterna_memory_search`

Search memories for relevant context.

```typescript
const memorySearchTool = tool({
  description: "Search memories for relevant context. Returns semantically similar memories.",
  args: {
    query: z.string().describe("Search query"),
    layers: z.array(z.enum(["working", "session", "episodic"])).optional()
      .describe("Layers to search (default: all)"),
    limit: z.number().min(1).max(20).optional()
      .describe("Max results (default: 5)"),
    threshold: z.number().min(0).max(1).optional()
      .describe("Similarity threshold (default: 0.7)"),
  },
  async execute(args, context) {
    const results = await client.memorySearch({
      query: args.query,
      layers: args.layers,
      limit: args.limit ?? 5,
      threshold: args.threshold ?? 0.7,
      sessionId: context.sessionID,
    })
    return formatMemoryResults(results)
  },
})
```

**Example Usage:**

```
You: What do I prefer for error handling?

AI: [Uses aeterna_memory_search with query="error handling preferences"]
Found 3 relevant memories:
1. "User prefers Result types over exceptions" (user, 0.91)
2. "Project uses anyhow for error context" (project, 0.87)
3. "Team standard: always include error codes" (team, 0.82)
```

---

#### `aeterna_memory_get`

Retrieve a specific memory by ID.

```typescript
const memoryGetTool = tool({
  description: "Retrieve a specific memory by ID.",
  args: {
    memoryId: z.string().describe("Memory ID to retrieve"),
  },
  async execute(args) {
    const memory = await client.memoryGet(args.memoryId)
    return formatMemory(memory)
  },
})
```

---

#### `aeterna_memory_promote`

Promote a memory to a higher layer.

```typescript
const memoryPromoteTool = tool({
  description: "Promote a memory from a lower layer to a higher layer (e.g., session -> project).",
  args: {
    memoryId: z.string().describe("Memory ID to promote"),
    targetLayer: z.enum(["project", "team", "org", "tenant"])
      .describe("Target layer for promotion"),
    reason: z.string().optional()
      .describe("Reason for promotion"),
  },
  async execute(args) {
    const result = await client.memoryPromote({
      memoryId: args.memoryId,
      targetLayer: args.targetLayer,
      reason: args.reason,
    })
    return `Memory ${args.memoryId} promoted to ${args.targetLayer} layer`
  },
})
```

**Example Usage:**

```
You: That database optimization trick we discovered should be shared with the team

AI: [Uses aeterna_memory_promote]
Memory mem_xyz789 promoted to team layer.
Now all team members' AI assistants will have access to this optimization pattern.
```

---

### Knowledge Tools

#### `aeterna_knowledge_query`

Search organizational knowledge (ADRs, policies, patterns).

```typescript
const knowledgeQueryTool = tool({
  description: "Query the knowledge repository for project/team/org knowledge.",
  args: {
    query: z.string().describe("Search query"),
    scope: z.enum(["project", "team", "org", "tenant"]).optional()
      .describe("Knowledge scope (default: project)"),
    types: z.array(z.enum(["adr", "pattern", "policy", "reference"])).optional()
      .describe("Knowledge types to include"),
    limit: z.number().min(1).max(10).optional()
      .describe("Max results (default: 3)"),
  },
  async execute(args) {
    const results = await client.knowledgeQuery({
      query: args.query,
      scope: args.scope ?? "project",
      types: args.types,
      limit: args.limit ?? 3,
    })
    return formatKnowledgeResults(results)
  },
})
```

**Example Usage:**

```
You: Are there any ADRs about database selection?

AI: [Uses aeterna_knowledge_query with type="adr", query="database selection"]
Found 1 ADR:
- ADR-042: Database Selection for New Services (org level, accepted)
  Summary: Use PostgreSQL for all new services requiring relational data.
  Has blocking constraints against MySQL/MariaDB.
```

---

#### `aeterna_knowledge_propose`

Propose a new knowledge item (ADR, policy, pattern).

```typescript
const knowledgeProposeTool = tool({
  description: "Propose a new knowledge item (ADR, policy, pattern) for review.",
  args: {
    type: z.enum(["adr", "policy", "pattern", "reference"])
      .describe("Type of knowledge item"),
    title: z.string().describe("Title of the item"),
    summary: z.string().describe("Brief summary"),
    content: z.string().describe("Full markdown content"),
    tags: z.array(z.string()).optional().describe("Tags for categorization"),
  },
  async execute(args, context) {
    const result = await client.knowledgePropose({
      type: args.type,
      title: args.title,
      summary: args.summary,
      content: args.content,
      tags: args.tags,
      proposer: context.userID,
    })
    return `Knowledge proposal created: ${result.id} (status: ${result.status})`
  },
})
```

**Example Usage:**

```
You: Let's create an ADR for our decision to use TigerBeetle for the ledger

AI: [Uses aeterna_knowledge_propose]
Knowledge proposal created: adr-047-tigerbeetle-ledger (status: draft)
The proposal has been submitted for review. Notify your tech lead for approval.
```

---

### Sync & Governance Tools

#### `aeterna_sync_status`

Check memory-knowledge synchronization status.

```typescript
const syncStatusTool = tool({
  description: "Check the sync status between memory and knowledge systems.",
  args: {},
  async execute() {
    const status = await client.syncStatus()
    return formatSyncStatus(status)
  },
})
```

---

#### `aeterna_governance_status`

Check governance state and pending proposals.

```typescript
const governanceStatusTool = tool({
  description: "Check governance state including pending proposals and policy violations.",
  args: {
    checkViolations: z.boolean().optional()
      .describe("Check for current policy violations"),
  },
  async execute(args) {
    const status = await client.governanceStatus({
      checkViolations: args.checkViolations ?? false,
    })
    return formatGovernanceStatus(status)
  },
})
```

---

## Hook System

### Chat Message Hook

Injects relevant knowledge and memories into user messages.

```typescript
"chat.message": async (input, output) => {
  const userMessage = output.message.content
  
  // Query relevant knowledge
  const knowledge = await client.queryRelevantKnowledge(userMessage, {
    limit: 3,
    threshold: 0.75,
  })
  
  // Query relevant memories
  const memories = await client.searchSessionMemories(userMessage, {
    limit: 5,
  })
  
  if (knowledge.length > 0 || memories.length > 0) {
    output.parts.unshift({
      type: "text",
      text: `<aeterna_context>
${formatKnowledgeContext(knowledge)}
${formatMemoryContext(memories)}
</aeterna_context>`,
    })
  }
}
```

**What Gets Injected:**

| Source | Content | Priority |
|--------|---------|----------|
| Knowledge | ADRs, policies, patterns relevant to query | High |
| Session Memory | Current conversation context | High |
| User Memory | User preferences and history | Medium |
| Project Memory | Project-specific conventions | Medium |
| Team Memory | Team standards and learnings | Low |

**Example Injection:**

```xml
<aeterna_context>
## Relevant Knowledge
- **ADR-042**: Database Selection - Use PostgreSQL for new services
- **Policy**: security-baseline - All dependencies must be scanned

## Relevant Memories
- User prefers functional programming patterns (user, 0.91)
- Last session discussed database optimization (session, 0.88)
</aeterna_context>
```

---

### System Prompt Transform Hook

Adds project context to the AI's system prompt.

```typescript
"experimental.chat.system.transform": async (input, output) => {
  const context = await client.getProjectContext()
  
  output.system.push(`
## Aeterna Project Context

Project: ${context.project.name}
Team: ${context.team?.name ?? "N/A"}
Organization: ${context.org?.name ?? "N/A"}

### Active Policies
${context.policies.map(p => `- ${p.name}: ${p.summary}`).join("\n")}

### Recent Learnings
${context.recentMemories.map(m => `- ${m.summary}`).join("\n")}

When you discover useful patterns or solutions, use aeterna_memory_add to capture them.
When you need context, use aeterna_memory_search or aeterna_knowledge_query.
`)
}
```

---

### Tool Execute Before Hook

Validates and enriches tool arguments before execution.

```typescript
"tool.execute.before": async (input, output) => {
  if (input.tool.startsWith("aeterna_")) {
    // Enrich with session context
    await client.enrichToolArgs(input.tool, output.args)
  }
  
  // Pre-check governance for sensitive operations
  if (input.tool === "aeterna_knowledge_propose") {
    const canPropose = await client.checkProposalPermission()
    if (!canPropose) {
      throw new Error("User lacks permission to propose knowledge items")
    }
  }
}
```

---

### Tool Execute After Hook

Captures tool executions as working memory.

```typescript
"tool.execute.after": async (input, output) => {
  // Capture all tool executions
  await client.captureToolExecution({
    tool: input.tool,
    sessionId: input.sessionID,
    callId: input.callID,
    title: output.title,
    output: output.output,
    metadata: output.metadata,
    timestamp: Date.now(),
  })
  
  // Detect significant patterns
  if (await client.detectSignificance(input, output)) {
    await client.flagForPromotion(input.sessionID, input.callID)
  }
}
```

---

### Permission Hook

Integrates with Aeterna's governance system.

```typescript
"permission.ask": async (input, output) => {
  if (input.tool?.startsWith("aeterna_knowledge_propose")) {
    const canPropose = await client.checkProposalPermission()
    if (!canPropose) {
      output.status = "deny"
      output.message = "You don't have permission to propose knowledge items. Contact your tech lead."
    }
  }
}
```

---

### Event Hook

Handles session lifecycle events.

```typescript
"event": async ({ event }) => {
  switch (event.type) {
    case "session.start":
      await client.sessionStart()
      await client.subscribeToGovernance()
      break
      
    case "session.end":
      await client.sessionEnd()
      // Auto-promote significant memories
      await client.promotePendingMemories()
      // Generate session summary
      await client.generateSessionSummary()
      break
  }
}
```

---

## Automatic Session Capture

Every tool execution is automatically captured without user action.

### What Gets Captured

| Event | Captured Data | Memory Layer |
|-------|---------------|--------------|
| Tool invocation | Tool name, args, output | Working |
| Error occurrence | Error message, stack trace | Working |
| File modification | File path, change type | Working |
| Chat message | Message content (if significant) | Working |

### Capture Flow

```
Tool Execution
      │
      ▼
┌─────────────────┐
│ Capture Context │  ← tool name, args, output, timestamp
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Create Memory  │  ← working memory entry
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Detect Signif.  │  ← error resolution? repeated? novel?
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
 Normal    Significant
    │         │
    ▼         ▼
 Store     Flag for
 Only      Promotion
```

### Session Summary Generation

At session end, Aeterna generates a summary:

```typescript
interface SessionSummary {
  sessionId: string
  duration: number
  toolsUsed: string[]
  filesModified: string[]
  memoriesCreated: number
  memoriesPromoted: number
  significantLearnings: string[]
}
```

---

## Knowledge Context Injection

### Semantic Matching

Knowledge is matched based on semantic similarity to the user's message:

```typescript
const knowledge = await client.queryRelevantKnowledge(userMessage, {
  limit: 3,           // Max 3 items
  threshold: 0.75,    // 75% similarity minimum
  types: ["adr", "policy", "pattern"],
})
```

### Priority Order

When multiple knowledge items match, priority follows:

1. **Project** (highest) - Most specific to current work
2. **Team** - Team-level standards
3. **Organization** - Org-wide policies
4. **Tenant** (lowest) - Tenant-wide rules

### Token Limits

To avoid context overflow:

```typescript
const context = await client.getContextWithLimits({
  maxTokens: 2000,
  knowledgeTokens: 1000,  // Max tokens for knowledge
  memoryTokens: 1000,     // Max tokens for memory
})
```

Items are truncated or omitted based on priority when limits are exceeded.

---

## Significance Detection

Aeterna automatically detects significant learnings worthy of promotion.

### Detection Criteria

| Pattern | Description | Auto-Action |
|---------|-------------|-------------|
| **Error Resolution** | Error followed by successful outcome | Flag for promotion |
| **Repeated Query** | Same query 3+ times | Consolidate & flag |
| **Novel Approach** | Solution differs from existing knowledge | Flag for review |
| **Explicit Capture** | User called `aeterna_memory_add` | Store immediately |

### Error Resolution Detection

```typescript
async function detectErrorResolution(session: Session): Promise<boolean> {
  const recentTools = session.toolExecutions.slice(-10)
  
  // Find error followed by success
  for (let i = 0; i < recentTools.length - 1; i++) {
    if (recentTools[i].error && !recentTools[i + 1].error) {
      // Check if same area (e.g., same file, same tool)
      if (isSameArea(recentTools[i], recentTools[i + 1])) {
        return true
      }
    }
  }
  return false
}
```

### Repeated Pattern Detection

```typescript
async function detectRepeatedPattern(session: Session): Promise<boolean> {
  const queries = session.memorySearches
  const grouped = groupBySimilarity(queries, 0.85)
  
  for (const group of grouped) {
    if (group.length >= 3) {
      // Same query 3+ times → consolidate
      return true
    }
  }
  return false
}
```

---

## Governance Integration

### Governance Notifications

The plugin subscribes to governance events:

```typescript
type GovernanceEvent = 
  | { type: "ProposalApproved"; proposalId: string; approver: string }
  | { type: "ProposalRejected"; proposalId: string; reason: string }
  | { type: "DriftDetected"; itemId: string; severity: string }
  | { type: "PolicyViolation"; policyId: string; violation: string }
```

### Event Handling

```typescript
client.onGovernanceEvent((event) => {
  switch (event.type) {
    case "ProposalApproved":
      notify(`Your proposal ${event.proposalId} was approved!`)
      break
    case "DriftDetected":
      warn(`Semantic drift detected in ${event.itemId}. Review recommended.`)
      break
    case "PolicyViolation":
      error(`Policy violation: ${event.violation}`)
      break
  }
})
```

### Permission Checks

Before sensitive operations:

```typescript
const permissions = await client.checkPermissions({
  action: "ProposeKnowledge",
  resource: "adr",
  scope: "team",
})

if (!permissions.allowed) {
  throw new Error(`Permission denied: ${permissions.reason}`)
}
```

---

## Configuration Reference

### `.aeterna/config.toml`

```toml
[project]
name = "my-project"
team = "backend"
org = "engineering"

[capture]
# Enable automatic session capture
enabled = true
# Capture sensitivity: low, medium, high
# - low: Only explicit aeterna_memory_add calls
# - medium: + error resolutions and repeated patterns
# - high: + all tool executions
sensitivity = "medium"
# Auto-promote significant memories at session end
auto_promote = true

[knowledge]
# Inject relevant knowledge into chat context
injection_enabled = true
# Maximum knowledge items to inject
max_items = 3
# Minimum similarity threshold (0.0 - 1.0)
threshold = 0.75

[governance]
# Enable governance event notifications
notifications = true
# Alert on semantic drift detection
drift_alerts = true
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AETERNA_SERVER_URL` | Aeterna server URL | `http://localhost:8080` |
| `AETERNA_TOKEN` | Static bearer token for API authentication | - |
| `AETERNA_PLUGIN_AUTH_ENABLED` | Enable GitHub OAuth device-flow login | `false` |
| `AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID` | GitHub OAuth App client ID for device flow | - |
| `AETERNA_TEAM` | Team context override | - |
| `AETERNA_ORG` | Organization context override | - |
| `AETERNA_USER_ID` | User identity override | - |
| `AETERNA_LOCAL_DB_PATH` | Local SQLite path for personal layers | `~/.aeterna/local.db` |

### `opencode.jsonc`

```jsonc
{
  "$schema": "https://opencode.ai/config.json",

  "plugin": ["@aeterna-org/opencode-plugin"]
}
```

The supported plugin configuration model is:
- OpenCode plugin registration in `opencode.jsonc`
- Aeterna runtime/auth settings via environment variables
- optional `.aeterna/config.toml` for project-specific plugin behavior

---

## Troubleshooting

### Common Issues

#### Plugin Not Loading

```
Error: Cannot find module '@aeterna-org/opencode-plugin'
```

**Solution:**
```bash
# Verify the plugin entry exists in opencode.jsonc
# OpenCode installs configured plugins automatically at startup
```

#### Connection Refused

```
Error: ECONNREFUSED 127.0.0.1:8080
```

**Solution:**
1. Ensure Aeterna server is running: `aeterna serve`
2. Check `AETERNA_SERVER_URL` environment variable
3. Verify firewall settings

#### Knowledge Not Injecting

**Symptoms:** AI doesn't mention ADRs or policies

**Diagnosis:**
```typescript
// Check configuration
const config = await client.getConfig()
console.log(config.knowledge.injectionEnabled) // Should be true

// Check threshold
const results = await client.knowledgeQuery({ query: "test" })
console.log(results) // Should have items above threshold
```

**Solutions:**
1. Lower `knowledge.threshold` in config
2. Verify knowledge items exist: `aeterna knowledge list`
3. Check semantic similarity with your queries

#### Memories Not Capturing

**Symptoms:** Tool executions not appearing in memory

**Diagnosis:**
```bash
# Check capture settings
cat .aeterna/config.toml | grep capture
```

**Solutions:**
1. Set `capture.enabled = true`
2. Increase `capture.sensitivity`
3. Verify working memory storage is configured

#### Permission Denied for Proposals

```
Error: You don't have permission to propose knowledge items
```

**Solutions:**
1. Check your role: `aeterna whoami`
2. Contact tech lead for elevated permissions
3. Verify Cedar policy allows your role

### Debug Mode

Enable verbose logging:

```bash
AETERNA_DEBUG=true opencode
```

Or in config:

```toml
[debug]
enabled = true
log_level = "trace"
```

### Health Check

```bash
# Check Aeterna connectivity
npx aeterna health

# Expected output:
# ✓ Server: http://localhost:8080 (healthy)
# ✓ Memory: Qdrant connection OK
# ✓ Knowledge: Git repository accessible
# ✓ Governance: Cedar policies loaded
```

---

## Next Steps

- [MCP Server Setup](./mcp-server) - For remote deployments
- [Strangler Fig Example](../examples/strangler-fig-migration) - Real-world usage
- [Tool Interface Specification](../specs/tool-interface) - Detailed tool contracts
