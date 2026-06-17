# Aeterna: Complete Sequence Diagrams

**Detailed Flow Diagrams for All Key Interactions**

---

## Table of Contents

1. [Memory Operations](#memory-operations)
2. [Knowledge Repository Operations](#knowledge-repository-operations)
3. [Sync Bridge Operations](#sync-bridge-operations)
4. [Governance & Policy Enforcement](#governance--policy-enforcement)
5. [Agent-to-Agent (A2A) Communication](#agent-to-agent-a2a-communication)
6. [Advanced Features (CCA)](#advanced-features-cca)
7. [Multi-Tenant Operations](#multi-tenant-operations)
8. [Error Handling & Recovery](#error-handling--recovery)

---

## Memory Operations

### 1.1 Memory Add with Embedding Generation

```mermaid
sequenceDiagram
    participant Client as Client/Agent
    participant API as Aeterna API
    participant MemManager as Memory Manager
    participant Validator as Input Validator
    participant Embedder as Embedding Service
    participant VectorDB as Qdrant
    participant Cache as Redis
    participant Metrics as Metrics Collector

    Client->>API: POST /api/v1/memory/add
    activate API

    API->>Validator: Validate input
    activate Validator
    Validator-->>API: ✓ Valid
    deactivate Validator

    API->>MemManager: add_memory(entry)
    activate MemManager

    MemManager->>Metrics: increment(memory_add_total)

    par Parallel Processing
        MemManager->>Embedder: generate_embedding(content)
        activate Embedder
        Embedder->>Embedder: Tokenize content
        Embedder->>Embedder: Call OpenAI API
        Embedder-->>MemManager: embedding[1536]
        deactivate Embedder
    and
        MemManager->>MemManager: Generate memory_id
        MemManager->>MemManager: Add metadata (timestamps, layer)
    end

    MemManager->>VectorDB: upsert_point(id, embedding, payload)
    activate VectorDB
    VectorDB-->>MemManager: ✓ Stored
    deactivate VectorDB

    MemManager->>Cache: set(memory_id, metadata)
    activate Cache
    Cache-->>MemManager: ✓ Cached
    deactivate Cache

    MemManager->>Metrics: observe(memory_add_duration_ms, 245)

    MemManager-->>API: MemoryEntry{id, layer}
    deactivate MemManager

    API-->>Client: 201 Created {id: "mem_abc123"}
    deactivate API
```

---

### 1.2 Multi-Layer Memory Search

```mermaid
sequenceDiagram
    participant Client as Client/Agent
    participant API as Aeterna API
    participant MemManager as Memory Manager
    participant Embedder as Embedding Service
    participant Redis as Redis Cache
    participant Qdrant as Qdrant
    participant Postgres as PostgreSQL
    participant Scorer as Relevance Scorer
    participant Dedup as Deduplicator

    Client->>API: POST /api/v1/memory/search
    activate API
    Note over Client,API: Query: "How to authenticate APIs?"<br/>Layers: [team, org, tenant]

    API->>MemManager: search(query, layers, limit)
    activate MemManager

    MemManager->>Embedder: generate_embedding(query)
    activate Embedder
    Embedder-->>MemManager: query_embedding[1536]
    deactivate Embedder

    par Search Layer: Team (Redis)
        MemManager->>Redis: search_by_embedding(team)
        activate Redis
        Redis-->>MemManager: results_team[5]
        deactivate Redis
    and Search Layer: Org (Qdrant)
        MemManager->>Qdrant: search_points(org_collection)
        activate Qdrant
        Qdrant-->>MemManager: results_org[8]
        deactivate Qdrant
    and Search Layer: Tenant (PostgreSQL metadata + Qdrant vectors)
        MemManager->>Postgres: SELECT metadata WHERE tenant=...
        activate Postgres
        Postgres-->>MemManager: results_company[3]
        deactivate Postgres
    end

    MemManager->>Scorer: rank_results(all_results)
    activate Scorer
    Scorer->>Scorer: Apply layer precedence (team > org > tenant)
    Scorer->>Scorer: Calculate relevance scores
    Scorer->>Scorer: Apply recency boost
    Scorer-->>MemManager: ranked_results[16]
    deactivate Scorer

    MemManager->>Dedup: deduplicate(ranked_results)
    activate Dedup
    Dedup->>Dedup: Compare by content hash
    Dedup->>Dedup: Keep highest-layer version
    Dedup-->>MemManager: unique_results[10]
    deactivate Dedup

    MemManager-->>API: SearchResults{results, total}
    deactivate MemManager

    API-->>Client: 200 OK {results: [...]}
    deactivate API
```

---

### 1.3 Memory Promotion (Working → Team)

```mermaid
sequenceDiagram
    participant Timer as Background Timer
    participant PromotionEngine as Promotion Engine
    participant MemManager as Memory Manager
    participant Redis as Redis (Working)
    participant Postgres as PostgreSQL (Team)
    participant Metrics as Metrics Collector
    participant Notifier as Event Notifier

    loop Every 5 minutes
        Timer->>PromotionEngine: check_promotion_candidates()
        activate PromotionEngine

        PromotionEngine->>MemManager: get_layer_memories(working)
        activate MemManager
        MemManager->>Redis: scan_with_pattern("working:*")
        activate Redis
        Redis-->>MemManager: memories[100]
        deactivate Redis
        MemManager-->>PromotionEngine: candidate_memories
        deactivate MemManager

        loop For each memory
            PromotionEngine->>PromotionEngine: calculate_promotion_score()
            Note over PromotionEngine: Score = (access_count * 0.4) +<br/>(confidence * 0.3) +<br/>(age_weight * 0.2) +<br/>(reward * 0.1)

            alt Score > threshold (0.75)
                PromotionEngine->>MemManager: promote_memory(id, target_layer=session)
                activate MemManager

                MemManager->>Redis: get(working:mem_id)
                activate Redis
                Redis-->>MemManager: memory_data
                deactivate Redis

                MemManager->>Postgres: INSERT INTO session_memories
                activate Postgres
                Postgres-->>MemManager: ✓ Inserted
                deactivate Postgres

                MemManager->>Redis: setex(session:mem_id, ttl=3600)
                activate Redis
                Redis-->>MemManager: ✓ Cached
                deactivate Redis

                MemManager->>Redis: del(working:mem_id)
                activate Redis
                Redis-->>MemManager: ✓ Deleted
                deactivate Redis

                MemManager-->>PromotionEngine: ✓ Promoted
                deactivate MemManager

                PromotionEngine->>Metrics: increment(memory_promotions_total, layer=session)

                PromotionEngine->>Notifier: emit_event(memory_promoted)
                activate Notifier
                Notifier->>Notifier: Publish to Redis pub/sub
                Notifier-->>PromotionEngine: ✓ Notified
                deactivate Notifier
            end
        end

        deactivate PromotionEngine
    end
```

---

## Knowledge Repository Operations

### 2.1 Knowledge Query with Policy Check

```mermaid
sequenceDiagram
    participant Client as Client/Agent
    participant API as Aeterna API
    participant KnowledgeRepo as Knowledge Repository
    participant GitBackend as Git Backend
    participant Parser as Document Parser
    participant Embedder as Embedding Service
    participant Qdrant as Qdrant Search
    participant Governance as Governance Engine
    participant Cache as Redis Cache

    Client->>API: POST /api/v1/knowledge/query
    activate API
    Note over Client,API: Query: "Database standards"

    API->>KnowledgeRepo: query(text, doc_types)
    activate KnowledgeRepo

    KnowledgeRepo->>Cache: get_cached_query(hash)
    activate Cache
    Cache-->>KnowledgeRepo: Cache miss
    deactivate Cache

    KnowledgeRepo->>Embedder: generate_embedding(query)
    activate Embedder
    Embedder-->>KnowledgeRepo: query_embedding[1536]
    deactivate Embedder

    KnowledgeRepo->>Qdrant: search_knowledge_index()
    activate Qdrant
    Qdrant-->>KnowledgeRepo: document_ids[10]
    deactivate Qdrant

    loop For each document_id
        KnowledgeRepo->>GitBackend: get_file(doc_id)
        activate GitBackend
        GitBackend->>GitBackend: git show HEAD:path
        GitBackend-->>KnowledgeRepo: raw_content
        deactivate GitBackend

        KnowledgeRepo->>Parser: parse(raw_content, type)
        activate Parser
        Parser->>Parser: Extract frontmatter
        Parser->>Parser: Parse markdown
        Parser-->>KnowledgeRepo: structured_doc
        deactivate Parser

        KnowledgeRepo->>Governance: check_access(user, doc_id)
        activate Governance
        Governance->>Governance: Evaluate Cedar policy
        Governance-->>KnowledgeRepo: access_granted=true
        deactivate Governance
    end

    KnowledgeRepo->>KnowledgeRepo: Rank by relevance
    KnowledgeRepo->>KnowledgeRepo: Apply access filters

    KnowledgeRepo->>Cache: set_cached_query(hash, results, ttl=300)
    activate Cache
    Cache-->>KnowledgeRepo: ✓ Cached
    deactivate Cache

    KnowledgeRepo-->>API: QueryResults{docs, total}
    deactivate KnowledgeRepo

    API-->>Client: 200 OK {results: [...]}
    deactivate API
```

---

### 2.2 Policy Addition with Approval Workflow

```mermaid
sequenceDiagram
    participant Sam as Sam (Architect)
    participant CLI as Aeterna CLI
    participant API as Aeterna API
    participant KnowledgeRepo as Knowledge Repository
    participant GitBackend as Git Backend
    participant Governance as Governance Engine
    participant Validator as Policy Validator
    participant Approver as Approval Service
    participant Notifier as Email/Slack Notifier

    Sam->>CLI: aeterna policy add --file policy.yaml
    activate CLI

    CLI->>Validator: validate_syntax(policy.yaml)
    activate Validator
    Validator->>Validator: Parse YAML
    Validator->>Validator: Check constraint syntax
    Validator->>Validator: Validate severity levels
    Validator-->>CLI: ✓ Valid
    deactivate Validator

    CLI->>API: POST /api/v1/knowledge/policy
    activate API

    API->>Governance: check_permission(sam, "policy:create")
    activate Governance
    Governance->>Governance: Query Cedar policy
    Governance-->>API: Allowed (role=architect)
    deactivate Governance

    API->>KnowledgeRepo: add_policy(policy_data)
    activate KnowledgeRepo

    KnowledgeRepo->>GitBackend: create_branch(feature/policy-xyz)
    activate GitBackend
    GitBackend->>GitBackend: git checkout -b
    GitBackend-->>KnowledgeRepo: ✓ Branch created
    deactivate GitBackend

    KnowledgeRepo->>GitBackend: commit_file(policies/new-policy.yaml)
    activate GitBackend
    GitBackend->>GitBackend: git add && git commit
    GitBackend-->>KnowledgeRepo: commit_sha
    deactivate GitBackend

    KnowledgeRepo->>Approver: create_approval_request()
    activate Approver
    Approver->>Approver: Determine required approvers
    Approver->>Approver: Create approval record
    Approver-->>KnowledgeRepo: approval_id
    deactivate Approver

    KnowledgeRepo->>Notifier: notify_approvers(approval_id)
    activate Notifier
    Notifier->>Notifier: Send emails to architects
    Notifier-->>KnowledgeRepo: ✓ Notified
    deactivate Notifier

    KnowledgeRepo-->>API: PolicyPending{id, approval_id}
    deactivate KnowledgeRepo

    API-->>CLI: 202 Accepted (pending approval)
    deactivate API

    CLI-->>Sam: ✓ Policy submitted, awaiting approval
    deactivate CLI

    Note over Sam,Notifier: Later: Approval Process

    participant Approver2 as Other Architect
    Approver2->>API: POST /api/v1/approvals/{id}/approve
    activate API

    API->>Approver: record_approval(user, approval_id)
    activate Approver
    Approver->>Approver: Check approval threshold met
    Approver-->>API: Status=approved
    deactivate Approver

    API->>GitBackend: merge_branch(feature/policy-xyz)
    activate GitBackend
    GitBackend->>GitBackend: git merge --ff
    GitBackend-->>API: ✓ Merged to main
    deactivate GitBackend

    API->>KnowledgeRepo: trigger_sync()
    activate KnowledgeRepo
    KnowledgeRepo->>KnowledgeRepo: Reindex policy
    KnowledgeRepo-->>API: ✓ Policy active
    deactivate KnowledgeRepo

    API-->>Approver2: 200 OK
    deactivate API

    API->>Notifier: notify_policy_active(sam)
    activate Notifier
    Notifier-->>Sam: 📧 Your policy is now active
    deactivate Notifier
```

---

## Sync Bridge Operations

### 3.1 Memory-to-Knowledge Sync (Bidirectional)

```mermaid
sequenceDiagram
    participant Timer as Sync Timer
    participant SyncBridge as Sync Bridge
    participant MemManager as Memory Manager
    participant KnowledgeRepo as Knowledge Repo
    participant DeltaDetector as Delta Detector
    participant ConflictResolver as Conflict Resolver
    participant GitBackend as Git Backend
    participant EventBus as Redis Event Bus

    loop Every 60 seconds
        Timer->>SyncBridge: trigger_sync()
        activate SyncBridge

        Note over SyncBridge: Phase 1: Memory → Knowledge

        SyncBridge->>MemManager: get_modified_since(last_sync)
        activate MemManager
        MemManager-->>SyncBridge: modified_memories[12]
        deactivate MemManager

        SyncBridge->>DeltaDetector: detect_deltas(memories)
        activate DeltaDetector

        loop For each memory
            DeltaDetector->>DeltaDetector: Calculate content hash
            DeltaDetector->>DeltaDetector: Compare with last known state

            alt Content changed
                DeltaDetector->>DeltaDetector: Mark as MODIFIED
            else New memory
                DeltaDetector->>DeltaDetector: Mark as ADDED
            else Memory deleted
                DeltaDetector->>DeltaDetector: Mark as REMOVED
            end
        end

        DeltaDetector-->>SyncBridge: deltas[7]
        deactivate DeltaDetector

        SyncBridge->>KnowledgeRepo: apply_deltas(deltas)
        activate KnowledgeRepo

        loop For each delta
            alt Delta type: ADDED
                KnowledgeRepo->>GitBackend: create_file(path, content)
                activate GitBackend
                GitBackend-->>KnowledgeRepo: ✓ Created
                deactivate GitBackend
            else Delta type: MODIFIED
                KnowledgeRepo->>ConflictResolver: check_conflict(file)
                activate ConflictResolver

                alt Conflict detected
                    ConflictResolver->>ConflictResolver: Apply merge strategy (last-write-wins)
                    ConflictResolver-->>KnowledgeRepo: resolved_content
                else No conflict
                    ConflictResolver-->>KnowledgeRepo: proceed
                end
                deactivate ConflictResolver

                KnowledgeRepo->>GitBackend: update_file(path, content)
                activate GitBackend
                GitBackend-->>KnowledgeRepo: ✓ Updated
                deactivate GitBackend
            else Delta type: REMOVED
                KnowledgeRepo->>GitBackend: delete_file(path)
                activate GitBackend
                GitBackend-->>KnowledgeRepo: ✓ Deleted
                deactivate GitBackend
            end
        end

        KnowledgeRepo->>GitBackend: commit("Sync: M→K batch")
        activate GitBackend
        GitBackend-->>KnowledgeRepo: commit_sha
        deactivate GitBackend

        KnowledgeRepo-->>SyncBridge: sync_complete(7 deltas)
        deactivate KnowledgeRepo

        Note over SyncBridge: Phase 2: Knowledge → Memory

        SyncBridge->>KnowledgeRepo: get_commits_since(last_sync)
        activate KnowledgeRepo
        KnowledgeRepo->>GitBackend: git log --since
        activate GitBackend
        GitBackend-->>KnowledgeRepo: commits[3]
        deactivate GitBackend
        KnowledgeRepo-->>SyncBridge: changed_files[5]
        deactivate KnowledgeRepo

        SyncBridge->>MemManager: import_knowledge_updates(files)
        activate MemManager

        loop For each file
            MemManager->>MemManager: Parse document
            MemManager->>MemManager: Extract key facts
            MemManager->>MemManager: Store as procedural memory
        end

        MemManager-->>SyncBridge: imported[5]
        deactivate MemManager

        SyncBridge->>EventBus: publish(sync_complete_event)
        activate EventBus
        EventBus-->>SyncBridge: ✓ Published
        deactivate EventBus

        SyncBridge->>SyncBridge: Update last_sync_time

        deactivate SyncBridge
    end
```

---

## Governance & Policy Enforcement

### 4.1 Real-Time Policy Validation

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant API as Aeterna API
    participant KnowledgeRepo as Knowledge Repo
    participant GovernanceEngine as Governance Engine
    participant PolicyEngine as Cedar Policy Engine
    participant ConstraintEvaluator as Constraint Evaluator
    participant AuditLogger as Audit Logger
    participant Notifier as Notifier

    Agent->>API: POST /api/v1/knowledge/check
    activate API
    Note over Agent,API: Content: "Use MongoDB for user data"

    API->>KnowledgeRepo: validate_content(content)
    activate KnowledgeRepo

    KnowledgeRepo->>GovernanceEngine: check_policies(content, context)
    activate GovernanceEngine

    GovernanceEngine->>GovernanceEngine: Extract technology mentions
    Note over GovernanceEngine: Detected: "MongoDB"

    GovernanceEngine->>PolicyEngine: query_policies(technology="mongodb")
    activate PolicyEngine

    PolicyEngine->>PolicyEngine: Load relevant policies
    Note over PolicyEngine: Found: db-standards policy

    PolicyEngine-->>GovernanceEngine: applicable_policies[1]
    deactivate PolicyEngine

    loop For each policy
        GovernanceEngine->>ConstraintEvaluator: evaluate(constraint, context)
        activate ConstraintEvaluator

        Note over ConstraintEvaluator: Constraint: MUST_USE postgresql FOR persistence

        ConstraintEvaluator->>ConstraintEvaluator: Check if MongoDB in approved list
        ConstraintEvaluator->>ConstraintEvaluator: Result: ❌ VIOLATION

        ConstraintEvaluator-->>GovernanceEngine: violation{policy, severity, message}
        deactivate ConstraintEvaluator
    end

    GovernanceEngine->>AuditLogger: log_violation(user, policy, content)
    activate AuditLogger
    AuditLogger->>AuditLogger: Record to audit trail
    AuditLogger-->>GovernanceEngine: ✓ Logged
    deactivate AuditLogger

    alt Severity: BLOCKING
        GovernanceEngine->>Notifier: send_alert(violation)
        activate Notifier
        Notifier->>Notifier: Notify team lead
        Notifier-->>GovernanceEngine: ✓ Notified
        deactivate Notifier
    end

    GovernanceEngine-->>KnowledgeRepo: ValidationResult{valid=false, violations}
    deactivate GovernanceEngine

    KnowledgeRepo-->>API: ValidationResult
    deactivate KnowledgeRepo

    API-->>Agent: 200 OK {valid: false, violations: [...]}
    deactivate API

    Note over Agent: Agent presents violation to user<br/>with alternative suggestions
```

---

### 4.2 Drift Detection Workflow

```mermaid
sequenceDiagram
    participant Timer as Scheduled Job
    participant DriftDetector as Drift Detector
    participant CodebaseScanner as Codebase Scanner
    participant KnowledgeRepo as Knowledge Repo
    participant Analyzer as Drift Analyzer
    participant Reporter as Report Generator
    participant Notifier as Notifier
    participant Dashboard as Drift Dashboard

    loop Every 6 hours
        Timer->>DriftDetector: run_drift_detection()
        activate DriftDetector

        DriftDetector->>KnowledgeRepo: get_active_policies()
        activate KnowledgeRepo
        KnowledgeRepo-->>DriftDetector: policies[23]
        deactivate KnowledgeRepo

        par Scan Codebase
            DriftDetector->>CodebaseScanner: scan_codebase(patterns)
            activate CodebaseScanner

            CodebaseScanner->>CodebaseScanner: Clone repositories
            CodebaseScanner->>CodebaseScanner: Parse source files
            CodebaseScanner->>CodebaseScanner: Extract technology usage
            Note over CodebaseScanner: Found: PostgreSQL: 45 files<br/>MongoDB: 3 files<br/>MySQL: 1 file

            CodebaseScanner-->>DriftDetector: usage_report
            deactivate CodebaseScanner
        and Load Policy Expectations
            DriftDetector->>KnowledgeRepo: get_policy_constraints()
            activate KnowledgeRepo
            KnowledgeRepo-->>DriftDetector: constraints[15]
            deactivate KnowledgeRepo
        end

        DriftDetector->>Analyzer: analyze_drift(usage, constraints)
        activate Analyzer

        loop For each technology
            Analyzer->>Analyzer: Compare actual vs expected

            alt Violation found
                Analyzer->>Analyzer: Identify violating files
                Analyzer->>Analyzer: Calculate drift score

                Note over Analyzer: MongoDB found in:<br/>- services/legacy-api/db.js<br/>- services/temp-service/store.js<br/>Policy: MUST_USE postgresql

                Analyzer->>Analyzer: Check if exception exists

                alt No exception
                    Analyzer->>Analyzer: Mark as VIOLATION
                else Exception approved
                    Analyzer->>Analyzer: Mark as EXCEPTION (track)
                end
            end
        end

        Analyzer-->>DriftDetector: drift_report{violations, exceptions}
        deactivate Analyzer

        DriftDetector->>Reporter: generate_report(drift_report)
        activate Reporter

        Reporter->>Reporter: Format HTML report
        Reporter->>Reporter: Calculate compliance score
        Reporter->>Reporter: Identify trends

        Reporter-->>DriftDetector: formatted_report
        deactivate Reporter

        alt Violations found
            DriftDetector->>Notifier: send_drift_alert(violations)
            activate Notifier

            Notifier->>Notifier: Group by team
            Notifier->>Notifier: Prioritize by severity

            loop For each team
                Notifier->>Notifier: Send email to team lead
                Notifier->>Notifier: Post to Slack channel
            end

            Notifier-->>DriftDetector: ✓ Notifications sent
            deactivate Notifier
        end

        DriftDetector->>Dashboard: update_dashboard(report)
        activate Dashboard
        Dashboard->>Dashboard: Update compliance metrics
        Dashboard->>Dashboard: Refresh violation trends
        Dashboard-->>DriftDetector: ✓ Updated
        deactivate Dashboard

        deactivate DriftDetector
    end
```

---

## Agent-to-Agent (A2A) Communication

### 5.1 A2A Memory Sharing Protocol

```mermaid
sequenceDiagram
    participant AgentA as Agent A (Frontend)
    participant A2AClient as A2A Client
    participant Gateway as A2A Gateway
    participant AuthMiddleware as Auth Middleware
    participant RateLimiter as Rate Limiter
    participant SkillRouter as Skill Router
    participant AgentB as Agent B (Backend)
    participant Memory as Memory System
    participant ThreadStore as Thread Store

    AgentA->>A2AClient: query_agent(skill="memory:search", query="JWT errors")
    activate A2AClient

    A2AClient->>A2AClient: Prepare JSONRPC 2.0 request
    Note over A2AClient: {<br/>  "jsonrpc": "2.0",<br/>  "method": "skill.invoke",<br/>  "params": {...}<br/>}

    A2AClient->>Gateway: POST /a2a/query
    activate Gateway

    Gateway->>AuthMiddleware: authenticate(request)
    activate AuthMiddleware

    AuthMiddleware->>AuthMiddleware: Verify API key
    AuthMiddleware->>AuthMiddleware: Validate JWT token

    AuthMiddleware-->>Gateway: Principal{agent_id, permissions}
    deactivate AuthMiddleware

    Gateway->>RateLimiter: check_rate_limit(agent_id)
    activate RateLimiter

    RateLimiter->>RateLimiter: Check token bucket
    Note over RateLimiter: Limit: 100 requests/minute

    RateLimiter-->>Gateway: ✓ Allowed
    deactivate RateLimiter

    Gateway->>SkillRouter: route_request(skill, params)
    activate SkillRouter

    SkillRouter->>SkillRouter: Parse skill identifier
    Note over SkillRouter: Skill: memory:search<br/>Target: agents with memory skill

    SkillRouter->>SkillRouter: Discover agents with skill
    Note over SkillRouter: Found: [AgentB, AgentC]

    SkillRouter->>SkillRouter: Select best agent (load, relevance)

    SkillRouter->>ThreadStore: get_or_create_thread(agent_a, agent_b)
    activate ThreadStore
    ThreadStore-->>SkillRouter: thread_id="thread_xyz"
    deactivate ThreadStore

    SkillRouter->>AgentB: invoke_skill(memory:search, params, thread_id)
    deactivate SkillRouter
    activate AgentB

    AgentB->>Memory: search(query="JWT errors", context={...})
    activate Memory

    Memory->>Memory: Generate embedding
    Memory->>Memory: Search across layers
    Memory->>Memory: Rank results

    Memory-->>AgentB: results[5]
    deactivate Memory

    AgentB->>AgentB: Format response
    Note over AgentB: Found solution:<br/>JWT key rotation yesterday<br/>Refresh from /.well-known/jwks.json

    AgentB->>ThreadStore: append_message(thread_id, response)
    activate ThreadStore
    ThreadStore-->>AgentB: ✓ Stored
    deactivate ThreadStore

    AgentB-->>Gateway: JSONRPC Response
    deactivate AgentB

    Gateway-->>A2AClient: 200 OK {result: {...}}
    deactivate Gateway

    A2AClient-->>AgentA: QueryResult{solution, confidence, source}
    deactivate A2AClient

    Note over AgentA: Agent A uses solution<br/>to help its user
```

---

### 5.2 Multi-Agent Collaboration Flow

```mermaid
sequenceDiagram
    participant User as Customer
    participant IntakeAgent as Intake Agent
    participant A2AGateway as A2A Gateway
    participant BillingAgent as Billing Agent
    participant TechAgent as Technical Agent
    participant Memory as Shared Memory
    participant Orchestrator as Orchestrator

    User->>IntakeAgent: "Payment failed during upgrade"
    activate IntakeAgent

    IntakeAgent->>IntakeAgent: Classify issue: payment

    IntakeAgent->>A2AGateway: query(skill="billing:diagnose_payment")
    activate A2AGateway

    A2AGateway->>BillingAgent: invoke_skill(diagnose_payment, context)
    activate BillingAgent

    BillingAgent->>Memory: search("payment upgrade failures")
    activate Memory
    Memory-->>BillingAgent: Common cause: expired cards
    deactivate Memory

    BillingAgent-->>A2AGateway: Diagnosis: Check card expiration
    deactivate BillingAgent

    A2AGateway-->>IntakeAgent: Result
    deactivate A2AGateway

    IntakeAgent-->>User: "Is your card expired?"
    User->>IntakeAgent: "No, I updated it. Still failing."

    IntakeAgent->>A2AGateway: query(skill="technical:payment_gateway")
    activate A2AGateway

    A2AGateway->>TechAgent: invoke_skill(diagnose_gateway, context)
    activate TechAgent

    TechAgent->>Memory: search("payment gateway failures recent")
    activate Memory
    Memory-->>TechAgent: Found incident from yesterday:<br/>Gateway timeout issue
    deactivate Memory

    TechAgent->>TechAgent: Confirm gateway status
    TechAgent->>TechAgent: Check workaround availability

    TechAgent-->>A2AGateway: Known issue + manual workaround
    deactivate TechAgent

    A2AGateway-->>IntakeAgent: Result
    deactivate A2AGateway

    IntakeAgent->>Orchestrator: request_action(manual_upgrade)
    activate Orchestrator

    Orchestrator->>A2AGateway: query(skill="billing:manual_upgrade_approval")
    activate A2AGateway

    A2AGateway->>BillingAgent: invoke_skill(approve_manual_upgrade)
    activate BillingAgent

    BillingAgent->>BillingAgent: Verify user eligibility
    BillingAgent->>BillingAgent: Check authorization

    BillingAgent-->>A2AGateway: Approved
    deactivate BillingAgent

    A2AGateway-->>Orchestrator: Approval granted
    deactivate A2AGateway

    Orchestrator->>Orchestrator: Execute manual upgrade
    Orchestrator-->>IntakeAgent: ✓ Upgrade complete
    deactivate Orchestrator

    IntakeAgent-->>User: "Your upgrade is complete!"

    IntakeAgent->>Memory: store_resolution(interaction)
    activate Memory
    Note over Memory: Store: payment_gateway_timeout<br/>Resolution: manual_upgrade<br/>Collaboration: 3 agents
    Memory-->>IntakeAgent: ✓ Stored for future
    deactivate Memory

    deactivate IntakeAgent
```

---

## Advanced Features (CCA)

### 6.1 Context Architect: Hierarchical Compression

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant ContextArchitect as Context Architect
    participant Memory as Memory System
    participant Knowledge as Knowledge Repo
    participant Compressor as Content Compressor
    participant Assembler as Context Assembler
    participant TokenCounter as Token Counter

    Agent->>ContextArchitect: assemble_context(task, token_budget=4096)
    activate ContextArchitect

    ContextArchitect->>Memory: retrieve_relevant(task)
    activate Memory

    par Retrieve from layers
        Memory->>Memory: Search working layer
        Memory->>Memory: Search session layer
        Memory->>Memory: Search team layer
        Memory->>Memory: Search org layer
    end

    Memory-->>ContextArchitect: memories[50], total_tokens=12000
    deactivate Memory

    ContextArchitect->>Knowledge: retrieve_relevant(task)
    activate Knowledge
    Knowledge-->>ContextArchitect: docs[8], total_tokens=8000
    deactivate Knowledge

    Note over ContextArchitect: Total: 20,000 tokens<br/>Budget: 4,096 tokens<br/>Compression needed!

    ContextArchitect->>Compressor: compress_hierarchically(content, budget)
    activate Compressor

    loop For each content item
        Compressor->>Compressor: Calculate relevance score

        Compressor->>Compressor: Generate multi-level summaries
        Note over Compressor: Level 1: 1 sentence (50 tokens)<br/>Level 2: 1 paragraph (150 tokens)<br/>Level 3: Detailed (500 tokens)

        Compressor->>Compressor: Store with priority tier
    end

    Compressor->>Assembler: assemble(summaries, budget)
    activate Assembler

    Assembler->>Assembler: Sort by relevance + layer precedence

    loop Build context
        Assembler->>TokenCounter: count_tokens(summary)
        activate TokenCounter
        TokenCounter-->>Assembler: token_count
        deactivate TokenCounter

        alt Tokens remaining
            alt High relevance (>0.9)
                Assembler->>Assembler: Add Level 3 (detailed)
            else Medium relevance (0.7-0.9)
                Assembler->>Assembler: Add Level 2 (paragraph)
            else Low relevance (<0.7)
                Assembler->>Assembler: Add Level 1 (sentence)
            end
        else Budget exhausted
            Assembler->>Assembler: Skip remaining
        end
    end

    Assembler-->>Compressor: assembled_context, used_tokens=4050
    deactivate Assembler

    Compressor-->>ContextArchitect: compressed_context
    deactivate Compressor

    ContextArchitect->>ContextArchitect: Validate coherence
    ContextArchitect->>ContextArchitect: Add cross-references

    ContextArchitect-->>Agent: OptimizedContext{content, metadata}
    deactivate ContextArchitect

    Note over Agent: Context assembled:<br/>4,050 tokens (99% of budget)<br/>15 memories (compressed from 50)<br/>6 docs (compressed from 8)<br/>Relevance-optimized!
```

---

### 6.2 Hindsight Learning: Error Pattern Capture

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant ErrorDetector as Error Detector
    participant HindsightLearner as Hindsight Learner
    participant Analyzer as Pattern Analyzer
    participant Memory as Memory System
    participant KnowledgeRepo as Knowledge Repo
    participant Recommender as Recommendation Engine

    Agent->>Agent: Execute task
    Agent->>Agent: ❌ Error occurred

    Agent->>ErrorDetector: report_error(error, context)
    activate ErrorDetector

    ErrorDetector->>ErrorDetector: Extract error details
    Note over ErrorDetector: Error: NullPointerException<br/>Context: user authentication flow<br/>Stack trace: [...]

    ErrorDetector->>HindsightLearner: analyze_error(error)
    deactivate ErrorDetector
    activate HindsightLearner

    HindsightLearner->>Analyzer: find_similar_errors(error)
    activate Analyzer

    Analyzer->>Memory: search("NullPointerException authentication")
    activate Memory
    Memory-->>Analyzer: similar_errors[3]
    deactivate Memory

    loop For each similar error
        Analyzer->>Analyzer: Compare stack traces
        Analyzer->>Analyzer: Identify common patterns

        Note over Analyzer: Pattern identified:<br/>Missing null check after JWT decode<br/>Occurred 3 times in last month
    end

    Analyzer-->>HindsightLearner: patterns[1]
    deactivate Analyzer

    HindsightLearner->>HindsightLearner: Generate resolution steps
    Note over HindsightLearner: Resolution:<br/>1. Add null check after JWT decode<br/>2. Return 401 if decode fails<br/>3. Add test case for invalid JWT

    HindsightLearner->>Memory: store_error_pattern(pattern, resolution)
    activate Memory
    Memory->>Memory: Store in procedural layer
    Memory->>Memory: Link to similar errors
    Memory-->>HindsightLearner: ✓ Stored
    deactivate Memory

    HindsightLearner->>Recommender: should_create_pattern?(frequency=3, impact=high)
    activate Recommender

    alt Frequency > threshold
        Recommender->>KnowledgeRepo: propose_pattern(error, resolution)
        activate KnowledgeRepo

        KnowledgeRepo->>KnowledgeRepo: Create pattern document
        Note over KnowledgeRepo: Pattern: JWT Null Handling<br/>When: Decoding JWT tokens<br/>Solution: Always null-check result<br/>Example: [code snippet]

        KnowledgeRepo->>KnowledgeRepo: Submit for approval
        KnowledgeRepo-->>Recommender: pattern_proposed
        deactivate KnowledgeRepo
    end

    Recommender-->>HindsightLearner: ✓ Pattern created
    deactivate Recommender

    HindsightLearner-->>Agent: LearningResult{resolution, pattern_id}
    deactivate HindsightLearner

    Agent->>Agent: Apply resolution
    Agent->>Agent: ✓ Error fixed

    Agent->>Memory: reward(pattern_id, success=true)
    activate Memory
    Memory->>Memory: Increase confidence score
    Memory->>Memory: Consider promotion to team layer
    Memory-->>Agent: ✓ Learning reinforced
    deactivate Memory
```

---

## Multi-Tenant Operations

### 7.1 Tenant Isolation & RBAC

```mermaid
sequenceDiagram
    participant User as User (Alex)
    participant API as Aeterna API
    participant AuthService as Auth Service
    participant TenantResolver as Tenant Resolver
    participant CedarPDP as Cedar PDP
    participant MemManager as Memory Manager
    participant DataIsolation as Data Isolation Layer
    participant Postgres as PostgreSQL

    User->>API: GET /api/v1/memory/search (with JWT)
    activate API

    API->>AuthService: validate_token(jwt)
    activate AuthService
    AuthService->>AuthService: Verify signature
    AuthService->>AuthService: Check expiration
    AuthService-->>API: Principal{user_id, tenant_id, org_id, team_id, roles}
    deactivate AuthService

    API->>TenantResolver: resolve_tenant_hierarchy(principal)
    activate TenantResolver

    TenantResolver->>TenantResolver: Build tenant path
    Note over TenantResolver: tenant: acme-corp<br/>org: engineering<br/>team: api-team<br/>user: alex@acme.com

    TenantResolver-->>API: TenantContext{hierarchy, scopes}
    deactivate TenantResolver

    API->>CedarPDP: authorize(principal, action="memory:read", resource)
    activate CedarPDP

    CedarPDP->>CedarPDP: Load applicable policies
    Note over CedarPDP: Policy: team-member-read<br/>Condition: user in team.members

    CedarPDP->>CedarPDP: Evaluate policies

    CedarPDP-->>API: Decision=ALLOW, constraints=[layers:team,user,session]
    deactivate CedarPDP

    API->>MemManager: search(query, tenant_context, allowed_layers)
    activate MemManager

    MemManager->>DataIsolation: apply_tenant_filter(tenant_context)
    activate DataIsolation

    DataIsolation->>DataIsolation: Build WHERE clause
    Note over DataIsolation: WHERE tenant_path LIKE 'acme-corp.engineering.api-team%'<br/>AND layer IN ('team', 'user', 'session')

    DataIsolation-->>MemManager: filtered_query
    deactivate DataIsolation

    MemManager->>Postgres: execute(filtered_query)
    activate Postgres

    Postgres->>Postgres: Apply row-level security
    Postgres->>Postgres: Execute with tenant filter

    Postgres-->>MemManager: results[10]
    deactivate Postgres

    MemManager->>MemManager: Verify no cross-tenant leakage

    loop For each result
        MemManager->>MemManager: Check tenant_path matches

        alt Tenant mismatch
            MemManager->>MemManager: ⚠️ Security violation detected!
            MemManager->>MemManager: Filter out result
        end
    end

    MemManager-->>API: safe_results[9]
    deactivate MemManager

    API-->>User: 200 OK {results: [...]}
    deactivate API
```

---

## Error Handling & Recovery

### 8.1 Graceful Degradation Flow

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant API as Aeterna API
    participant MemManager as Memory Manager
    participant Qdrant as Qdrant (Primary)
    participant Postgres as PostgreSQL (Fallback)
    participant Redis as Redis (Cache)
    participant CircuitBreaker as Circuit Breaker
    participant Fallback as Fallback Logic
    participant Metrics as Metrics

    Agent->>API: POST /api/v1/memory/search
    activate API

    API->>MemManager: search(query, layers=[semantic])
    activate MemManager

    MemManager->>CircuitBreaker: check_state(qdrant)
    activate CircuitBreaker

    alt Circuit CLOSED (healthy)
        CircuitBreaker-->>MemManager: ✓ Proceed
        deactivate CircuitBreaker

        MemManager->>Qdrant: search_vectors(query)
        activate Qdrant

        alt Qdrant timeout
            Qdrant-->>MemManager: ❌ Timeout (5s)
            deactivate Qdrant

            MemManager->>CircuitBreaker: record_failure(qdrant)
            activate CircuitBreaker
            CircuitBreaker->>CircuitBreaker: Increment failure count: 3/5

            alt Failure threshold reached
                CircuitBreaker->>CircuitBreaker: Open circuit
                Note over CircuitBreaker: Circuit OPEN<br/>Fast-fail for 30s
            end

            CircuitBreaker-->>MemManager: Circuit state updated
            deactivate CircuitBreaker

            MemManager->>Metrics: increment(qdrant_failures)

            MemManager->>Fallback: execute_fallback(query)
            activate Fallback

            Fallback->>Redis: try_cache_lookup(query)
            activate Redis

            alt Cache hit
                Redis-->>Fallback: cached_results[5]
                deactivate Redis
                Fallback-->>MemManager: results (from cache)
            else Cache miss
                Redis-->>Fallback: Cache miss
                deactivate Redis

                Fallback->>Postgres: search_with_keywords(query)
                activate Postgres
                Postgres-->>Fallback: results[8]
                deactivate Postgres

                Fallback->>Fallback: Mark as degraded quality
                Fallback-->>MemManager: results (degraded)
            end
            deactivate Fallback

            MemManager->>MemManager: Add degradation warning

        else Qdrant success
            Qdrant-->>MemManager: results[10]
            deactivate Qdrant

            MemManager->>CircuitBreaker: record_success(qdrant)
            activate CircuitBreaker
            CircuitBreaker->>CircuitBreaker: Reset failure count
            CircuitBreaker-->>MemManager: ✓
            deactivate CircuitBreaker
        end

    else Circuit OPEN (unhealthy)
        CircuitBreaker-->>MemManager: ❌ Circuit open, fast-fail
        deactivate CircuitBreaker

        Note over MemManager: Skip Qdrant, go directly to fallback

        MemManager->>Fallback: execute_fallback(query)
        activate Fallback
        Fallback->>Postgres: search_with_keywords(query)
        activate Postgres
        Postgres-->>Fallback: results[8]
        deactivate Postgres
        Fallback-->>MemManager: results (degraded)
        deactivate Fallback

    else Circuit HALF_OPEN (testing)
        CircuitBreaker-->>MemManager: ⚠️ Test request
        deactivate CircuitBreaker

        MemManager->>Qdrant: search_vectors(query) [test]
        activate Qdrant

        alt Success
            Qdrant-->>MemManager: results[10]
            deactivate Qdrant

            MemManager->>CircuitBreaker: record_success(qdrant)
            activate CircuitBreaker
            CircuitBreaker->>CircuitBreaker: Close circuit (recovered)
            Note over CircuitBreaker: Circuit CLOSED<br/>Service recovered!
            CircuitBreaker-->>MemManager: ✓ Circuit closed
            deactivate CircuitBreaker

        else Failure
            Qdrant-->>MemManager: ❌ Still failing
            deactivate Qdrant

            MemManager->>CircuitBreaker: record_failure(qdrant)
            activate CircuitBreaker
            CircuitBreaker->>CircuitBreaker: Re-open circuit
            CircuitBreaker-->>MemManager: Circuit re-opened
            deactivate CircuitBreaker

            MemManager->>Fallback: execute_fallback(query)
            activate Fallback
            Fallback->>Postgres: search_with_keywords(query)
            activate Postgres
            Postgres-->>Fallback: results[8]
            deactivate Postgres
            Fallback-->>MemManager: results (degraded)
            deactivate Fallback
        end
    end

    MemManager-->>API: SearchResults{results, quality_indicator}
    deactivate MemManager

    API-->>Agent: 200 OK (with degradation warning if applicable)
    deactivate API

    Note over Agent: Agent receives results<br/>even when Qdrant is down<br/>(Graceful degradation!)
```

---

## Summary

This document provides **complete sequence diagrams** for all major Aeterna workflows:

1. **Memory Operations** - Add, search, promote with detailed timing
2. **Knowledge Repository** - Query, policy management, approval workflows
3. **Sync Bridge** - Bidirectional synchronization with conflict resolution
4. **Governance** - Real-time validation, drift detection
5. **A2A Communication** - Agent-to-agent protocol with skill routing
6. **CCA Advanced** - Context compression, hindsight learning
7. **Multi-Tenant** - Isolation, RBAC, row-level security
8. **Error Handling** - Circuit breakers, fallback, graceful degradation

These diagrams demonstrate:

- **System resilience** - Fallbacks and graceful degradation
- **Security** - Multi-layer authorization and tenant isolation
- **Performance** - Caching, parallel processing, token optimization
- **Collaboration** - A2A protocol for multi-agent systems
- **Learning** - Error pattern capture and memory promotion

Each flow includes timing, error paths, and real-world scenarios.
