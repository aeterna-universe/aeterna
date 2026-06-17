# opencode-integration Specification

## Purpose
Define the supported Aeterna integration contract for OpenCode, including the plugin package, tool surface, hook behavior, session lifecycle, and user-visible integration model.
## Requirements
### Requirement: NPM Plugin Package

The system SHALL provide an NPM package `@aeterna-org/opencode-plugin` that integrates with OpenCode using the official `@opencode-ai/plugin` SDK.

The plugin MUST:
- Export a default Plugin function conforming to OpenCode's Plugin type
- Register all supported Aeterna tools as OpenCode tools
- Implement lifecycle hooks for deep integration
- Support configuration via `opencode.jsonc`, environment variables, and `.aeterna/config.toml`
- Initialize a `LocalMemoryManager` on startup for local-first personal layer access
- Run a background `SyncEngine` for bidirectional memory synchronization when a server URL is configured

#### Scenario: Plugin installation
- **WHEN** a user adds `"plugin": ["@aeterna-org/opencode-plugin"]` to `opencode.jsonc`
- **THEN** OpenCode SHALL install and load the plugin automatically at startup
- **AND** the supported installation flow SHALL NOT require a separate manual `npm install -D` step for normal usage
- **AND** the plugin SHALL use the Bun-compatible SQLite runtime supported by OpenCode

#### Scenario: Plugin initialization with local store
- **WHEN** OpenCode starts with the Aeterna plugin configured
- **THEN** the plugin SHALL initialize the `LocalMemoryManager` with the configured database path
- **AND** the plugin SHALL start the `SyncEngine` background loop if a server URL is configured
- **AND** the plugin SHALL register all tools and hooks
- **AND** the plugin SHALL establish exactly one active session context for the OpenCode session

#### Scenario: Plugin initialization without server
- **WHEN** OpenCode starts with the Aeterna plugin configured
- **AND** no `AETERNA_SERVER_URL` is set
- **THEN** the plugin SHALL initialize the `LocalMemoryManager` for offline-only operation
- **AND** personal layer tools SHALL function normally
- **AND** shared layer tools SHALL return empty or cached results with an informative message rather than failing the session

#### Scenario: Plugin shutdown with pending sync
- **WHEN** OpenCode shuts down
- **AND** the `sync_queue` contains pending operations
- **THEN** the plugin SHALL attempt a final sync push (up to 5s timeout)
- **AND** the plugin SHALL close the SQLite database cleanly

#### Scenario: Plugin configuration via opencode.jsonc
- **WHEN** the user adds `"plugin": ["@aeterna-org/opencode-plugin"]` to `opencode.jsonc`
- **THEN** OpenCode SHALL load and initialize the Aeterna plugin
- **AND** all supported Aeterna tools SHALL be available to the AI

### Requirement: Tool Registration

The system SHALL register the supported Aeterna tools as OpenCode tools using the `tool()` helper from `@opencode-ai/plugin/tool`.

| Tool Name | Description |
|-----------|-------------|
| `aeterna_memory_add` | Add memory entry with content, layer, tags, importance |
| `aeterna_memory_search` | Search memories with semantic similarity |
| `aeterna_memory_get` | Retrieve specific memory by ID |
| `aeterna_memory_promote` | Promote memory to higher layer |
| `aeterna_knowledge_query` | Query knowledge repository |
| `aeterna_knowledge_propose` | Propose new knowledge item |
| `aeterna_graph_query` | Query graph relationships and traversals |
| `aeterna_graph_neighbors` | Find directly related memories |
| `aeterna_graph_path` | Find shortest path between memories |
| `aeterna_context_assemble` | Assemble hierarchical context |
| `aeterna_note_capture` | Capture trajectory events for distillation |
| `aeterna_hindsight_query` | Query learned error patterns |
| `aeterna_meta_loop_status` | Inspect meta-loop state |
| `aeterna_sync_status` | Check sync status |
| `aeterna_governance_status` | Check governance state |

