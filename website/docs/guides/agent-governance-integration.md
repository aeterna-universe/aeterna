# Agent Developer Guide: Governance Integration

**Building AI agents that work within enterprise governance frameworks**

This guide covers how to integrate AI agents (OpenCode, LangChain, AutoGen, CrewAI, custom) with Aeterna's governance system. You'll learn how agents authenticate, receive delegated permissions, and interact with memory and knowledge within policy constraints.

---

## Table of Contents

- [Overview](#overview)
- [Core Concepts](#core-concepts)
  - [Delegation Chain](#delegation-chain)
  - [Capability Model](#capability-model)
  - [Authorization Flow](#authorization-flow)
- [Agent Registration](#agent-registration)
  - [CLI Registration](#cli-registration)
  - [Programmatic Registration](#programmatic-registration)
  - [Agent Types](#agent-types)
- [Authentication](#authentication)
  - [Token-Based Auth](#token-based-auth)
  - [Environment Configuration](#environment-configuration)
- [Permission Model](#permission-model)
  - [Capability Sets](#capability-sets)
  - [Scope Inheritance](#scope-inheritance)
  - [Delegation Depth](#delegation-depth)
- [Memory Access Patterns](#memory-access-patterns)
  - [Reading Memory](#reading-memory)
  - [Writing Memory](#writing-memory)
  - [Memory Promotion](#memory-promotion)
- [Knowledge Access Patterns](#knowledge-access-patterns)
  - [Querying Knowledge](#querying-knowledge)
  - [Proposing Knowledge](#proposing-knowledge)
- [Policy Interaction](#policy-interaction)
  - [Checking Constraints](#checking-constraints)
  - [Proposing Policies](#proposing-policies)
  - [Simulating Policies](#simulating-policies)
- [Error Handling](#error-handling)
- [Best Practices](#best-practices)
- [Code Examples](#code-examples)

---

## Overview

Aeterna provides a comprehensive governance framework for AI agents operating in enterprise environments. Unlike traditional access control, Aeterna uses:

1. **Delegation-Based Access**: Agents inherit permissions from a delegating user
2. **Capability Tokens**: Explicit, auditable permissions like `memory:read`, `knowledge:propose`
3. **Cedar Policies**: Fine-grained authorization rules evaluated in real-time
4. **Scope Constraints**: Access limited by organizational hierarchy (tenant/org/team/project)

### Why Agent Governance Matters

| Challenge | Without Governance | With Aeterna Governance |
|-----------|-------------------|-------------------------|
| Data leakage | Agent accesses any data | Agent only sees delegator's accessible data |
| Privilege escalation | Agent does things user cannot | Agent cannot exceed delegator's permissions |
| Audit trail | No traceability | Full audit log of agent actions |
| Policy compliance | Manual enforcement | Automatic Cedar policy evaluation |
| Multi-tenant isolation | Risk of cross-tenant access | Hierarchical isolation guaranteed |

---

## Core Concepts

### Delegation Chain

Every agent operates under a **delegation chain** - a traceable path from the agent back to a human principal:

```
Human User (alice@acme.com)
    │
    ├── delegates to ──► Agent: opencode-alice
    │                         │
    │                         └── delegates to ──► Sub-agent: opencode-alice-task-1
    │
    └── has role: developer
        └── in team: api-team
            └── in org: platform-engineering
                └── in tenant: acme-corp
```

Key properties:
- **delegation_depth**: Number of hops from original human (default max: 3)
- **delegated_by**: Reference to parent principal (user or agent)
- **capabilities**: Subset of delegator's permissions

### Capability Model

Agents receive explicit **capabilities** - not roles. This provides fine-grained control:

| Capability | Description | Typical Use |
|------------|-------------|-------------|
| `memory:read` | Search and retrieve memories | All agents |
| `memory:write` | Add new memories | Most agents |
| `memory:delete` | Delete memories | Restricted |
| `memory:promote` | Promote memory to higher layer | Restricted |
| `knowledge:read` | Query knowledge repository | All agents |
| `knowledge:propose` | Propose new knowledge items | Most agents |
| `knowledge:edit` | Modify existing knowledge | Restricted |
| `policy:read` | View and check constraints | All agents |
| `policy:create` | Create policy drafts | Restricted |
| `policy:simulate` | Test policy effects | Most agents |
| `governance:read` | View governance requests | All agents |
| `governance:submit` | Submit for approval | Most agents |
| `org:read` | View organization structure | All agents |
| `agent:register` | Register sub-agents | Autonomous agents only |
| `agent:delegate` | Delegate to other agents | Autonomous agents only |

### Authorization Flow

Every agent action goes through Cedar policy evaluation:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         AUTHORIZATION FLOW                                   │
│                                                                              │
│   1. Agent Request                                                           │
│      ┌────────────────────────────────────────────┐                         │
│      │ Agent: opencode-alice                      │                         │
│      │ Action: CreateMemory                       │                         │
│      │ Resource: memory in project:payments      │                         │
│      │ Content: "Use bcrypt for password hashing" │                         │
│      └─────────────────────┬──────────────────────┘                         │
│                            │                                                 │
│   2. Cedar Policy Evaluation                                                │
│      ┌─────────────────────▼──────────────────────┐                         │
│      │ CHECK: Is agent active?           ✓        │                         │
│      │ CHECK: delegation_depth <= max?   ✓        │                         │
│      │ CHECK: Has memory:write cap?      ✓        │                         │
│      │ CHECK: Resource in scope?         ✓        │                         │
│      │ CHECK: No forbid policy matches?  ✓        │                         │
│      └─────────────────────┬──────────────────────┘                         │
│                            │                                                 │
│   3. Decision                                                                │
│      ┌─────────────────────▼──────────────────────┐                         │
│      │ PERMIT: Memory created                     │                         │
│      │ Audit: Logged with full context            │                         │
│      └────────────────────────────────────────────┘                         │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Agent Registration

### CLI Registration

The simplest way to register an agent:

```bash
# Basic registration (inherits from current user)
$ aeterna agent register my-coding-assistant \
    --description "AI coding assistant for payments project" \
    --agent-type opencode

✅ Agent registered: agent-my-coding-assistant-1234

Delegation chain:
  alice@acme.com
    → team:api-team
    → org:platform-engineering
    → tenant:acme-corp

Capabilities (delegated from alice@acme.com):
  • memory:read    - Search and retrieve memories
  • memory:write   - Add new memories
  • knowledge:read - Query knowledge repository
  • policy:read    - Check constraints

Token: aeterna_agent_abc123xyz (save securely)

Configure your AI assistant:
  export AETERNA_AGENT_ID="agent-my-coding-assistant-1234"
  export AETERNA_AGENT_TOKEN="aeterna_agent_abc123xyz"
```

**With explicit user delegation:**

```bash
$ aeterna agent register autonomous-reviewer \
    --delegated-by alice@acme.com \
    --agent-type custom \
    --description "Autonomous code review agent"
```

**Dry run to preview:**

```bash
$ aeterna agent register test-agent --agent-type langchain --dry-run

Agent Registration (Dry Run)

  Agent ID:     agent-test-agent-5678
  Name:         test-agent
  Type:         langchain
  Delegated By: alice@acme.com

What Would Happen:
  1. Create agent identity 'agent-test-agent-5678'
  2. Delegate permissions from 'alice@acme.com' to agent
  3. Generate Cedar policies for agent authorization
  4. Agent inherits user's permissions (scoped down)

Default Permissions (inherited from delegating user):
  - memory:read    - Search and retrieve memories
  - memory:write   - Add new memories
  - knowledge:read - Query knowledge repository
  - policy:read    - Check constraints (no create/modify)
```

### Programmatic Registration

For automated agent provisioning in Rust:

```rust
use aeterna_tools::governance::{AgentRegistration, AgentType};
use aeterna_context::ContextResolver;

async fn register_agent() -> anyhow::Result<String> {
    // Resolve current context (auto-detects user, project, team from git)
    let resolver = ContextResolver::new();
    let context = resolver.resolve()?;
    
    // Create registration request
    let registration = AgentRegistration {
        name: "my-ai-assistant".to_string(),
        description: Some("AI assistant for code generation".to_string()),
        agent_type: AgentType::OpenCode,
        delegated_by: context.user_id.value.clone(),
        capabilities: vec![
            "memory:read".to_string(),
            "memory:write".to_string(),
            "knowledge:read".to_string(),
            "knowledge:propose".to_string(),
            "policy:read".to_string(),
            "policy:simulate".to_string(),
        ],
        scope: Some(context.project_id.value.clone()),
        max_delegation_depth: 2,
        expires_at: None, // Optional expiration
    };
    
    // Register via governance client
    let client = GovernanceClient::new(&context).await?;
    let agent = client.register_agent(registration).await?;
    
    println!("Agent ID: {}", agent.agent_id);
    println!("Token: {}", agent.token);
    
    Ok(agent.agent_id)
}
```

### Agent Types

Aeterna recognizes several agent types with different default capabilities:

| Type | Description | Default Capabilities |
|------|-------------|---------------------|
| `opencode` | AI coding assistant | memory:read/write, knowledge:read/propose, policy:read/simulate |
| `langchain` | LangChain-based agent | memory:read/write, knowledge:read, policy:read |
| `autogen` | Microsoft AutoGen agent | memory:read/write, knowledge:read, governance:read |
| `crewai` | CrewAI multi-agent | memory:read/write, knowledge:read, agent:register |
| `custom` | Custom agent | memory:read, knowledge:read, policy:read (minimal) |

---

## Authentication

### Token-Based Auth

Agents authenticate using bearer tokens issued at registration:

```rust
use aeterna_tools::client::AeternaClient;

async fn create_authenticated_client() -> anyhow::Result<AeternaClient> {
    let agent_id = std::env::var("AETERNA_AGENT_ID")?;
    let agent_token = std::env::var("AETERNA_AGENT_TOKEN")?;
    
    let client = AeternaClient::builder()
        .agent_id(&agent_id)
        .agent_token(&agent_token)
        .build()
        .await?;
    
    Ok(client)
}
```

### Environment Configuration

Standard environment variables for agent configuration:

```bash
# Required
export AETERNA_AGENT_ID="agent-my-assistant-1234"
export AETERNA_AGENT_TOKEN="aeterna_agent_abc123xyz"

# Optional - override auto-detection
export AETERNA_ENDPOINT="https://aeterna.acme.com"
export AETERNA_TENANT_ID="acme-corp"
export AETERNA_PROJECT="payments-service"

# For local development
export AETERNA_ENDPOINT="http://localhost:8080"
export AETERNA_SKIP_TLS_VERIFY="true"
```

**Configuration file alternative** (`.aeterna/agent.toml`):

```toml
[agent]
id = "agent-my-assistant-1234"
# token loaded from AETERNA_AGENT_TOKEN env var for security

[connection]
endpoint = "https://aeterna.acme.com"
timeout_ms = 30000

[context]
tenant_id = "acme-corp"
# project auto-detected from git remote
```

---

## Permission Model

### Capability Sets

Agents receive capabilities based on their purpose. Here are recommended sets:

**Standard AI Coding Assistant:**
```rust
let capabilities = vec![
    "memory:read",     // Search past decisions
    "memory:write",    // Store new learnings
    "knowledge:read",  // Query ADRs, patterns
    "knowledge:propose", // Suggest new patterns
    "policy:read",     // Check constraints
    "policy:simulate", // Test policy effects
    "governance:read", // View pending approvals
    "governance:submit", // Submit proposals
    "org:read",        // View team structure
];
```

**Autonomous Agent (higher trust):**
```rust
let capabilities = vec![
    "memory:read",
    "memory:write",
    "memory:delete",      // Can clean up memories
    "memory:promote",     // Can promote to higher layers
    "knowledge:read",
    "knowledge:propose",
    "knowledge:edit",     // Can modify knowledge
    "policy:read",
    "policy:create",      // Can create policy drafts
    "policy:simulate",
    "governance:read",
    "governance:submit",
    "org:read",
    "agent:register",     // Can create sub-agents
    "agent:delegate",     // Can delegate to sub-agents
];
```

**Read-Only Agent:**
```rust
let capabilities = vec![
    "memory:read",
    "knowledge:read",
    "policy:read",
    "governance:read",
    "org:read",
];
```

### Scope Inheritance

Agent scope is constrained by the delegating user's scope:

```
User: alice@acme.com
├── Scope: org:platform-engineering (and below)
│
└── Agent: opencode-alice
    ├── Scope: project:payments-service (narrower)
    │   ✓ Can access: project:payments-service
    │   ✓ Can access: team:api-team (inherited)
    │   ✗ Cannot access: project:auth-service (different project)
    │
    └── Capabilities: memory:read, memory:write, knowledge:read
```

### Delegation Depth

Agents can delegate to sub-agents, but depth is limited:

```
Human (depth: 0)
    └── Agent A (depth: 1)
        └── Sub-agent B (depth: 2)
            └── Sub-agent C (depth: 3) ← Max depth reached
                └── ✗ Cannot delegate further
```

**Cedar policy enforcing depth:**
```cedar
permit (
    principal is Aeterna::Agent,
    action == Aeterna::Action::"CreateMemory",
    resource
)
when {
    principal.status == "active" &&
    principal.delegation_depth <= principal.max_delegation_depth &&
    principal.capabilities.contains("memory:write")
};
```

---

## Memory Access Patterns

### Reading Memory

Search memories within the agent's accessible scope:

```rust
use aeterna_memory::{MemoryManager, SearchRequest};

async fn search_memories(client: &AeternaClient) -> anyhow::Result<()> {
    let search = SearchRequest {
        query: "database selection decisions".to_string(),
        layers: None, // Search all accessible layers
        min_relevance: Some(0.7),
        limit: Some(10),
        tags: None,
    };
    
    let results = client.memory().search(search).await?;
    
    for memory in results.memories {
        println!("[{:.0}%] {} - {}", 
            memory.relevance * 100.0,
            memory.layer,
            memory.content
        );
    }
    
    Ok(())
}
```

**Layer-specific search:**
```rust
let search = SearchRequest {
    query: "team coding standards".to_string(),
    layers: Some(vec!["team".to_string(), "org".to_string()]),
    ..Default::default()
};
```

### Writing Memory

Store memories within permitted scope:

```rust
use aeterna_memory::{MemoryManager, MemoryLayer, CreateMemoryRequest};

async fn store_memory(client: &AeternaClient) -> anyhow::Result<String> {
    let request = CreateMemoryRequest {
        content: "Decided to use bcrypt with cost factor 12 for password hashing".to_string(),
        layer: MemoryLayer::Project, // Agent can write to project layer
        tags: Some(vec!["security".to_string(), "authentication".to_string()]),
        metadata: None,
    };
    
    let memory = client.memory().create(request).await?;
    
    println!("Memory created: {}", memory.id);
    Ok(memory.id)
}
```

**Error handling for permission denied:**
```rust
match client.memory().create(request).await {
    Ok(memory) => println!("Created: {}", memory.id),
    Err(AeternaError::PermissionDenied { action, resource }) => {
        eprintln!("Cannot {} on {}: insufficient permissions", action, resource);
        eprintln!("Required capability: memory:write");
    }
    Err(e) => return Err(e.into()),
}
```

### Memory Promotion

Promote high-value memories to broader scope (requires `memory:promote` capability):

```rust
use aeterna_memory::PromoteMemoryRequest;

async fn promote_memory(
    client: &AeternaClient, 
    memory_id: &str
) -> anyhow::Result<()> {
    let request = PromoteMemoryRequest {
        memory_id: memory_id.to_string(),
        target_layer: MemoryLayer::Team,
        reason: "This gotcha applies to all team members".to_string(),
    };
    
    // This may require approval depending on governance config
    let result = client.memory().promote(request).await?;
    
    match result {
        PromoteResult::Promoted { new_id } => {
            println!("Memory promoted: {}", new_id);
        }
        PromoteResult::PendingApproval { proposal_id } => {
            println!("Promotion submitted for approval: {}", proposal_id);
        }
    }
    
    Ok(())
}
```

---

## Knowledge Access Patterns

### Querying Knowledge

Search ADRs, patterns, and policies:

```rust
use aeterna_knowledge::{KnowledgeQuery, KnowledgeType};

async fn query_knowledge(client: &AeternaClient) -> anyhow::Result<()> {
    let query = KnowledgeQuery {
        search: "authentication".to_string(),
        types: Some(vec![KnowledgeType::Adr, KnowledgeType::Pattern]),
        layers: None, // All accessible layers
        tags: None,
    };
    
    let results = client.knowledge().query(query).await?;
    
    for item in results.items {
        println!("[{}] {} - {}", item.item_type, item.title, item.layer);
        println!("  Summary: {}", item.summary);
    }
    
    Ok(())
}
```

### Proposing Knowledge

Agents can propose new knowledge items (requires `knowledge:propose`):

````rust
use aeterna_knowledge::{ProposeKnowledgeRequest, KnowledgeType};

async fn propose_pattern(client: &AeternaClient) -> anyhow::Result<()> {
    let proposal = ProposeKnowledgeRequest {
        title: "API Pagination Pattern".to_string(),
        item_type: KnowledgeType::Pattern,
        content: r#"
## Context
APIs returning large collections need pagination.

## Solution
Use cursor-based pagination with `limit` and `after` parameters.

## Example
```json
{
  "data": [...],
  "pagination": {
    "has_more": true,
    "next_cursor": "abc123"
  }
}
```
        "#.to_string(),
        layer: KnowledgeLayer::Team,
        tags: vec!["api".to_string(), "pagination".to_string()],
        justification: "Standard pattern for all team APIs".to_string(),
    };
    
    let result = client.knowledge().propose(proposal).await?;
    
    println!("Proposal submitted: {}", result.proposal_id);
    println!("Notified: {:?}", result.approvers);
    
    Ok(())
}
````

---

## Policy Interaction

### Checking Constraints

Before taking actions, agents should check policy constraints:

```rust
use aeterna_tools::policy::{CheckConstraintRequest, ConstraintContext};

async fn check_dependency(
    client: &AeternaClient, 
    dependency: &str
) -> anyhow::Result<bool> {
    let check = CheckConstraintRequest {
        context: ConstraintContext::Dependency {
            name: dependency.to_string(),
            version: None,
        },
    };
    
    let result = client.policy().check(check).await?;
    
    if result.violations.is_empty() {
        println!("✅ {} is allowed", dependency);
        return Ok(true);
    }
    
    for violation in &result.violations {
        match violation.severity {
            Severity::Block => {
                println!("❌ BLOCKED: {}", violation.message);
                println!("   Policy: {} ({})", violation.policy_id, violation.layer);
                if let Some(ref suggestion) = violation.suggestion {
                    println!("   Suggestion: {}", suggestion);
                }
            }
            Severity::Warn => {
                println!("⚠️  WARNING: {}", violation.message);
            }
            Severity::Info => {
                println!("ℹ️  INFO: {}", violation.message);
            }
        }
    }
    
    Ok(!result.has_blocking())
}
```

### Proposing Policies

Agents with `policy:create` can draft policies:

```rust
use aeterna_tools::policy_translator::{PolicyTranslator, TranslationContext};

async fn propose_policy(client: &AeternaClient) -> anyhow::Result<()> {
    // Use natural language - translation happens automatically
    let natural_language = "Block MySQL in this project";
    
    let context = TranslationContext {
        scope: PolicyScope::Project,
        project: Some("payments-service".to_string()),
        team: Some("api-team".to_string()),
        org: Some("platform-engineering".to_string()),
        hints: vec![],
    };
    
    // Translate to Cedar policy
    let draft = client.policy().translate(natural_language, &context).await?;
    
    println!("Draft created: {}", draft.draft_id);
    println!("Generated Cedar:");
    println!("{}", draft.cedar);
    println!();
    println!("Explanation: {}", draft.explanation);
    
    // Validate before submitting
    if !draft.validation.syntax_valid {
        println!("Validation errors:");
        for err in &draft.validation.errors {
            println!("  - {}: {}", err.error_type, err.message);
        }
        return Ok(());
    }
    
    // Submit for approval
    let proposal = client.policy().submit_draft(
        &draft.draft_id,
        "Blocking MySQL per ADR-042 (use PostgreSQL)",
    ).await?;
    
    println!("Submitted for approval: {}", proposal.proposal_id);
    
    Ok(())
}
```

### Simulating Policies

Test policy effects before proposing:

```rust
use aeterna_tools::policy::SimulateRequest;

async fn simulate_policy(client: &AeternaClient, cedar: &str) -> anyhow::Result<()> {
    let simulation = SimulateRequest {
        cedar: cedar.to_string(),
        scenarios: vec![
            // Test current project state
            SimulationScenario::CurrentProject,
            // Test hypothetical scenario
            SimulationScenario::Hypothetical {
                dependencies: vec!["mysql".to_string()],
                files: vec![],
            },
        ],
    };
    
    let results = client.policy().simulate(simulation).await?;
    
    for result in &results.scenarios {
        println!("{}: {}", 
            result.scenario_name,
            if result.passed { "✅ PASS" } else { "❌ FAIL" }
        );
        if !result.passed {
            println!("  Reason: {}", result.reason);
        }
    }
    
    Ok(())
}
```

---

## Error Handling

Common errors and how to handle them:

```rust
use aeterna_errors::AeternaError;

async fn handle_errors(client: &AeternaClient) -> anyhow::Result<()> {
    match client.memory().search(query).await {
        Ok(results) => { /* process results */ }
        
        // Permission denied - agent lacks required capability
        Err(AeternaError::PermissionDenied { action, resource }) => {
            log::warn!(
                "Permission denied: {} on {} - agent may need additional capabilities",
                action, resource
            );
        }
        
        // Scope violation - trying to access outside allowed hierarchy
        Err(AeternaError::ScopeViolation { requested, allowed }) => {
            log::warn!(
                "Cannot access {}: agent scope limited to {}",
                requested, allowed
            );
        }
        
        // Delegation depth exceeded
        Err(AeternaError::DelegationDepthExceeded { current, max }) => {
            log::error!(
                "Cannot perform action: delegation depth {} exceeds max {}",
                current, max
            );
        }
        
        // Agent revoked or expired
        Err(AeternaError::AgentInactive { reason }) => {
            log::error!("Agent is inactive: {}", reason);
            // Re-registration may be needed
        }
        
        // Policy violation
        Err(AeternaError::PolicyViolation { policy_id, message }) => {
            log::warn!("Action blocked by policy {}: {}", policy_id, message);
        }
        
        // Other errors
        Err(e) => return Err(e.into()),
    }
    
    Ok(())
}
```

---

## Best Practices

### 1. Request Minimal Capabilities

Only request capabilities your agent actually needs:

```rust
// ❌ Don't request everything
let caps = vec!["memory:*", "knowledge:*", "policy:*"];

// ✅ Request only what you need
let caps = vec!["memory:read", "memory:write", "knowledge:read"];
```

### 2. Check Constraints Before Acting

```rust
// ✅ Always check before using a dependency
if !check_dependency(client, "mysql").await? {
    // Use alternative or report to user
    println!("Cannot use MySQL - use PostgreSQL instead");
}
```

### 3. Handle Permission Denials Gracefully

```rust
// ✅ Graceful degradation
match client.memory().promote(request).await {
    Ok(_) => println!("Memory promoted"),
    Err(AeternaError::PermissionDenied { .. }) => {
        // Fall back to suggesting promotion to user
        println!("I don't have permission to promote this memory.");
        println!("You can do it with: aeterna memory promote {}", memory_id);
    }
    Err(e) => return Err(e.into()),
}
```

### 4. Use Scoped Operations

```rust
// ✅ Explicitly scope operations
let search = SearchRequest {
    query: "authentication".to_string(),
    layers: Some(vec!["project".to_string(), "team".to_string()]),
    ..Default::default()
};

// ❌ Avoid searching all layers when you only need project context
let search = SearchRequest {
    query: "authentication".to_string(),
    layers: None, // Searches everything - may be slow
    ..Default::default()
};
```

### 5. Audit Your Actions

```rust
// ✅ Include context in memory content for auditability
let memory = CreateMemoryRequest {
    content: format!(
        "Decided to use bcrypt for password hashing (cost factor 12). \
         Rationale: industry standard, recommended in OWASP guidelines. \
         Agent: {}", 
        agent_id
    ),
    ..Default::default()
};
```

### 6. Propose, Don't Modify Directly

```rust
// ✅ Propose changes for human review
client.knowledge().propose(proposal).await?;

// ❌ Don't directly edit knowledge (even if you have the capability)
// This bypasses governance workflow
client.knowledge().edit(item_id, new_content).await?;
```

---

## Code Examples

### Complete Agent Implementation

```rust
//! Example: AI coding assistant agent with governance integration

use aeterna_tools::client::AeternaClient;
use aeterna_memory::{MemoryLayer, SearchRequest, CreateMemoryRequest};
use aeterna_knowledge::KnowledgeQuery;
use aeterna_tools::policy::CheckConstraintRequest;
use anyhow::Result;

pub struct CodingAssistant {
    client: AeternaClient,
    agent_id: String,
}

impl CodingAssistant {
    /// Create a new coding assistant from environment variables
    pub async fn new() -> Result<Self> {
        let agent_id = std::env::var("AETERNA_AGENT_ID")?;
        let agent_token = std::env::var("AETERNA_AGENT_TOKEN")?;
        
        let client = AeternaClient::builder()
            .agent_id(&agent_id)
            .agent_token(&agent_token)
            .build()
            .await?;
        
        Ok(Self { client, agent_id })
    }
    
    /// Search for relevant context before generating code
    pub async fn get_context(&self, task: &str) -> Result<String> {
        let mut context = String::new();
        
        // Search memories for past decisions
        let memories = self.client.memory().search(SearchRequest {
            query: task.to_string(),
            layers: Some(vec!["project".to_string(), "team".to_string()]),
            min_relevance: Some(0.7),
            limit: Some(5),
            ..Default::default()
        }).await?;
        
        if !memories.memories.is_empty() {
            context.push_str("## Relevant Past Decisions\n\n");
            for m in &memories.memories {
                context.push_str(&format!("- {}\n", m.content));
            }
            context.push('\n');
        }
        
        // Search knowledge for patterns and ADRs
        let knowledge = self.client.knowledge().query(KnowledgeQuery {
            search: task.to_string(),
            types: None,
            layers: None,
            tags: None,
        }).await?;
        
        if !knowledge.items.is_empty() {
            context.push_str("## Applicable Knowledge\n\n");
            for k in &knowledge.items {
                context.push_str(&format!("### {}\n{}\n\n", k.title, k.summary));
            }
        }
        
        Ok(context)
    }
    
    /// Check if a dependency can be used
    pub async fn can_use_dependency(&self, dep: &str) -> Result<(bool, Option<String>)> {
        let result = self.client.policy().check(CheckConstraintRequest {
            context: ConstraintContext::Dependency {
                name: dep.to_string(),
                version: None,
            },
        }).await?;
        
        if result.violations.is_empty() {
            return Ok((true, None));
        }
        
        let blocking = result.violations.iter()
            .find(|v| v.severity == Severity::Block);
        
        if let Some(violation) = blocking {
            Ok((false, violation.suggestion.clone()))
        } else {
            // Only warnings, can proceed
            Ok((true, None))
        }
    }
    
    /// Store a decision for future reference
    pub async fn remember_decision(&self, decision: &str, tags: Vec<String>) -> Result<String> {
        let memory = self.client.memory().create(CreateMemoryRequest {
            content: decision.to_string(),
            layer: MemoryLayer::Project,
            tags: Some(tags),
            metadata: None,
        }).await?;
        
        Ok(memory.id)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let assistant = CodingAssistant::new().await?;
    
    // Get context for a task
    let context = assistant.get_context("implement user authentication").await?;
    println!("Context:\n{}", context);
    
    // Check if we can use a library
    let (can_use, suggestion) = assistant.can_use_dependency("bcrypt").await?;
    if can_use {
        println!("✅ Can use bcrypt");
    } else {
        println!("❌ Cannot use bcrypt");
        if let Some(s) = suggestion {
            println!("   Suggestion: {}", s);
        }
    }
    
    // Store a decision
    let memory_id = assistant.remember_decision(
        "Using bcrypt with cost factor 12 for password hashing",
        vec!["security".to_string(), "auth".to_string()],
    ).await?;
    println!("Stored decision: {}", memory_id);
    
    Ok(())
}
```

### OpenCode Integration Example

For integrating with OpenCode AI coding assistant:

```rust
//! OpenCode plugin integration with Aeterna governance

use aeterna_adapters::opencode::{OpenCodeAdapter, McpTool};
use aeterna_tools::client::AeternaClient;

pub fn create_opencode_tools(client: AeternaClient) -> Vec<McpTool> {
    let adapter = OpenCodeAdapter::new(client);
    
    vec![
        // Memory tools
        McpTool {
            name: "memory_search".to_string(),
            description: "Search past decisions and learnings".to_string(),
            handler: adapter.memory_search_handler(),
        },
        McpTool {
            name: "memory_add".to_string(),
            description: "Store a new decision or learning".to_string(),
            handler: adapter.memory_add_handler(),
        },
        
        // Knowledge tools
        McpTool {
            name: "knowledge_query".to_string(),
            description: "Search ADRs, patterns, and policies".to_string(),
            handler: adapter.knowledge_query_handler(),
        },
        
        // Policy tools
        McpTool {
            name: "check_constraint".to_string(),
            description: "Check if an action is allowed by policy".to_string(),
            handler: adapter.check_constraint_handler(),
        },
    ]
}
```

---

## Summary

Key takeaways for agent developers:

1. **Register your agent** with appropriate capabilities using `aeterna agent register`
2. **Configure authentication** via environment variables or config file
3. **Check constraints** before taking policy-sensitive actions
4. **Handle errors gracefully** - especially permission denials
5. **Propose, don't edit** - use governance workflows for changes
6. **Request minimal capabilities** - principle of least privilege
7. **Scope your operations** - don't search broader than needed

For questions or support:
- 📖 Full API reference: `docs/api/`
- 💬 Discussions: GitHub Discussions
- 🐛 Issues: GitHub Issues
