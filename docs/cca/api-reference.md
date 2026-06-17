# CCA API Reference

This document provides complete API specifications for the four MCP tools that expose CCA (Confucius Code Agent) capabilities to AI agents and applications.

## Overview

CCA exposes four tools via Model Context Protocol (MCP):

1. **context_assemble** - Assemble hierarchical context from memory layers
2. **note_capture** - Capture trajectory events for distillation
3. **hindsight_query** - Query error patterns and resolutions
4. **meta_loop_status** - Get status of meta-agent loops

All tools follow MCP JSON-RPC 2.0 specification and return standardized responses.

## Tool 1: context_assemble

Assemble hierarchical context from memory layers using the Context Architect component. This tool queries multiple memory layers, scores relevance, deduplicates entries, and fits results within a token budget.

### Input Schema

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Optional semantic query to filter context. If omitted, retrieves all relevant context."
    },
    "tokenBudget": {
      "type": "integer",
      "minimum": 100,
      "maximum": 32000,
      "default": 4000,
      "description": "Maximum tokens for assembled context. Adjust based on your LLM's context window."
    },
    "layers": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": ["agent", "user", "session", "project", "team", "org", "tenant"]
      },
      "description": "Memory layers to query. If omitted, uses configured layer_priorities."
    }
  },
  "required": []
}
```

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | No | null | Semantic query string to filter context. Example: "authentication patterns" |
| `tokenBudget` | integer | No | 4000 | Token limit for assembled context (100-32000) |
| `layers` | string[] | No | config | Array of layer names to query. Values: "agent", "user", "session", "project", "team", "org", "tenant" |

### Response

```json
{
  "success": true,
  "context": {
    "totalTokens": 3847,
    "tokenBudget": 4000,
    "layersIncluded": ["Session", "Project", "Team"],
    "isWithinBudget": true,
    "entryCount": 23,
    "content": "## Session Context\n- Current task: Implement JWT authentication\n...\n\n## Project Context\n- Tech stack: Rust, PostgreSQL, Redis\n..."
  }
}
```

### Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `success` | boolean | Always true on successful assembly |
| `context.totalTokens` | integer | Actual tokens used in assembled context |
| `context.tokenBudget` | integer | Token budget that was requested |
| `context.layersIncluded` | string[] | Which memory layers contributed to context (in priority order) |
| `context.isWithinBudget` | boolean | Whether total tokens fit within budget |
| `context.entryCount` | integer | Number of memory entries included |
| `context.content` | string | Assembled context as formatted text (Markdown) |

### Example Usage

#### Basic Context Assembly

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "context_assemble",
    "arguments": {}
  }
}
```

Returns context from all configured layers with default 4000 token budget.

#### Query-Specific Context

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "context_assemble",
    "arguments": {
      "query": "database connection pooling",
      "tokenBudget": 2000,
      "layers": ["project", "team", "org"]
    }
  }
}
```

Returns context related to database connection pooling from project, team, and org layers, limited to 2000 tokens.

#### Large Context for Complex Tasks

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "context_assemble",
    "arguments": {
      "tokenBudget": 16000,
      "layers": ["agent", "user", "session", "project", "team"]
    }
  }
}
```

Returns comprehensive context from 5 layers with 16000 token budget (suitable for GPT-4-32k).

### Error Handling

| Error Code | Description | Resolution |
|------------|-------------|------------|
| `-32602` | Invalid parameters (e.g., tokenBudget out of range) | Check parameter constraints |
| `-32000` | Context assembly timeout | Reduce layers or token budget |
| `-32001` | Memory layer unavailable | Check Aeterna service status |

## Tool 2: note_capture

Capture a trajectory event for note-taking and eventual distillation to knowledge. Events accumulate until the auto-distill threshold is reached, at which point they are distilled into Markdown notes.

### Input Schema

