# @aeterna-org/opencode-plugin

OpenCode plugin for Aeterna memory and knowledge integration with CCA and RLM support.

## Features

- **Local-First Memory**: Embedded SQLite store for personal layers (agent/user/session) — works offline, syncs when connected
- **Memory Management**: Add, search, retrieve, and promote memories across 7 layers
- **Knowledge Repository**: Query and propose knowledge items with governance
- **Graph Layer**: Explore memory relationships and find paths
- **CCA Capabilities**:
  - Context Architect: Assemble hierarchical context with token budgeting
  - Note-Taking Agent: Capture trajectory events for distillation
  - Hindsight Learning: Query error patterns and resolutions
  - Meta-Agent: Build-test-improve loop status
- **Governance**: Check sync status and governance compliance
- **Automatic Capture**: Tool executions captured as working memory
- **Knowledge Injection**: Relevant knowledge auto-injected into chat context

## Installation

Add the plugin to your global or project OpenCode config:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["@aeterna-org/opencode-plugin"]
}
```

OpenCode installs configured npm plugins automatically at startup.

## Configuration

Add the plugin to your `opencode.jsonc`:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",

  "plugin": ["@aeterna-org/opencode-plugin"]
}
```

### Environment Variables

| Variable | Description | Default |
|-----------|-------------|---------|
| `AETERNA_SERVER_URL` | Aeterna server URL | `http://localhost:8080` |
| `AETERNA_TOKEN` | API authentication token | (required for API calls) |
| `AETERNA_TEAM` | Team context for multi-tenant hierarchy | (optional) |
| `AETERNA_ORG` | Organization context for multi-tenant hierarchy | (optional) |
| `AETERNA_USER_ID` | User ID for personalization | (optional) |
| `AETERNA_LOCAL_ENABLED` | Enable local-first memory store | `true` |
| `AETERNA_LOCAL_DB_PATH` | SQLite database path | `~/.aeterna/local.db` |
| `AETERNA_LOCAL_SYNC_PUSH_INTERVAL_MS` | Push sync interval (ms) | `30000` |
| `AETERNA_LOCAL_SYNC_PULL_INTERVAL_MS` | Pull sync interval (ms) | `60000` |
| `AETERNA_LOCAL_MAX_CACHED_ENTRIES` | Max cached shared-layer entries | `50000` |
| `AETERNA_LOCAL_SESSION_STORAGE_TTL_HOURS` | Session memory retention (hours) | `24` |
| `AETERNA_PLUGIN_AUTH_ENABLED` | Enable interactive GitHub OAuth plugin auth (device flow) | `false` |
| `AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID` | GitHub OAuth App client ID for plugin auth | (required when auth enabled) |
| `AETERNA_PLUGIN_REFRESH_TOKEN` | Cached refresh token for silent re-auth on start | (optional) |

### Authentication

By default the plugin authenticates with a static bearer token set via `AETERNA_TOKEN`. For interactive use the plugin supports a GitHub OAuth device flow that exchanges a GitHub access token for Aeterna-issued short-lived JWTs.

#### Token Priority

1. **Static token** (`AETERNA_TOKEN`) — set this for service-to-service or CI use. The plugin uses it as-is; no interactive auth is performed.
2. **Dynamic auth** (`AETERNA_PLUGIN_AUTH_ENABLED=true`) — the plugin performs the GitHub OAuth device flow sign-in and stores a rotating refresh token. The access token is renewed automatically on each session start.
2. **Dynamic auth** (`AETERNA_PLUGIN_AUTH_ENABLED=true`) — the plugin performs the GitHub OAuth device flow sign-in and stores a rotating refresh token. The access token is refreshed automatically and reused across later starts until re-auth is required.
3. **Unauthenticated** — if neither is configured the plugin runs in local-only mode; personal memory layers work offline, shared layers return empty results.

#### Interactive Sign-in (Device Flow)

```bash
# Set these in your shell or .env before starting OpenCode:
export AETERNA_PLUGIN_AUTH_ENABLED=true
export AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID=<your-oauth-app-client-id>

# On first start the plugin prints a verification URL and a user code.
# Open the URL in your browser, enter the code, and authorise the app.
# The plugin polls GitHub automatically and completes sign-in.
```

#### Migration from Static Tokens

Existing deployments using `AETERNA_TOKEN` continue to work without any change. Interactive auth is opt-in; the static token path is preserved.

#### Server-side Configuration

```bash
AETERNA_PLUGIN_AUTH_ENABLED=true
AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID=<oauth-app-client-id>
AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_SECRET=<oauth-app-client-secret>
AETERNA_PLUGIN_AUTH_JWT_SECRET=<min-32-char-random-secret>
# Optional defaults:
AETERNA_PLUGIN_AUTH_ACCESS_TOKEN_TTL_SECONDS=3600
AETERNA_PLUGIN_AUTH_REFRESH_TOKEN_TTL_SECONDS=2592000
AETERNA_PLUGIN_AUTH_TOKEN_ISSUER=aeterna
```