#### Scenario: Tool schema definition
- **WHEN** OpenCode requests tool definitions
- **THEN** the plugin SHALL return Zod-based schemas for each tool
- **AND** each schema SHALL include descriptions for all parameters

#### Scenario: Memory add tool invocation
- **WHEN** the AI invokes `aeterna_memory_add` with content
- **THEN** the plugin SHALL create a memory entry in Aeterna
- **AND** the plugin SHALL return confirmation with memory ID and computed importance

#### Scenario: Knowledge query tool invocation
- **WHEN** the AI invokes `aeterna_knowledge_query` with a query
- **THEN** the plugin SHALL search the knowledge repository
- **AND** the plugin SHALL return formatted results with relevance scores

### Requirement: Chat Message Hook

The system SHALL implement the `chat.message` hook to inject relevant knowledge and memories into user messages before they are sent to the LLM.

#### Scenario: Knowledge injection on chat
- **WHEN** a user sends a message to the AI
- **THEN** the plugin SHALL query relevant knowledge based on message content
- **AND** the plugin SHALL prepend knowledge context to the message parts
- **AND** the LLM SHALL receive the enriched context

#### Scenario: Memory recall on chat
- **WHEN** a user sends a message referencing previous work
- **THEN** the plugin SHALL search session and episodic memories
- **AND** the plugin SHALL include relevant memories in the context

#### Scenario: Empty context handling
- **WHEN** no relevant knowledge or memories are found
- **THEN** the plugin SHALL NOT modify the message parts
- **AND** the original message SHALL pass through unchanged

### Requirement: System Prompt Transform Hook

The system SHALL implement the `experimental.chat.system.transform` hook to add project context to the AI's system prompt.

#### Scenario: Project context injection
- **WHEN** a chat session is active
- **THEN** the plugin SHALL add project, team, and org context to the system prompt
- **AND** the plugin SHALL include active policies and recent learnings

#### Scenario: Guidance for Aeterna tools
- **WHEN** the system prompt is transformed
- **THEN** the plugin SHALL include instructions for using Aeterna tools
- **AND** the AI SHALL be guided to capture useful patterns with `aeterna_memory_add`

### Requirement: Tool Execute Before Hook

The system SHALL implement the `tool.execute.before` hook to validate and enrich tool arguments before execution.

#### Scenario: Aeterna tool argument enrichment
- **WHEN** an Aeterna tool is about to be executed
- **THEN** the plugin SHALL enrich arguments with session context
- **AND** the plugin SHALL validate arguments against governance policies

#### Scenario: Permission pre-check
- **WHEN** `aeterna_knowledge_propose` is about to be executed
- **THEN** the plugin SHALL verify the user has proposal permissions
- **AND** the plugin SHALL add proposer identity to the arguments

### Requirement: Tool Execute After Hook

The system SHALL implement the `tool.execute.after` hook to capture tool executions as working memory.

#### Scenario: Tool execution capture
- **WHEN** any tool execution completes
- **THEN** the plugin SHALL record the execution in Aeterna
- **AND** the plugin SHALL include tool name, arguments, output, and timestamp

#### Scenario: Significance detection
- **WHEN** a tool execution is captured
- **THEN** the plugin SHALL analyze the execution for significance
- **AND** significant executions SHALL be flagged for promotion

#### Scenario: Error pattern detection
- **WHEN** a tool execution follows a previous error
- **THEN** the plugin SHALL detect the error-resolution pattern
- **AND** the plugin SHALL flag the pattern as significant

#### Scenario: Captured arguments reflect executed tool call
- **WHEN** the plugin captures a completed tool execution
- **THEN** the stored execution record SHALL preserve the executed arguments for that tool call
- **AND** the plugin SHALL NOT replace captured arguments with an empty placeholder object

### Requirement: Permission Hook