```json
{
  "type": "object",
  "properties": {
    "description": {
      "type": "string",
      "description": "Human-readable description of the event"
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional tags for categorizing the event"
    },
    "toolName": {
      "type": "string",
      "default": "manual_capture",
      "description": "Name of the tool that triggered this event"
    },
    "success": {
      "type": "boolean",
      "default": false,
      "description": "Whether the event represents a successful action"
    }
  },
  "required": ["description"]
}
```

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | Yes | - | Description of the trajectory event. Example: "Successfully connected to PostgreSQL with connection pooling" |
| `tags` | string[] | No | [] | Tags for categorization. Example: ["database", "success"] |
| `toolName` | string | No | "manual_capture" | Tool that generated this event. Example: "memory_search" |
| `success` | boolean | No | false | Whether this event was successful |

### Response

```json
{
  "success": true,
  "message": "Trajectory event captured: Successfully connected to PostgreSQL",
  "eventCount": 7
}
```

### Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `success` | boolean | Always true on successful capture |
| `message` | string | Confirmation message with event description |
| `eventCount` | integer | Total events captured so far (before distillation) |

### Example Usage

#### Capture Successful Tool Usage

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "note_capture",
    "arguments": {
      "description": "Used memory_search to find JWT authentication patterns, found 3 relevant examples",
      "tags": ["memory", "auth", "success"],
      "toolName": "memory_search",
      "success": true
    }
  }
}
```

#### Capture Error Event

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/call",
  "params": {
    "name": "note_capture",
    "arguments": {
      "description": "Failed to connect to Redis: connection refused on localhost:6379",
      "tags": ["redis", "error", "connection"],
      "toolName": "memory_add",
      "success": false
    }
  }
}
```

#### Manual Milestone Capture

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "tools/call",
  "params": {
    "name": "note_capture",
    "arguments": {
      "description": "Completed authentication module implementation with 95% test coverage",
      "tags": ["milestone", "auth", "testing"]
    }
  }
}
```

### Behavior Notes

- Events are captured asynchronously (non-blocking)
- When `eventCount` reaches `auto_distill_threshold` (default: 10), automatic distillation triggers
- Distilled notes are stored as Memory entries in Project/Team layer
- If `capture_mode = "sampled"`, only 1 in N events are actually captured

### Error Handling

| Error Code | Description | Resolution |
|------------|-------------|------------|
| `-32602` | Missing required `description` | Provide description |
| `-32000` | Queue full (exceeds `queue_size`) | Increase queue_size or reduce capture rate |
| `-32001` | Overhead budget exceeded | Event dropped silently (check logs) |

## Tool 3: hindsight_query

Query the Hindsight Learning system for error patterns and resolutions. This tool performs semantic matching against previously captured errors and returns ranked resolution suggestions.

### Input Schema

```json
{
  "type": "object",
  "properties": {
    "errorType": {
      "type": "string",
      "description": "Type of error (e.g., 'TypeError', 'BuildError', 'NetworkError')"
    },
    "messagePattern": {
      "type": "string",
      "description": "Error message pattern or substring"
    },
    "contextPatterns": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional context patterns (file paths, stack traces, etc.)"
    }
  },
  "required": ["errorType", "messagePattern"]
}
```

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `errorType` | string | Yes | - | Error type classification. Example: "BuildError", "TypeError", "DatabaseError" |
| `messagePattern` | string | Yes | - | Error message pattern. Example: "cannot find symbol: class JwtValidator" |
| `contextPatterns` | string[] | No | [] | Additional context for matching. Example: ["src/auth/", "Java"] |

### Response

```json
{
  "success": true,
  "matchCount": 2,
  "matches": [
    {
      "noteId": "hs_42",
      "score": 0.92,
      "content": "Build failed due to missing import for JwtValidator class",
      "resolution": {
        "description": "Add import statement: import com.example.auth.JwtValidator;",
        "successRate": 0.95,
        "applicationCount": 12
      }
    },
    {
      "noteId": "hs_38",
      "score": 0.87,
      "content": "Compilation error: JwtValidator not found in classpath",
      "resolution": {
        "description": "Add dependency to build.gradle: implementation 'com.example:auth:1.2.0'",
        "successRate": 0.89,
        "applicationCount": 8
      }
    }
  ]
}
```

### Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `success` | boolean | Always true on successful query |
| `matchCount` | integer | Number of matching errors found |
| `matches` | array | Array of matched errors with resolutions (sorted by score descending) |
| `matches[].noteId` | string | Unique identifier for this hindsight note |
| `matches[].score` | float | Similarity score (0.0-1.0), higher = more similar |
| `matches[].content` | string | Original error note content |
| `matches[].resolution` | object | Suggested resolution (may be null if no resolution exists) |
| `matches[].resolution.description` | string | How to resolve this error |
| `matches[].resolution.successRate` | float | Historical success rate (0.0-1.0) |
| `matches[].resolution.applicationCount` | integer | How many times this resolution has been applied |

### Example Usage

#### Query Build Error

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "tools/call",
  "params": {
    "name": "hindsight_query",
    "arguments": {
      "errorType": "BuildError",
      "messagePattern": "cannot find symbol: class JwtValidator",
      "contextPatterns": ["src/auth/TokenService.java"]
    }
  }
}
```