### Aeterna Configuration

The plugin also supports a `.aeterna/config.toml` file for project-specific settings:

```toml
[capture]
enabled = true
sensitivity = "medium"  # low, medium, high
auto_promote = true
sample_rate = 1.0
debounce_ms = 500

[knowledge]
injection_enabled = true
max_items = 3
threshold = 0.75
cache_ttl_seconds = 60
timeout_ms = 200

[governance]
notifications = true
drift_alerts = true

[session]
storage_ttl_hours = 24
use_redis = false

[experimental]
system_prompt_hook = true
permission_hook = true

[local]
enabled = true
db_path = "~/.aeterna/local.db"
sync_push_interval_ms = 30000
sync_pull_interval_ms = 60000
max_cached_entries = 50000
session_storage_ttl_hours = 24
```

## Usage

### Memory Tools

- `aeterna_memory_add` - Capture learnings, solutions, or important context
- `aeterna_memory_search` - Find semantically similar memories
- `aeterna_memory_get` - Retrieve a specific memory by ID
- `aeterna_memory_promote` - Promote memory to higher layers

### Knowledge Tools

- `aeterna_knowledge_query` - Search knowledge repository by scope and type
- `aeterna_knowledge_propose` - Propose new knowledge (requires approval)

### Graph Tools

- `aeterna_graph_query` - Query memory relationships and traversals
- `aeterna_graph_neighbors` - Find memories directly related to a memory
- `aeterna_graph_path` - Find shortest path between two memories

### CCA Tools

- `aeterna_context_assemble` - Assemble hierarchical context from memory layers
- `aeterna_note_capture` - Capture trajectory events for note distillation
- `aeterna_hindsight_query` - Find error patterns and resolutions
- `aeterna_meta_loop_status` - Get meta-agent build-test-improve loop status

### Governance Tools

- `aeterna_sync_status` - Check sync status between memory and knowledge
- `aeterna_governance_status` - Check governance state and compliance

## Hooks

The plugin implements several OpenCode hooks for deep integration:

1. **Chat Message Hook** - Injects relevant knowledge and memories into user messages
2. **System Prompt Hook** - Adds project context, active policies, and Aeterna guidance to system prompt
3. **Tool Execute Before Hook** - Enriches Aeterna tool arguments with session context
4. **Tool Execute After Hook** - Captures tool executions as working memory, flags significant patterns for promotion
5. **Permission Hook** - Validates knowledge proposal permissions
6. **Session Event Hook** - Manages session lifecycle and cleanup

## Local-First Memory Architecture

The plugin includes an embedded SQLite store for personal memory layers (agent, user, session). This enables offline-first operation with automatic bidirectional sync when a server is available.

### Layer Ownership

| Layer | Storage | Write Path | Read Path |
|-------|---------|------------|-----------|
| agent, user, session | Local SQLite | Direct local write | Local query |
| project, team, org, tenant | Remote server | HTTP API | Local cache, remote fallback |

### How It Works

1. **Personal layers** (agent/user/session) are owned locally — reads and writes go directly to the embedded SQLite database at `~/.aeterna/local.db`
2. **Shared layers** (project/team/org/tenant) are owned by the remote server — writes go via HTTP, reads check the local cache first (< 60s staleness) then fall back to HTTP
3. **Sync engine** runs in the background:
   - **Push** every 30s: local changes queued and batch-pushed to the server
   - **Pull** every 60s: shared-layer updates pulled and cached locally
4. **Offline resilience**: personal layers work without any server connection. Changes queue up and sync when connectivity is restored
5. **Conflict resolution**: server-wins for same-ID conflicts (remote has newer `updated_at`)

### Sync Status

Use the `aeterna_sync_status` tool to check:
- Pending push count (queued local changes)
- Last push/pull timestamps
- Server connectivity
- Local store size (entry counts per layer)

Memory search results include `source` metadata (`local`, `cache`, or `remote`) and staleness warnings when cached data is older than 10 minutes.

## How It Works

1. **Session Start**: When an OpenCode session starts, the plugin:
   - Initializes the local SQLite memory store
   - Starts the sync engine (push/pull background timers)
   - Initializes the Aeterna client with memory router
   - Starts a session context on the backend
   - Prefetches frequently accessed knowledge
   - Subscribes to governance events

2. **Automatic Knowledge Injection**:
   - When you send a message, the plugin automatically:
     - Queries relevant knowledge based on message content
     - Searches session memories for recent context
     - Injects the combined context into the chat

3. **Tool Execution Capture**:
    - Every tool execution is automatically captured with its executed arguments and outcome
    - Significant patterns (error resolution, repeated usage) are flagged
    - Captured memories are available for future sessions