The system SHALL implement the `permission.ask` hook to integrate with Aeterna's governance system.

#### Scenario: Knowledge proposal permission
- **WHEN** `aeterna_knowledge_propose` requires permission
- **THEN** the plugin SHALL check the user's role in Aeterna
- **AND** the plugin SHALL deny if user lacks proposal permissions

#### Scenario: Sensitive operation permission
- **WHEN** a tool accesses team or org level resources
- **THEN** the plugin SHALL verify scope-appropriate permissions
- **AND** the plugin SHALL log permission checks for audit

### Requirement: Event Hook

The system SHALL implement the `event` hook to handle session lifecycle events.

#### Scenario: Session end event
- **WHEN** an OpenCode session ends
- **THEN** the plugin SHALL call `client.sessionEnd()`
- **AND** the plugin SHALL promote significant memories to session layer
- **AND** the plugin SHALL generate a session summary

#### Scenario: Session start event
- **WHEN** a new OpenCode session starts
- **THEN** the plugin SHALL initialize session context
- **AND** the plugin SHALL subscribe to governance notifications

#### Scenario: Session startup does not create duplicate backend sessions
- **WHEN** the plugin starts and later receives the `session.start` event for the same OpenCode session
- **THEN** the plugin SHALL maintain a single active Aeterna session context for that OpenCode session
- **AND** the plugin SHALL NOT create duplicate backend sessions for the same logical start

### Requirement: MCP Server Alternative

The system SHALL provide an MCP server as an alternative integration method for remote or hybrid deployments.

The MCP server MUST:
- Support stdio transport for local use
- Support HTTP transport for remote use
- Expose the supported Aeterna tools via MCP protocol
- Expose knowledge and memory resources

#### Scenario: MCP stdio transport
- **WHEN** MCP server is configured with local type
- **THEN** OpenCode SHALL spawn the `aeterna-mcp` process
- **AND** communicate via stdin/stdout using JSON-RPC

#### Scenario: MCP HTTP transport
- **WHEN** MCP server is configured with remote type
- **THEN** OpenCode SHALL connect to the configured URL
- **AND** authenticate using Bearer token

#### Scenario: MCP tool invocation
- **WHEN** OpenCode invokes an Aeterna tool via MCP
- **THEN** the MCP server SHALL translate to internal operation
- **AND** return results in MCP-compliant format

#### Scenario: MCP HTTP Transport via Server Runtime
- **WHEN** the Aeterna server is running with `aeterna serve`
- **THEN** the MCP HTTP transport SHALL be served on the main HTTP port at `/mcp/*`
- **AND** the transport SHALL follow the MCP 2024-11-05 streamable HTTP spec (SSE)
- **AND** the MCP server process management (health, restart) is handled by the server runtime rather than a standalone supervisor

### Requirement: Automatic Session Capture

The system SHALL automatically capture session context during OpenCode coding sessions without requiring explicit user action.

Capture events:
- Tool invocations and their results
- Chat messages with significant content
- File modifications (via tool execution context)

#### Scenario: Automatic working memory creation
- **WHEN** a tool execution completes
- **THEN** the plugin SHALL create a working memory entry
- **AND** the memory SHALL include execution context and outcome

#### Scenario: Session summary generation
- **WHEN** a session ends
- **THEN** the plugin SHALL analyze all captured memories
- **AND** the plugin SHALL generate a session summary memory
- **AND** significant memories SHALL be promoted to session layer

### Requirement: Significance Detection

The system SHALL automatically detect significant learnings that warrant promotion from working memory to session memory.

Significance criteria:
- **Error resolution**: Error followed by successful outcome
- **Repeated pattern**: Similar queries 3+ times
- **Novel approach**: Solution differs from existing knowledge
- **Explicit capture**: User explicitly called `aeterna_memory_add`

