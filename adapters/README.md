# Adapters

Provider and ecosystem adapters for memory-knowledge system.

## Ecosystem Adapters

### OpenCode Adapter
```rust
use adapters::ecosystem::OpenCodeAdapter;
use tools::server::McpServer;
use std::sync::Arc;

let server = Arc::new(/* McpServer */);
let adapter = OpenCodeAdapter::new(server);

// Get memory-specific tools
let memory_tools = adapter.get_memory_tools();

// Get knowledge-specific tools
let knowledge_tools = adapter.get_knowledge_tools();

// Handle MCP requests
let response = adapter.handle_mcp_request(request).await?;
```

### LangChain Adapter
```rust
use adapters::langchain::LangChainAdapter;
use tools::server::McpServer;
use std::sync::Arc;

let server = Arc::new(/* McpServer */);
let adapter = LangChainAdapter::new(server);

// Convert to LangChain tool format
let langchain_tools = adapter.to_langchain_tools();

// Handle MCP requests
let response = adapter.handle_mcp_request(request).await?;
```

## Context Hooks

```rust
use adapters::hooks::MemoryContextHooks;
use mk_core::traits::ContextHooks;

let hooks = MemoryContextHooks::new();

// Session lifecycle
hooks.on_session_start("session-123").await?;
hooks.on_message("session-123", "User asked about authentication").await?;
hooks.on_tool_use("session-123", "memory_search", json!({"query": "auth"})).await?;
hooks.on_session_end("session-123").await?;
```

## JSON Schema Validation

All tools generate JSON Schema for input validation:
```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
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
    }
  },
  "required": ["content"],
  "additionalProperties": false
}
```

## Integration Tests

Run adapter tests:
```bash
cargo test -p adapters
```