4. **Session End**: When session ends, the plugin:
   - Flushes pending sync queue (final push with 5s timeout)
   - Generates a session summary
   - Flushes all pending captures to the backend
   - Promotes significant memories to broader layers
   - Closes local SQLite database cleanly
   - Cleans up subscriptions and caches

## Development

```bash
# Install dependencies
npm install

# Build the plugin
npm run build

# Watch mode for development
npm run dev

# Type check
npm run typecheck
```

## Integration with Rust Backend

The plugin communicates with Aeterna's Rust backend via HTTP API:

- **Memory**: `/api/v1/memories/*` operations
- **Knowledge**: `/api/v1/knowledge/*` operations
- **Graph**: `/api/v1/graph/*` operations
- **CCA**: `/api/v1/cca/*` operations
- **Governance**: `/api/v1/governance/*` operations
- **Session**: `/api/v1/sessions/*` operations
- **Sync**: `/api/v1/sync/push` and `/api/v1/sync/pull` for local-first memory synchronization

All requests include proper authentication and tenant context headers.

## License

Apache License 2.0 - See [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please read the main Aeterna repository for guidelines.

## Support

- [Aeterna Documentation](https://github.com/kikokikok/aeterna)
- [OpenCode Documentation](https://opencode.ai/docs)
- Report issues in [Aeterna Repository](https://github.com/kikokikok/aeterna/issues)

## Compatibility

- Requires `@opencode-ai/plugin` version 1.3.0 or higher
- Node.js version 18.0.0 or higher
- Requires Aeterna backend server for shared layers (personal layers work offline)
- Bun runtime SQLite support is used for local memory store inside OpenCode

## Changelog

### 0.4.0

- Migrated plugin authentication from browser redirect/code exchange to GitHub OAuth device flow
- `bootstrapAuth()` now accepts a GitHub access token (obtained via device flow) instead of an authorization code + redirect URI
- Added `requestDeviceCode()` to initiate the device flow and return `user_code` + `verification_uri`
- Added `pollDeviceToken()` to poll GitHub for the OAuth access token after user authorisation
- Plugin entry point now prints verification URL and user code, then polls automatically (no restart needed)
- Removed `AETERNA_PLUGIN_AUTH_REDIRECT_URI` and `AETERNA_OAUTH_CODE` env vars (no longer needed)
- Static `AETERNA_TOKEN` precedence and refresh/logout behaviour unchanged

### 0.3.0

- Added interactive GitHub OAuth plugin authentication flow (task add-opencode-github-app-auth)
- `AeternaClient` now holds a mutable access token updated on bootstrap/refresh
- Added `bootstrapAuth()`, `refreshAuth()`, `logoutAuth()`, `setAuthTokens()`, `getAccessToken()`, `hasRefreshToken()` methods
- Plugin entry point attempts silent token refresh or interactive bootstrap before `sessionStart()`
- Session hook performs silent token refresh on `session.start` events when a refresh token is held
- `callWithReauth()` helper retries API calls after a 401 by refreshing the token
- Static `AETERNA_TOKEN` is preserved as first-priority fallback (no behaviour change for existing deployments)
- New env vars: `AETERNA_PLUGIN_AUTH_ENABLED`, `AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID`, `AETERNA_PLUGIN_REFRESH_TOKEN`

### 0.2.3

- Replaced unsupported `bun:sqlite` `.pragma()` helper calls with SQL `PRAGMA` statements that work in OpenCode's Bun runtime
- Verified local-first SQLite initialization under Bun with passing tests, typecheck, and build

### 0.2.2

- Replaced `better-sqlite3` with Bun-compatible `bun:sqlite` so the plugin loads inside OpenCode
- Kept local-first SQLite storage for personal layers while removing the unsupported native Node addon
- Corrected installation instructions to use the official OpenCode `plugin` config flow

### 0.2.1

- Added `oc-plugin` target declaration for OpenCode 1.3.0 plugin discovery
- Added `./server` export path required by OpenCode plugin loader
- Fixed plugin package metadata required by the OpenCode loader

### 0.2.0

- Added local-first memory architecture with embedded SQLite store
- Personal layers (agent/user/session) work fully offline
- Bidirectional sync engine with push/pull cycles and exponential backoff
- Shared-layer caching with configurable staleness thresholds
- Memory router dispatches reads/writes by layer ownership
- Sync status tool with data provenance and staleness warnings
- Configurable via `[local]` section in `.aeterna/config.toml` or environment variables
- Upgraded to `@opencode-ai/plugin` v1.3.6 with Zod v4 schemas
- Updated `@types/node` to v22, TypeScript to v5.8, Vitest to v3.1
- Fixed `experimental.chat.system.transform` hook for optional `sessionID`

### 0.1.0

- Initial release with memory, knowledge, graph, CCA, and governance tools
- Updated to `@opencode-ai/plugin` v1.1.36 API (Zod v4, new hook signatures)