#### Scenario: Error resolution detection
- **WHEN** a successful outcome follows an error in the session
- **THEN** the plugin SHALL flag the resolution as significant
- **AND** auto-promote the error-solution pair

#### Scenario: Repeated pattern detection
- **WHEN** similar queries are detected 3+ times
- **THEN** the plugin SHALL consolidate into a single memory
- **AND** flag for promotion

#### Scenario: Repeated pattern history is recorded
- **WHEN** the plugin analyzes tool executions for repeated significant patterns
- **THEN** the execution history used for repeated-pattern detection SHALL be updated for each completed execution
- **AND** repeated-pattern significance SHALL be based on actual recorded session history rather than an empty in-memory baseline

### Requirement: Knowledge Context Injection

The system SHALL proactively inject relevant knowledge into chat context based on the conversation.

#### Scenario: Semantic knowledge matching
- **WHEN** a user message is processed
- **THEN** the plugin SHALL query knowledge with semantic similarity
- **AND** return top-N matches above threshold

#### Scenario: Scoped knowledge priority
- **WHEN** multiple knowledge items match
- **THEN** project-level knowledge SHALL have highest priority
- **AND** team, org, tenant knowledge SHALL follow in order

#### Scenario: Context token limits
- **WHEN** injecting knowledge context
- **THEN** the plugin SHALL respect configured token limits
- **AND** truncate or omit lower-priority items as needed

### Requirement: Governance Notifications

The system SHALL subscribe to governance events and surface relevant notifications.

Notification types:
- `ProposalApproved`: Knowledge proposal approved
- `ProposalRejected`: Proposal rejected with reason
- `DriftDetected`: Semantic drift detected in project

#### Scenario: Governance event subscription
- **WHEN** a session starts
- **THEN** the plugin SHALL subscribe to project governance events
- **AND** unsubscribe when session ends

#### Scenario: Notification surfacing
- **WHEN** a governance event is received
- **THEN** the plugin SHALL surface the notification to the user
- **AND** include relevant context and suggested actions

### Requirement: Project Initialization

The system SHALL provide CLI commands to initialize Aeterna for OpenCode projects.

#### Scenario: Project initialization
- **WHEN** a user runs `npx aeterna init --opencode`
- **THEN** the system SHALL create `opencode.jsonc` with plugin config
- **AND** create `.aeterna/config.toml` with project settings
- **AND** update `.gitignore` to exclude local files

#### Scenario: Existing project detection
- **WHEN** `opencode.jsonc` already exists
- **THEN** the system SHALL merge Aeterna plugin configuration
- **AND** preserve existing settings

### Requirement: Configuration Management

The system SHALL support configuration via `.aeterna/config.toml` for project-specific settings.

Configuration options:
- `capture.enabled`: Enable/disable automatic capture
- `capture.sensitivity`: Capture sensitivity (low/medium/high)
- `capture.auto_promote`: Enable auto-promotion
- `knowledge.injection_enabled`: Enable knowledge injection
- `knowledge.max_items`: Max items to inject
- `knowledge.threshold`: Similarity threshold
- `governance.notifications`: Enable governance notifications

#### Scenario: Configuration loading
- **WHEN** the plugin initializes
- **THEN** the plugin SHALL load configuration from `.aeterna/config.toml`
- **AND** apply default values for missing settings

#### Scenario: Configuration validation
- **WHEN** configuration is loaded
- **THEN** the plugin SHALL validate all settings
- **AND** report errors for invalid configurations

### Requirement: Dual Integration Support

The system SHALL support both NPM plugin and MCP server integration methods simultaneously.

#### Scenario: Plugin-only mode
- **WHEN** only the NPM plugin is configured
- **THEN** all features SHALL work via plugin hooks

#### Scenario: MCP-only mode
- **WHEN** only the MCP server is configured
- **THEN** tools SHALL be available but hooks limited to MCP capabilities