#### Query Runtime Error

```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "tools/call",
  "params": {
    "name": "hindsight_query",
    "arguments": {
      "errorType": "RuntimeError",
      "messagePattern": "NullPointerException at line 42"
    }
  }
}
```

#### Query Network Error

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "tools/call",
  "params": {
    "name": "hindsight_query",
    "arguments": {
      "errorType": "NetworkError",
      "messagePattern": "connection timeout",
      "contextPatterns": ["Redis", "localhost:6379"]
    }
  }
}
```

### Using Resolution Suggestions

Agents should:

1. Review all matches, prioritizing high `score` and high `successRate`
2. Apply the top resolution suggestion
3. If successful, capture the outcome (success increases successRate)
4. If failed, try the next suggestion or create a new resolution

### Error Handling

| Error Code | Description | Resolution |
|------------|-------------|------------|
| `-32602` | Missing required fields | Provide errorType and messagePattern |
| `-32000` | Query timeout | Simplify contextPatterns |
| `-32001` | No matches found | Not an error, just empty matches array |

## Tool 4: meta_loop_status

Get the current status of Meta-Agent build-test-improve loops. Useful for monitoring long-running autonomous agent tasks.

### Input Schema

```json
{
  "type": "object",
  "properties": {
    "loopId": {
      "type": "string",
      "description": "Optional specific loop ID to query. If omitted, returns summary of all loops."
    },
    "includeDetails": {
      "type": "boolean",
      "default": false,
      "description": "Include detailed state (build/test/improve outputs)"
    }
  },
  "required": []
}
```

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `loopId` | string | No | null | Specific loop ID to query. If omitted, returns summary. |
| `includeDetails` | boolean | No | false | Whether to include detailed state (can be large) |

### Response (Summary)

```json
{
  "success": true,
  "status": "running",
  "activeLoops": 2,
  "loopState": null,
  "details": null
}
```

### Response (Specific Loop with Details)

```json
{
  "success": true,
  "status": "running",
  "activeLoops": 2,
  "loopState": {
    "iterations": 2,
    "hasLastBuild": true,
    "hasLastTest": true
  },
  "details": {
    "iterations": 2,
    "lastBuild": {
      "output": "Build successful: auth.rs compiled in 1.2s",
      "notes": "Used JWT crate version 0.16",
      "tokensUsed": 1847
    },
    "lastTest": {
      "status": "Failed",
      "output": "3 tests failed: test_token_expiry, test_refresh_token, test_invalid_signature",
      "durationMs": 423
    },
    "lastImprove": {
      "action": "Refine"
    }
  }
}
```

### Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `success` | boolean | Always true on successful query |
| `status` | string | Overall status: "running", "idle" |
| `activeLoops` | integer | Number of currently running loops |
| `loopState` | object | State of specific loop (null if loopId not provided or not found) |
| `loopState.iterations` | integer | Current iteration number |
| `loopState.hasLastBuild` | boolean | Whether a build has completed |
| `loopState.hasLastTest` | boolean | Whether a test has completed |
| `details` | object | Detailed state (only if `includeDetails: true`) |
| `details.lastBuild.output` | string | Build output/logs |
| `details.lastBuild.notes` | string | Build notes/observations |
| `details.lastBuild.tokensUsed` | integer | Tokens consumed during build |
| `details.lastTest.status` | string | Test status: "Passed", "Failed", "TimedOut" |
| `details.lastTest.output` | string | Test output/logs |
| `details.lastTest.durationMs` | integer | Test duration in milliseconds |
| `details.lastImprove.action` | string | Action taken: "Retry", "Refine", "Escalate" |

### Example Usage

#### Check Overall Status

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "tools/call",
  "params": {
    "name": "meta_loop_status",
    "arguments": {}
  }
}
```

