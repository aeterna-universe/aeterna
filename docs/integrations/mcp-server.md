# MCP Server Integration Guide

Comprehensive guide for integrating Aeterna via Model Context Protocol (MCP) servers.

---

## Table of Contents

1. [Overview](#overview)
2. [When to Use MCP Server](#when-to-use-mcp-server)
3. [Quick Start](#quick-start)
4. [Server Transports](#server-transports)
5. [Tool Registration](#tool-registration)
6. [Resource Exposure](#resource-exposure)
7. [Authentication](#authentication)
8. [Configuration Reference](#configuration-reference)
9. [Deployment Patterns](#deployment-patterns)
10. [Comparison: NPM Plugin vs MCP Server](#comparison-npm-plugin-vs-mcp-server)
11. [Troubleshooting](#troubleshooting)

---

## Overview

Aeterna provides an MCP (Model Context Protocol) server for AI assistant integrations. MCP is an open protocol that enables AI assistants to interact with external tools and data sources in a standardized way.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         MCP SERVER ARCHITECTURE                              │
│                                                                              │
│   ┌────────────────┐     ┌────────────────┐     ┌────────────────┐          │
│   │    OpenCode    │     │     Claude     │     │   Other MCP    │          │
│   │    Client      │     │    Desktop     │     │   Clients      │          │
│   └───────┬────────┘     └───────┬────────┘     └───────┬────────┘          │
│           │                      │                      │                    │
│           └──────────────────────┼──────────────────────┘                    │
│                                  │                                           │
│                                  ▼                                           │
│                    ┌─────────────────────────┐                               │
│                    │       MCP PROTOCOL      │                               │
│                    │  - ListTools            │                               │
│                    │  - CallTool             │                               │
│                    │  - ListResources        │                               │
│                    │  - ReadResource         │                               │
│                    └────────────┬────────────┘                               │
│                                 │                                            │
│                                 ▼                                            │
│   ┌─────────────────────────────────────────────────────────────────┐       │
│   │                     AETERNA MCP SERVER                           │       │
│   │                                                                  │       │
│   │   Transport: Stdio (local) or HTTP/SSE (remote)                 │       │
│   │                                                                  │       │
│   │   ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐    │       │
│   │   │     8 Tools     │  │    Resources    │  │     Auth     │    │       │
│   │   │  memory_*       │  │  knowledge://   │  │  JWT/API Key │    │       │
│   │   │  knowledge_*    │  │  memory://      │  │  Cedar       │    │       │
│   │   │  sync_*         │  │  governance://  │  │              │    │       │
│   │   └─────────────────┘  └─────────────────┘  └──────────────┘    │       │
│   │                                                                  │       │
│   └──────────────────────────────┬───────────────────────────────────┘       │
│                                  │                                           │
│                                  ▼                                           │
│   ┌─────────────────────────────────────────────────────────────────┐       │
│   │                      AETERNA BACKEND                             │       │
│   │                                                                  │       │
│   │   Memory (Qdrant)  │  Knowledge (Git)  │  Governance (Cedar)    │       │
│   │                                                                  │       │
│   └─────────────────────────────────────────────────────────────────┘       │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## When to Use MCP Server

### Choose MCP Server When:

| Scenario | Why MCP Server |
|----------|----------------|
| **Remote/centralized deployment** | Single server for multiple clients |
| **Enterprise fleet** | Consistent configuration across 100s of developers |
| **Strict security requirements** | Centralized auth, audit logging |
| **Non-OpenCode clients** | Claude Desktop, custom MCP clients |
| **Network isolation** | Backend behind firewall, MCP as gateway |

### Choose NPM Plugin When:

| Scenario | Why NPM Plugin |
|----------|----------------|
| **Local development** | Full hooks, deep integration |
| **Rich context injection** | System prompt, chat hooks |
| **Session lifecycle** | Automatic capture, promotion |
| **OpenCode-specific features** | Permission hooks, event handling |

**Rule of thumb**: Start with NPM Plugin for development, deploy MCP Server for enterprise.

---

## Quick Start

### Local Setup (Stdio Transport)

1. **Install the MCP server:**

```bash
cargo install aeterna-mcp
```

2. **Configure OpenCode:**

```jsonc
// opencode.jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "aeterna": {
      "type": "local",
      "command": ["aeterna-mcp", "--mode", "stdio"],
      "env": {
        "AETERNA_PROJECT": "${project.name}",
        "AETERNA_CONFIG": ".aeterna/config.toml"
      }
    }
  }
}
```

3. **Start using:**

```
You: Search my memory for database preferences

AI: [Uses aeterna_memory_search]
Found 2 relevant memories from Aeterna...
```

### Remote Setup (HTTP Transport)

1. **Deploy the MCP server:**

```bash
# Start HTTP server
aeterna-mcp --mode http --port 8081

# Or with Docker
docker run -p 8081:8081 aeterna/mcp-server:latest
```

2. **Configure OpenCode:**

```jsonc
// opencode.jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "aeterna": {
      "type": "remote",
      "url": "https://aeterna.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${AETERNA_TOKEN}"
      }
    }
  }
}
```

---

## Server Transports

### Stdio Transport (Local)

The stdio transport runs the MCP server as a subprocess, communicating via stdin/stdout.

```
┌─────────────┐     stdin      ┌─────────────────┐
│   OpenCode  │ ─────────────▶ │  aeterna-mcp    │
│   (parent)  │ ◀───────────── │   (subprocess)  │
└─────────────┘    stdout      └─────────────────┘
```

**Advantages:**
- Zero network configuration
- Process isolation per client
- No authentication required (trusts parent process)

**Configuration:**

```jsonc
{
  "mcp": {
    "aeterna": {
      "type": "local",
      "command": ["aeterna-mcp", "--mode", "stdio"],
      "env": {
        "AETERNA_PROJECT": "my-project",
        "AETERNA_QDRANT_URL": "http://localhost:6333",
        "AETERNA_KNOWLEDGE_PATH": "./knowledge-repo"
      }
    }
  }
}
```

**Command-line options:**

```bash
aeterna-mcp --mode stdio [OPTIONS]

Options:
  --config <PATH>       Path to config file [default: .aeterna/config.toml]
  --project <NAME>      Project name override
  --log-level <LEVEL>   Log level: error, warn, info, debug, trace
```

---

### HTTP Transport (Remote)

The HTTP transport exposes the MCP server over HTTP with Server-Sent Events (SSE) for streaming.

```
┌─────────────┐    HTTPS     ┌─────────────────┐
│   OpenCode  │ ◀──────────▶ │   aeterna-mcp   │
│   (client)  │    SSE       │   (HTTP server) │
└─────────────┘              └─────────────────┘
```

**Advantages:**
- Centralized deployment
- Enterprise authentication
- Audit logging
- Load balancing possible

**Configuration:**

```jsonc
{
  "mcp": {
    "aeterna": {
      "type": "remote",
      "url": "https://aeterna.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${AETERNA_TOKEN}",
        "X-Tenant-ID": "acme-corp"
      }
    }
  }
}
```

**Server startup:**

```bash
aeterna-mcp --mode http [OPTIONS]

Options:
  --port <PORT>         HTTP port [default: 8081]
  --host <HOST>         Bind address [default: 0.0.0.0]
  --tls-cert <PATH>     TLS certificate path
  --tls-key <PATH>      TLS private key path
  --auth <MODE>         Auth mode: none, api-key, jwt [default: api-key]
  --config <PATH>       Path to config file
```

---

## Tool Registration

The MCP server exposes all 8 Aeterna tools via the MCP protocol.

### Tool Listing

```typescript
// Client request
{ method: "tools/list" }

// Server response
{
  tools: [
    {
      name: "aeterna_memory_add",
      description: "Add a memory entry to Aeterna...",
      inputSchema: {
        type: "object",
        properties: {
          content: { type: "string", description: "The content to remember" },
          layer: { 
            type: "string", 
            enum: ["working", "session", "episodic"],
            description: "Memory layer (default: working)" 
          },
          tags: { 
            type: "array", 
            items: { type: "string" },
            description: "Tags for categorization" 
          },
          importance: { 
            type: "number", 
            minimum: 0, 
            maximum: 1,
            description: "Importance score 0-1" 
          }
        },
        required: ["content"]
      }
    },
    // ... 7 more tools
  ]
}
```

### Tool Invocation

```typescript
// Client request
{
  method: "tools/call",
  params: {
    name: "aeterna_memory_search",
    arguments: {
      query: "database preferences",
      limit: 5,
      threshold: 0.7
    }
  }
}

// Server response
{
  content: [
    {
      type: "text",
      text: "Found 3 relevant memories:\n\n1. \"Use PostgreSQL for new services\" (project, 0.92)\n2. \"User prefers NoSQL for analytics\" (user, 0.85)\n3. \"Team standard: connection pooling\" (team, 0.78)"
    }
  ]
}
```

### Complete Tool Reference

| Tool | Description | Required Args | Optional Args |
|------|-------------|---------------|---------------|
| `aeterna_memory_add` | Store memory | `content` | `layer`, `tags`, `importance` |
| `aeterna_memory_search` | Search memories | `query` | `layers`, `limit`, `threshold` |
| `aeterna_memory_get` | Get by ID | `memoryId` | - |
| `aeterna_memory_promote` | Promote layer | `memoryId`, `targetLayer` | `reason` |
| `aeterna_knowledge_query` | Search knowledge | `query` | `scope`, `types`, `limit` |
| `aeterna_knowledge_propose` | Propose item | `type`, `title`, `summary`, `content` | `tags` |
| `aeterna_sync_status` | Check sync | - | - |
| `aeterna_governance_status` | Check governance | - | `checkViolations` |

---

## Resource Exposure

MCP servers can expose resources that clients can read. Aeterna exposes knowledge and memory as resources.

### Resource Listing

```typescript
// Client request
{ method: "resources/list" }

// Server response
{
  resources: [
    {
      uri: "aeterna://knowledge/project",
      name: "Project Knowledge",
      description: "ADRs, policies, and patterns for the current project",
      mimeType: "application/json"
    },
    {
      uri: "aeterna://knowledge/team",
      name: "Team Knowledge",
      description: "Team-level knowledge items"
    },
    {
      uri: "aeterna://memory/session",
      name: "Session Memory",
      description: "Current session working memory"
    },
    {
      uri: "aeterna://memory/user",
      name: "User Memory",
      description: "User preferences and history"
    },
    {
      uri: "aeterna://governance/policies",
      name: "Active Policies",
      description: "Policies affecting current context"
    }
  ]
}
```

### Resource Reading

```typescript
// Client request
{
  method: "resources/read",
  params: {
    uri: "aeterna://knowledge/project"
  }
}

// Server response
{
  contents: [
    {
      uri: "aeterna://knowledge/project",
      mimeType: "application/json",
      text: JSON.stringify({
        adrs: [
          { id: "adr-042", title: "Database Selection", status: "accepted" },
          { id: "adr-047", title: "TigerBeetle Ledger", status: "accepted" }
        ],
        policies: [
          { id: "security-baseline", severity: "block" }
        ],
        patterns: [
          { id: "strangler-facade", category: "migration" }
        ]
      })
    }
  ]
}
```

### Resource URIs

| URI Pattern | Description | Example |
|-------------|-------------|---------|
| `aeterna://knowledge/{scope}` | Knowledge by scope | `aeterna://knowledge/team` |
| `aeterna://knowledge/{scope}/{type}` | Knowledge by type | `aeterna://knowledge/project/adr` |
| `aeterna://knowledge/item/{id}` | Specific item | `aeterna://knowledge/item/adr-042` |
| `aeterna://memory/{layer}` | Memory by layer | `aeterna://memory/session` |
| `aeterna://memory/item/{id}` | Specific memory | `aeterna://memory/item/mem_abc123` |
| `aeterna://governance/policies` | Active policies | - |
| `aeterna://governance/violations` | Current violations | - |

---

## Authentication

### No Authentication (Development)

For local development with stdio transport:

```bash
aeterna-mcp --mode stdio --auth none
```

### API Key Authentication

For simple deployments:

```bash
# Server startup
aeterna-mcp --mode http --auth api-key

# Generate API key
aeterna-mcp keygen --name "dev-team" --expires 30d
# Output: aet_k_abc123...

# Client configuration
export AETERNA_TOKEN="aet_k_abc123..."
```

**API Key Format:**
```
aet_k_{random_32_chars}
```

**Header:**
```
Authorization: Bearer aet_k_abc123...
```

### JWT Authentication (Enterprise)

For enterprise deployments with identity providers:

```bash
# Server startup
aeterna-mcp --mode http --auth jwt \
  --jwt-issuer "https://auth.example.com" \
  --jwt-audience "aeterna-mcp"
```

**Configuration file:**

```toml
# aeterna-mcp.toml
[auth]
mode = "jwt"

[auth.jwt]
issuer = "https://auth.example.com"
audience = "aeterna-mcp"
jwks_url = "https://auth.example.com/.well-known/jwks.json"
# Required claims for authorization
required_claims = ["sub", "tenant_id"]
# Map JWT claims to Aeterna context
claim_mappings = { tenant_id = "tenant", department = "org", team = "team" }
```

**JWT Claims Mapping:**

| JWT Claim | Aeterna Context | Usage |
|-----------|-----------------|-------|
| `sub` | User ID | Memory layer filtering |
| `tenant_id` | Tenant | Multi-tenant isolation |
| `department` | Org | Org-level knowledge |
| `team` | Team | Team-level knowledge |
| `roles` | RBAC roles | Permission checks |

### Cedar Authorization

For fine-grained access control:

```cedar
// Allow users to search memories in their scope
permit (
    principal,
    action == Action::"MemorySearch",
    resource
)
when {
    resource.layer in principal.accessibleLayers
};

// Allow tech leads to propose knowledge
permit (
    principal,
    action == Action::"KnowledgePropose",
    resource
)
when {
    principal.role == "TechLead" ||
    principal.role == "Architect" ||
    principal.role == "Admin"
};

// Deny cross-tenant access
forbid (
    principal,
    action,
    resource
)
when {
    principal.tenantId != resource.tenantId
};
```

---

## Configuration Reference

### Server Configuration File

```toml
# aeterna-mcp.toml

[server]
mode = "http"           # stdio, http
port = 8081
host = "0.0.0.0"

[server.tls]
enabled = true
cert_path = "/etc/aeterna/tls/cert.pem"
key_path = "/etc/aeterna/tls/key.pem"

[auth]
mode = "jwt"            # none, api-key, jwt

[auth.api_key]
keys_file = "/etc/aeterna/api-keys.json"

[auth.jwt]
issuer = "https://auth.example.com"
audience = "aeterna-mcp"
jwks_url = "https://auth.example.com/.well-known/jwks.json"

[backend]
# Aeterna backend configuration
[backend.memory]
provider = "qdrant"
url = "http://qdrant:6333"

[backend.knowledge]
provider = "git"
repository = "/data/knowledge-repo"

[backend.governance]
provider = "cedar"
schema_path = "/etc/aeterna/cedar.cedarschema"

[logging]
level = "info"          # error, warn, info, debug, trace
format = "json"         # text, json
output = "stdout"       # stdout, file
file_path = "/var/log/aeterna/mcp.log"

[metrics]
enabled = true
port = 9090
path = "/metrics"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AETERNA_MCP_MODE` | Server mode (stdio, http) | `stdio` |
| `AETERNA_MCP_PORT` | HTTP port | `8081` |
| `AETERNA_MCP_HOST` | Bind address | `0.0.0.0` |
| `AETERNA_MCP_AUTH` | Auth mode | `api-key` |
| `AETERNA_QDRANT_URL` | Qdrant server URL | `http://localhost:6333` |
| `AETERNA_KNOWLEDGE_PATH` | Git repository path | `./knowledge-repo` |
| `AETERNA_LOG_LEVEL` | Log level | `info` |
| `AETERNA_JWT_ISSUER` | JWT issuer URL | - |
| `AETERNA_JWT_AUDIENCE` | JWT audience | - |

### Client Configuration (OpenCode)

```jsonc
// opencode.jsonc - Full reference
{
  "$schema": "https://opencode.ai/config.json",
  
  "mcp": {
    "aeterna": {
      // Local (stdio) configuration
      "type": "local",
      "command": ["aeterna-mcp", "--mode", "stdio"],
      "env": {
        "AETERNA_PROJECT": "${project.name}",
        "AETERNA_CONFIG": ".aeterna/config.toml",
        "AETERNA_LOG_LEVEL": "info"
      },
      
      // OR Remote (HTTP) configuration
      "type": "remote",
      "url": "https://aeterna.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${AETERNA_TOKEN}",
        "X-Tenant-ID": "${AETERNA_TENANT}",
        "X-Project": "${project.name}"
      },
      
      // Connection settings
      "timeout": 30000,           // Request timeout (ms)
      "retries": 3,               // Retry count on failure
      "retryDelay": 1000          // Delay between retries (ms)
    }
  }
}
```

---

## Deployment Patterns

### Pattern 1: Local Development

```
┌─────────────┐
│   OpenCode  │
└──────┬──────┘
       │ stdio
       ▼
┌─────────────┐     ┌─────────────┐
│ aeterna-mcp │────▶│   Qdrant    │ (Docker)
└──────┬──────┘     └─────────────┘
       │
       ▼
┌─────────────┐
│  Git Repo   │ (local)
└─────────────┘
```

**Setup:**
```bash
# Start dependencies
docker-compose up -d qdrant

# Configure OpenCode
cat > opencode.jsonc << 'EOF'
{
  "mcp": {
    "aeterna": {
      "type": "local",
      "command": ["aeterna-mcp", "--mode", "stdio"]
    }
  }
}
EOF
```

---

### Pattern 2: Team Server

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│  Dev 1      │  │  Dev 2      │  │  Dev 3      │
│  OpenCode   │  │  OpenCode   │  │  OpenCode   │
└──────┬──────┘  └──────┬──────┘  └──────┬──────┘
       │                │                │
       └────────────────┼────────────────┘
                        │ HTTPS
                        ▼
               ┌─────────────────┐
               │   aeterna-mcp   │
               │   (team server) │
               └────────┬────────┘
                        │
          ┌─────────────┼─────────────┐
          │             │             │
          ▼             ▼             ▼
    ┌──────────┐  ┌──────────┐  ┌──────────┐
    │  Qdrant  │  │ Git Repo │  │  Redis   │
    └──────────┘  └──────────┘  └──────────┘
```

**Setup:**
```yaml
# docker-compose.yml
services:
  aeterna-mcp:
    image: aeterna/mcp-server:latest
    ports:
      - "8081:8081"
    environment:
      - AETERNA_MCP_MODE=http
      - AETERNA_MCP_AUTH=api-key
      - AETERNA_QDRANT_URL=http://qdrant:6333
    depends_on:
      - qdrant
      - redis

  qdrant:
    image: qdrant/qdrant:latest
    volumes:
      - qdrant_data:/qdrant/storage

  redis:
    image: redis:7-alpine
    volumes:
      - redis_data:/data

volumes:
  qdrant_data:
  redis_data:
```

---

### Pattern 3: Enterprise Multi-Tenant

```
                    ┌─────────────────────────────┐
                    │       Load Balancer         │
                    │     (TLS termination)       │
                    └──────────────┬──────────────┘
                                   │
          ┌────────────────────────┼────────────────────────┐
          │                        │                        │
          ▼                        ▼                        ▼
   ┌─────────────┐          ┌─────────────┐          ┌─────────────┐
   │ aeterna-mcp │          │ aeterna-mcp │          │ aeterna-mcp │
   │  replica 1  │          │  replica 2  │          │  replica 3  │
   └──────┬──────┘          └──────┬──────┘          └──────┬──────┘
          │                        │                        │
          └────────────────────────┼────────────────────────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
                    ▼              ▼              ▼
             ┌──────────┐   ┌──────────┐   ┌──────────┐
             │  Qdrant  │   │PostgreSQL│   │  Redis   │
             │ cluster  │   │ (events) │   │ (cache)  │
             └──────────┘   └──────────┘   └──────────┘
```

**Kubernetes deployment:**

```yaml
# aeterna-mcp-deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: aeterna-mcp
spec:
  replicas: 3
  selector:
    matchLabels:
      app: aeterna-mcp
  template:
    metadata:
      labels:
        app: aeterna-mcp
    spec:
      containers:
      - name: aeterna-mcp
        image: aeterna/mcp-server:latest
        ports:
        - containerPort: 8081
        env:
        - name: AETERNA_MCP_MODE
          value: "http"
        - name: AETERNA_MCP_AUTH
          value: "jwt"
        - name: AETERNA_JWT_ISSUER
          valueFrom:
            secretKeyRef:
              name: aeterna-secrets
              key: jwt-issuer
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
        readinessProbe:
          httpGet:
            path: /health
            port: 8081
          initialDelaySeconds: 5
          periodSeconds: 10
---
apiVersion: v1
kind: Service
metadata:
  name: aeterna-mcp
spec:
  selector:
    app: aeterna-mcp
  ports:
  - port: 8081
    targetPort: 8081
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: aeterna-mcp
  annotations:
    kubernetes.io/ingress.class: nginx
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  tls:
  - hosts:
    - aeterna.tenant.com
    secretName: aeterna-tls
  rules:
  - host: aeterna.tenant.com
    http:
      paths:
      - path: /mcp
        pathType: Prefix
        backend:
          service:
            name: aeterna-mcp
            port:
              number: 8081
```

---

## Comparison: NPM Plugin vs MCP Server

### Feature Matrix

| Feature | NPM Plugin | MCP Server (Stdio) | MCP Server (HTTP) |
|---------|------------|--------------------|--------------------|
| **All 8 tools** | Yes | Yes | Yes |
| **Chat message hook** | Yes | No | No |
| **System prompt injection** | Yes | No | No |
| **Tool execution capture** | Automatic | Manual | Manual |
| **Permission hooks** | Yes | No | No |
| **Session lifecycle** | Yes | Limited | Limited |
| **Resource access** | Via backend | Yes | Yes |
| **Remote deployment** | No | No | Yes |
| **Enterprise auth** | Via backend | Process | JWT/API Key |
| **Multi-client** | No | No | Yes |
| **Load balancing** | No | No | Yes |

### Decision Matrix

| Your Situation | Recommendation |
|----------------|----------------|
| Single developer, local dev | NPM Plugin |
| Team sharing one machine | MCP Server (Stdio) |
| Remote team, centralized | MCP Server (HTTP) |
| Enterprise fleet (100+) | MCP Server (HTTP) + NPM Plugin |
| Claude Desktop / other clients | MCP Server only |
| Maximum features | NPM Plugin |
| Maximum security | MCP Server (HTTP) with JWT |

### Hybrid Approach

For enterprise deployments, combine both:

```jsonc
// opencode.jsonc - Hybrid configuration
{
  "$schema": "https://opencode.ai/config.json",
  
  // NPM Plugin for hooks and deep integration
  "plugin": ["@aeterna-org/opencode-plugin"],
  
  // Plugin configured to use remote MCP as backend
  "pluginConfig": {
    "@aeterna-org/opencode-plugin": {
      "backend": {
        "type": "mcp",
        "url": "https://aeterna.example.com/mcp",
        "auth": {
          "type": "jwt",
          "tokenEnv": "AETERNA_TOKEN"
        }
      }
    }
  }
}
```

This gives you:
- All hook functionality (chat, system prompt, session)
- Centralized backend (auth, audit, multi-tenant)
- Enterprise security (JWT, Cedar)

---

## Troubleshooting

### Connection Issues

#### Cannot Connect to MCP Server

```
Error: connection refused to aeterna-mcp
```

**Diagnosis:**
```bash
# Check if server is running (HTTP mode)
curl -v https://aeterna.example.com/mcp/health

# Check if process is running (stdio mode)
ps aux | grep aeterna-mcp

# Check logs
aeterna-mcp --mode http --log-level debug
```

**Solutions:**
1. Verify server is started: `aeterna-mcp --mode http`
2. Check port binding: `netstat -tlnp | grep 8081`
3. Verify firewall rules allow traffic
4. Check TLS certificates are valid

#### Timeout on Tool Calls

```
Error: request timeout after 30000ms
```

**Solutions:**
1. Increase timeout in client config
2. Check backend connectivity (Qdrant, Git)
3. Reduce result limits for large queries

### Authentication Issues

#### Invalid API Key

```
Error: 401 Unauthorized - Invalid API key
```

**Diagnosis:**
```bash
# Verify key format
echo $AETERNA_TOKEN | head -c 10
# Should start with: aet_k_

# Test key
curl -H "Authorization: Bearer $AETERNA_TOKEN" \
  https://aeterna.example.com/mcp/health
```

**Solutions:**
1. Regenerate API key: `aeterna-mcp keygen --name "dev"`
2. Check key hasn't expired
3. Verify environment variable is set

#### JWT Token Invalid

```
Error: 401 Unauthorized - JWT validation failed
```

**Diagnosis:**
```bash
# Decode JWT (without verification)
echo $AETERNA_TOKEN | cut -d. -f2 | base64 -d | jq

# Check expected claims
# Should have: sub, tenant_id, exp
```

**Solutions:**
1. Refresh token from identity provider
2. Verify issuer matches server config
3. Check token hasn't expired
4. Ensure required claims are present

### Tool Execution Issues

#### Tool Not Found

```
Error: Unknown tool: aeterna_memory_add
```

**Diagnosis:**
```bash
# List available tools
curl https://aeterna.example.com/mcp/tools/list | jq '.tools[].name'
```

**Solutions:**
1. Verify MCP server version is current
2. Check tool registration in server startup

#### Permission Denied

```
Error: Permission denied for KnowledgePropose
```

**Solutions:**
1. Check user role: verify JWT claims
2. Review Cedar policies
3. Contact admin for elevated permissions

### Resource Access Issues

#### Resource Not Found

```
Error: Resource not found: aeterna://knowledge/project
```

**Diagnosis:**
```bash
# List available resources
curl https://aeterna.example.com/mcp/resources/list | jq '.resources[].uri'
```

**Solutions:**
1. Verify project context is set
2. Check knowledge repository is accessible
3. Ensure user has access to requested scope

### Debug Mode

Enable verbose logging:

```bash
# Server-side
aeterna-mcp --mode http --log-level trace

# Or via environment
AETERNA_LOG_LEVEL=trace aeterna-mcp --mode http
```

**Log locations:**
- Stdio mode: stderr (visible in parent process)
- HTTP mode: stdout or configured file

### Health Check Endpoint

```bash
# Check server health
curl https://aeterna.example.com/mcp/health

# Expected response
{
  "status": "healthy",
  "version": "1.0.0",
  "backends": {
    "memory": "connected",
    "knowledge": "connected",
    "governance": "connected"
  },
  "uptime_seconds": 3600
}
```

### Metrics Endpoint

```bash
# Prometheus metrics (if enabled)
curl https://aeterna.example.com/mcp/metrics

# Key metrics
aeterna_mcp_tool_calls_total{tool="aeterna_memory_search"}
aeterna_mcp_request_duration_seconds{method="tools/call"}
aeterna_mcp_auth_failures_total{reason="invalid_token"}
```

---

## Next Steps

- [OpenCode Integration Guide](./opencode-integration) - Full NPM plugin documentation
- [Strangler Fig Example](../examples/strangler-fig-migration) - Real-world usage
- [Tool Interface Specification](../specs/tool-interface) - Detailed tool contracts
- [Helm Chart Deployment](../helm/architecture) - Kubernetes deployment