#### Scenario: Hybrid mode
- **WHEN** both plugin and MCP are configured
- **THEN** the plugin SHALL handle local operations
- **AND** MCP SHALL handle remote knowledge/governance sync

### Requirement: Plugin SDK Version Stability (OC-C1)
The system SHALL maintain stability guarantees despite underlying SDK pre-release status.

#### Scenario: SDK Version Pinning
- **WHEN** the plugin package builds
- **THEN** the `@opencode-ai/plugin` dependency MUST be pinned to an exact version
- **AND** the pinned version MUST be documented in CHANGELOG on update

#### Scenario: SDK Abstraction Layer
- **WHEN** implementing OpenCode SDK integration
- **THEN** the system MUST implement an abstraction layer between business logic and SDK
- **AND** this abstraction MUST allow SDK replacement without changing business logic

#### Scenario: SDK Compatibility Tests
- **WHEN** the SDK version changes
- **THEN** compatibility tests MUST verify all hooks work correctly
- **AND** tests MUST cover tool registration, chat hooks, and event handlers

### Requirement: Credential Security (OC-C2)
The system SHALL implement secure credential handling to prevent token exposure.

#### Scenario: Credential Masking in Logs
- **WHEN** debug logging is enabled
- **THEN** the system MUST mask `AETERNA_TOKEN` and other credentials
- **AND** masked values MUST use format `[REDACTED:...last4chars]`

#### Scenario: Secure Credential Storage
- **WHEN** credentials are persisted
- **THEN** the system MUST use secure storage (keychain/credential manager)
- **AND** credentials MUST NOT be stored in plain text files

#### Scenario: Token Rotation Support
- **WHEN** tokens are rotated
- **THEN** the system MUST support seamless token refresh
- **AND** ongoing operations MUST NOT be interrupted during rotation

### Requirement: Experimental Hook Fallback (OC-H1)
The system SHALL handle experimental hook API changes gracefully.

#### Scenario: Feature Flag for Experimental Hooks
- **WHEN** experimental hooks are used
- **THEN** the system MUST wrap them in feature flags
- **AND** each experimental hook MUST be individually configurable

#### Scenario: Fallback Behavior
- **WHEN** an experimental hook is unavailable or fails
- **THEN** the system MUST provide fallback behavior
- **AND** core functionality MUST remain operational

#### Scenario: Hook Availability Detection
- **WHEN** the plugin initializes
- **THEN** it MUST detect available hooks from SDK version
- **AND** warn when using hooks marked as experimental

### Requirement: Session Capture Performance (OC-H2)
The system SHALL minimize latency impact from tool execution capture.

#### Scenario: Async Capture
- **WHEN** `tool.execute.after` hook fires
- **THEN** the capture operation MUST be asynchronous
- **AND** MUST NOT block the tool response to the AI

#### Scenario: Capture Sampling
- **WHEN** tool execution rate exceeds threshold (default: 10/second)
- **THEN** the system MUST apply sampling
- **AND** sample rate MUST be configurable

#### Scenario: Capture Debouncing
- **WHEN** multiple similar tool executions occur rapidly
- **THEN** the system MUST debounce captures
- **AND** debounce window MUST be configurable (default: 500ms)

### Requirement: Knowledge Query Performance (OC-H3)
The system SHALL optimize knowledge injection latency.

#### Scenario: Pre-fetch on Session Start
- **WHEN** a session starts
- **THEN** the system MUST pre-fetch frequently accessed knowledge
- **AND** cache project and team-level knowledge for duration of session

#### Scenario: Query Caching
- **WHEN** a knowledge query is executed
- **THEN** results MUST be cached with TTL (default: 60 seconds)
- **AND** cache key MUST include query and filter parameters

#### Scenario: Timeout Fallback
- **WHEN** knowledge query exceeds timeout (default: 200ms)
- **THEN** the system MUST return cached results if available
- **AND** proceed without injection if no cache available