Returns summary of all active loops.

#### Check Specific Loop

```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "tools/call",
  "params": {
    "name": "meta_loop_status",
    "arguments": {
      "loopId": "meta_loop_abc123"
    }
  }
}
```

Returns state of specific loop without details.

#### Get Full Details

```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "tools/call",
  "params": {
    "name": "meta_loop_status",
    "arguments": {
      "loopId": "meta_loop_abc123",
      "includeDetails": true
    }
  }
}
```

Returns full state including build/test/improve outputs.

### Polling Recommendations

For monitoring long-running loops:

- Poll every 5-10 seconds for active loops
- Use `includeDetails: false` for frequent polls (reduces bandwidth)
- Only fetch details when iteration count changes
- Stop polling when `status: "idle"` and `activeLoops: 0`

### Error Handling

| Error Code | Description | Resolution |
|------------|-------------|------------|
| `-32602` | Invalid loopId format | Check loopId string |
| `-32000` | Loop not found | Loop may have completed or been cleaned up |

## Authentication and Authorization

All tools respect Aeterna's multi-tenant authorization:

- Tenant context is derived from MCP session
- Users can only query memory from their accessible layers (based on role)
- Agents inherit delegated permissions from the user who spawned them

Example: A Developer role can query Session/Project/Team layers, but not Tenant/Org unless explicitly granted.

## Rate Limiting

CCA tools are subject to rate limiting:

- Default: 100 requests/minute per tenant
- Burst: 20 requests in 10 seconds
- Exceeded: Returns error `-32429` (Too Many Requests)

Configure in `config/aeterna.toml`:

```toml
[api.rate_limit]
cca_tools_per_minute = 100
cca_tools_burst = 20
```

## Integration Examples

### LangChain Python

```python
from langchain.tools import Tool
from aeterna_mcp_client import AeternaMCPClient

client = AeternaMCPClient("http://localhost:8080")

context_tool = Tool(
    name="context_assemble",
    func=lambda q: client.call_tool("context_assemble", {"query": q, "tokenBudget": 4000}),
    description="Assemble hierarchical context from Aeterna memory"
)

hindsight_tool = Tool(
    name="hindsight_query",
    func=lambda err: client.call_tool("hindsight_query", {"errorType": err["type"], "messagePattern": err["msg"]}),
    description="Query error resolutions from Hindsight Learning"
)

# Use in agent
agent = initialize_agent([context_tool, hindsight_tool], llm, agent="zero-shot-react-description")
```

### OpenCode Plugin

```typescript
import { McpClient } from '@aeterna-org/opencode-plugin';

const client = new McpClient({ url: 'http://localhost:8080' });

// Assemble context before LLM call
const context = await client.callTool('context_assemble', {
  query: 'authentication patterns',
  tokenBudget: 4000
});

// Capture trajectory after successful action
await client.callTool('note_capture', {
  description: 'Implemented JWT auth with refresh tokens',
  tags: ['auth', 'success', 'jwt'],
  success: true
});
```

## Next Steps

- [Configuration](configuration.md) - Configure CCA components
- [Extension Guide](extension-guide.md) - Build custom extensions
- [Architecture](architecture.md) - Understand the hybrid execution model