### Requirement: Session State Persistence (OC-H4)
The system SHALL define and implement session state storage strategy.

#### Scenario: Redis Session Storage
- **WHEN** multiple plugin instances are deployed
- **THEN** session state MUST be stored in Redis
- **AND** state MUST be accessible from any instance

#### Scenario: Local Fallback Storage
- **WHEN** Redis is unavailable
- **THEN** the system MUST fall back to local file storage
- **AND** warn that multi-instance deployments may have inconsistent state

#### Scenario: Session State TTL
- **WHEN** session state is stored
- **THEN** it MUST have configurable TTL (default: 24 hours)
- **AND** expired sessions MUST be automatically cleaned up

### Requirement: MCP Server Health Management (OC-H5)
The system SHALL implement robust MCP server process management with runtime behavior aligned to actual backend connectivity and supported integration paths.

#### Scenario: Health Check Implementation
- **WHEN** MCP server is running
- **THEN** it MUST expose health check endpoint
- **AND** health check MUST verify backend connectivity

#### Scenario: Supervisor Pattern
- **WHEN** MCP server process crashes
- **THEN** the system MUST implement automatic restart
- **AND** restart MUST use exponential backoff (max 3 retries, then alert)

#### Scenario: Crash Recovery
- **WHEN** MCP server recovers from crash
- **THEN** it MUST restore in-flight request state if possible
- **AND** emit metrics for crash events

#### Scenario: Unsupported integration path
- **WHEN** a documented plugin or MCP integration mode is not fully implemented for the current build or deployment mode
- **THEN** the system SHALL fail explicitly during initialization
- **AND** the system SHALL NOT present the mode as healthy or ready
- **AND** documentation and examples SHALL identify the supported integration modes

### Requirement: Memory Layer Routing
The plugin SHALL transparently route memory operations to the local store or remote server based on the target layer.

#### Scenario: Personal layer routed locally
- **WHEN** a memory operation targets layer `agent`, `user`, or `session`
- **THEN** the plugin SHALL execute the operation against the `LocalMemoryManager`
- **AND** the plugin SHALL NOT send a synchronous HTTP request to the remote server

#### Scenario: Shared layer routed to remote with cache
- **WHEN** a memory search targets layer `project`, `team`, `org`, or `tenant`
- **THEN** the plugin SHALL check the local cache first
- **AND** if the cache has recent results (< 60s), the plugin SHALL return cached results
- **AND** if the cache is stale or empty, the plugin SHALL fetch from the remote server and update the cache

#### Scenario: Shared layer write routed to remote
- **WHEN** a memory write targets a shared layer
- **THEN** the plugin SHALL send the write to the remote server via HTTP
- **AND** the plugin SHALL NOT write directly to the local store
- **AND** the entry SHALL appear locally on the next pull cycle

### Requirement: MCP Promotion Lifecycle Tools
The OpenCode integration SHALL expose first-class MCP tools for knowledge promotion workflows.

#### Scenario: Preview promotion from MCP
- **WHEN** a client invokes `aeterna_knowledge_promotion_preview`
- **THEN** the tool SHALL return suggested shared content, residual content, and residual role

#### Scenario: Submit promotion from MCP
- **WHEN** a client invokes `aeterna_knowledge_promote`
- **THEN** the tool SHALL create a promotion request
- **AND** the tool SHALL return the request identifier and review status

#### Scenario: Review pending promotion requests
- **WHEN** a client invokes `aeterna_knowledge_review_pending`
- **THEN** the tool SHALL return pending promotion requests with target layer and reviewer context

### Requirement: MCP Backward Compatibility
The OpenCode integration SHALL preserve existing proposal tools while adding promotion lifecycle tools.

#### Scenario: Existing proposal tool still works
- **WHEN** a client invokes `aeterna_knowledge_propose`
- **THEN** the tool SHALL continue to behave as before
- **AND** promotion lifecycle tools SHALL be additive

