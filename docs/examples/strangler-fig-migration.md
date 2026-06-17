# Strangler Fig Migration with Aeterna

**How 300 engineers transform a monolith to microservices using AI-assisted knowledge governance**

This guide demonstrates how Aeterna serves as the knowledge backbone for a multi-year platform transformation using the Strangler Fig pattern.

---

## The Scenario: Project Wolf

**Starting Point (Legacy "KApp"):**
- Monolithic Java application (2M LOC)
- Batch-oriented processing (1M payments/month)
- End-of-day reconciliation
- Single PostgreSQL database
- 15 years of accumulated technical debt

**Target State (Autonomous Platform):**
- 50M+ payments/month
- <30 second latency
- ~200 versioned microservices ("Bricks")
- Cellular multi-tenant architecture
- Policy-driven, agent-ready operations

**Timeline:** 3 years (2024-2027)
**Teams:** 12 teams, 300 engineers
**Challenge:** How do you coordinate this transformation without chaos?

---

## Aeterna's Role: The Transformation Knowledge Graph

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                     STRANGLER FIG TRANSFORMATION                             │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        AETERNA KNOWLEDGE LAYER                       │    │
│  │                                                                      │    │
│  │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐             │    │
│  │   │     ADRs     │  │   Policies   │  │   Patterns   │             │    │
│  │   │              │  │              │  │              │             │    │
│  │   │ • Migration  │  │ • Code rules │  │ • Brick spec │             │    │
│  │   │   decisions  │  │ • API stds   │  │ • Anti-cors  │             │    │
│  │   │ • Tech debt  │  │ • Security   │  │ • Wrappers   │             │    │
│  │   │   payoffs    │  │ • Testing    │  │ • Facades    │             │    │
│  │   └──────────────┘  └──────────────┘  └──────────────┘             │    │
│  │                              │                                      │    │
│  │                    ┌─────────▼─────────┐                           │    │
│  │                    │  CONSTRAINT DSL   │                           │    │
│  │                    │                   │                           │    │
│  │                    │ "Block if using   │                           │    │
│  │                    │  legacy patterns" │                           │    │
│  │                    └─────────┬─────────┘                           │    │
│  │                              │                                      │    │
│  └──────────────────────────────┼──────────────────────────────────────┘    │
│                                 │                                            │
│  ┌──────────────────────────────▼──────────────────────────────────────┐    │
│  │                        AETERNA MEMORY LAYER                          │    │
│  │                                                                      │    │
│  │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐             │    │
│  │   │ Team Memory  │  │ Agent Memory │  │  Migration   │             │    │
│  │   │              │  │              │  │   Learnings  │             │    │
│  │   │ • Decisions  │  │ • What works │  │              │             │    │
│  │   │ • Blockers   │  │ • Edge cases │  │ • Gotchas    │             │    │
│  │   │ • Workarounds│  │ • Tool prefs │  │ • Successes  │             │    │
│  │   └──────────────┘  └──────────────┘  └──────────────┘             │    │
│  │                                                                      │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                                                                       │   │
│  │   MONOLITH (KApp)              │           MICROSERVICES (Wolf)      │   │
│  │   ━━━━━━━━━━━━━━━━             │           ━━━━━━━━━━━━━━━━━━━━      │   │
│  │                                │                                      │   │
│  │   ┌─────────────┐   Strangler  │   ┌─────────────┐                   │   │
│  │   │  Payment    │──── Fig ─────┼──►│  Payment    │                   │   │
│  │   │  Module     │   Wrapper    │   │  Service    │                   │   │
│  │   └─────────────┘              │   └─────────────┘                   │   │
│  │                                │                                      │   │
│  │   ┌─────────────┐              │   ┌─────────────┐                   │   │
│  │   │  Liquidity  │──────────────┼──►│  Liquidity  │                   │   │
│  │   │  Module     │              │   │  Service    │                   │   │
│  │   └─────────────┘              │   └─────────────┘                   │   │
│  │                                │                                      │   │
│  │   ┌─────────────┐              │                                      │   │
│  │   │  Legacy     │  (not yet    │                                      │   │
│  │   │  Modules    │   migrated)  │                                      │   │
│  │   └─────────────┘              │                                      │   │
│  │                                │                                      │   │
│  └────────────────────────────────┴──────────────────────────────────────┘   │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## Organizational Structure in Aeterna

### Tenant Layer: Global Standards

```yaml
# knowledge/tenant/wolf-corp/policies/migration-baseline.yaml
id: migration-baseline
type: policy
layer: tenant
mode: mandatory
merge_strategy: merge

rules:
  # No new code in monolith
  - id: no-monolith-features
    target: file
    operator: must_not_match
    pattern: "src/main/java/com/legacy/kapp/.*\\.java$"
    severity: block
    message: |
      BLOCKED: No new features in legacy KApp.
      All new development must use Wolf microservices.
      See ADR-001: Strangler Fig Migration Strategy
  
  # All new services must be "Bricks"
  - id: brick-pattern-required
    target: code
    operator: must_match
    pattern: "@Brick\\(version\\s*=\\s*\"\\d+\\.\\d+\""
    severity: warn
    message: |
      All new services must follow the Brick pattern.
      See pattern://wolf/brick-specification
  
  # API versioning required
  - id: api-versioning
    target: config
    operator: must_match
    pattern: "api\\.version:\\s*v[0-9]+"
    severity: block
    message: |
      All APIs must be versioned (v1, v2, etc.)
      See ADR-015: API Versioning Strategy
```

### Organization Layer: Domain Standards

```yaml
# knowledge/org/platform-engineering/policies/payments-domain.yaml
id: payments-domain-standards
type: policy
layer: org
mode: mandatory

rules:
  # Payment services must use TigerBeetle for ledger
  - id: ledger-technology
    target: dependency
    operator: must_use
    pattern: "tigerbeetle-client"
    severity: block
    message: |
      Payment services MUST use TigerBeetle for ledger operations.
      PostgreSQL is NOT approved for financial transactions.
      See ADR-023: Ledger Technology Selection
  
  # Idempotency required
  - id: idempotency-required
    target: code
    operator: must_match
    pattern: "@Idempotent|idempotency_key"
    severity: block
    message: |
      All payment operations MUST be idempotent.
      See pattern://wolf/idempotent-operations
  
  # Double-entry accounting
  - id: double-entry
    target: code
    operator: must_match
    pattern: "debit.*credit|credit.*debit"
    severity: warn
    message: |
      Financial operations should use double-entry accounting.
      See ADR-024: Accounting Model
```

### Team Layer: Implementation Standards

```yaml
# knowledge/team/payments-core/policies/team-standards.yaml
id: payments-core-standards
type: policy
layer: team
mode: optional
merge_strategy: merge

rules:
  # Team uses Kotlin for new services
  - id: kotlin-preferred
    target: file
    operator: must_match
    pattern: ".*\\.kt$"
    severity: info
    message: |
      Payments Core team prefers Kotlin for new services.
      Java is acceptable for legacy integration.
  
  # Circuit breaker on all external calls
  - id: circuit-breaker
    target: code
    operator: must_match
    pattern: "@CircuitBreaker|resilience4j"
    severity: warn
    message: |
      External service calls should use circuit breakers.
      See pattern://payments-core/resilience-patterns
```

---

## ADR Library: Capturing Migration Decisions

### ADR-001: Strangler Fig Migration Strategy

```markdown
# knowledge/tenant/wolf-corp/adrs/adr-001-strangler-fig.md

# ADR-001: Strangler Fig Migration Strategy

## Status
Accepted (2024-01-15)

## Context
We need to transform KApp (2M LOC monolith) to Wolf (microservices) 
while maintaining 99.9% uptime and serving existing customers.

Big-bang rewrites fail. We need incremental transformation.

## Decision
We will use the **Strangler Fig Pattern**:

1. **Wrap**: New facade intercepts requests to legacy modules
2. **Replace**: Gradually implement functionality in new services
3. **Retire**: Remove legacy code when traffic fully migrated

```
     ┌─────────────────────────────────────────┐
     │            API Gateway                   │
     │                                          │
     │   ┌────────────────────────────────┐    │
     │   │      Strangler Facade          │    │
     │   │                                │    │
     │   │  if (feature_flag.new_service) │    │
     │   │    → route to Wolf service     │    │
     │   │  else                          │    │
     │   │    → route to KApp module      │    │
     │   │                                │    │
     │   └────────────────────────────────┘    │
     │              │              │            │
     └──────────────┼──────────────┼────────────┘
                    ▼              ▼
            ┌───────────┐  ┌───────────┐
            │   Wolf    │  │   KApp    │
            │  Service  │  │  Module   │
            └───────────┘  └───────────┘
```

## Consequences

### Positive
- Zero downtime migration
- Incremental risk reduction
- Team autonomy (different teams migrate different modules)
- Rollback capability per feature

### Negative
- Temporary complexity (two systems)
- Need robust feature flag infrastructure
- Requires comprehensive testing of both paths

### Constraints (Enforced by Aeterna)

```yaml
constraints:
  - operator: must_not_exist
    target: file
    pattern: "src/main/java/com/legacy/kapp/new_feature_*"
    severity: block
    message: "No new features in legacy KApp - use Wolf services"
  
  - operator: must_use
    target: dependency
    pattern: "strangler-facade-sdk"
    severity: warn
    message: "Use official SDK for strangler facades"
```

## Related
- ADR-002: Feature Flag Strategy
- ADR-003: Traffic Shadowing for Validation
- Pattern: Strangler Facade Implementation
```

### ADR-015: API Versioning Strategy

```markdown
# knowledge/tenant/wolf-corp/adrs/adr-015-api-versioning.md

# ADR-015: API Versioning Strategy

## Status
Accepted (2024-03-01)

## Context
With 200 microservices and 12 teams, we need consistent API evolution
without breaking consumers. Migration period means both KApp and Wolf
consumers exist simultaneously.

## Decision
**URL Path Versioning** with **Sunset Headers**:

```
# Version in path
POST /api/v2/payments/initiate

# Sunset header for deprecation
Sunset: Sat, 01 Jan 2025 00:00:00 GMT
Deprecation: true
Link: </api/v3/payments/initiate>; rel="successor-version"
```

### Version Lifecycle

| Stage | Duration | Policy |
|-------|----------|--------|
| **Current** | Indefinite | Full support |
| **Deprecated** | 6 months | Bug fixes only, sunset header |
| **Retired** | - | Returns 410 Gone |

### Breaking vs Non-Breaking

**Non-breaking (no version bump):**
- Adding optional fields
- Adding new endpoints
- Performance improvements

**Breaking (version bump required):**
- Removing fields
- Changing field types
- Changing endpoint behavior

## Constraints (Enforced by Aeterna)

```yaml
constraints:
  - operator: must_match
    target: code
    pattern: "@ApiVersion\\(\"v[0-9]+\"\\)"
    severity: block
    message: "All API endpoints must declare version"
  
  - operator: must_not_match
    target: code
    pattern: "@DeleteMapping.*v1.*payment"
    severity: block
    message: "Cannot remove v1 payment endpoints until 2025-06-01"
```
```

### ADR-023: Ledger Technology Selection

```markdown
# knowledge/org/platform-engineering/adrs/adr-023-ledger-tech.md

# ADR-023: Ledger Technology Selection

## Status
Accepted (2024-04-15)

## Context
Payment services require ACID guarantees for financial transactions.
Current PostgreSQL setup cannot guarantee:
- Double-entry accounting consistency
- Sub-millisecond balance checks
- 50M transactions/month throughput

## Options Considered

| Option | Throughput | Latency | Consistency | Team Expertise |
|--------|------------|---------|-------------|----------------|
| PostgreSQL | 1M/month | 50ms | ACID | High |
| CockroachDB | 10M/month | 20ms | Serializable | Medium |
| TigerBeetle | 100M/month | <1ms | Strict Serial | Low |
| Custom Ledger | Variable | Variable | Custom | None |

## Decision
**TigerBeetle** for all financial ledger operations.

### Rationale
1. Purpose-built for financial transactions
2. 1000x faster than PostgreSQL for balance operations
3. Built-in double-entry accounting primitives
4. Deterministic - same inputs always produce same outputs

### Migration Path
```
Phase 1: Shadow mode (TigerBeetle mirrors PostgreSQL)
Phase 2: Read from TigerBeetle, write to both
Phase 3: Write to TigerBeetle, async sync to PostgreSQL
Phase 4: TigerBeetle primary, PostgreSQL retired
```

## Constraints (Enforced by Aeterna)

```yaml
constraints:
  - operator: must_use
    target: dependency
    pattern: "tigerbeetle-client"
    appliesTo: ["services/payments-*", "services/ledger-*"]
    severity: block
    message: "Payment services must use TigerBeetle"
  
  - operator: must_not_use
    target: code
    pattern: "JdbcTemplate.*balance|UPDATE.*account.*balance"
    appliesTo: ["services/payments-*"]
    severity: block
    message: "Direct PostgreSQL balance updates prohibited"
```

## Related
- ADR-024: Double-Entry Accounting Model
- ADR-025: Reconciliation Strategy
- Pattern: TigerBeetle Integration
```

---

## Patterns Library: Reusable Solutions

### Pattern: Strangler Facade

```markdown
# knowledge/tenant/wolf-corp/patterns/strangler-facade.md

# Pattern: Strangler Facade

## Problem
How do you incrementally migrate traffic from a legacy module to a 
new microservice without downtime?

## Solution
Implement a facade that intercepts requests and routes them based on
feature flags, gradually shifting traffic to the new service.

```
┌─────────────────────────────────────────────────────────────────┐
│                     Strangler Facade                             │
│                                                                  │
│   Request ──►  ┌─────────────────────────────────────────┐      │
│                │         Router                          │      │
│                │                                         │      │
│                │  1. Check feature flag                  │      │
│                │  2. Check traffic percentage            │      │
│                │  3. Check client tier (Platinum/Gold)   │      │
│                │  4. Route to appropriate backend        │      │
│                │                                         │      │
│                └──────────┬──────────────┬───────────────┘      │
│                           │              │                       │
│                     ┌─────▼─────┐  ┌─────▼─────┐                │
│                     │   Wolf    │  │   KApp    │                │
│                     │  Service  │  │  Module   │                │
│                     └───────────┘  └───────────┘                │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

## Implementation

```kotlin
@Component
class PaymentStranglerFacade(
    private val featureFlags: FeatureFlagService,
    private val wolfClient: PaymentWolfClient,
    private val kappClient: PaymentKAppClient,
    private val metrics: MeterRegistry
) {
    @Brick(version = "2.1", domain = "payments")
    suspend fun initiatePayment(request: PaymentRequest): PaymentResponse {
        val routingDecision = decideRouting(request)
        
        return when (routingDecision) {
            is RouteToWolf -> {
                metrics.counter("strangler.wolf.requests").increment()
                wolfClient.initiate(request)
            }
            is RouteToKApp -> {
                metrics.counter("strangler.kapp.requests").increment()
                kappClient.initiate(request)
            }
            is ShadowBoth -> {
                // Send to both, return KApp result, compare async
                val kappResult = kappClient.initiate(request)
                launch { shadowCompare(request, kappResult) }
                kappResult
            }
        }
    }
    
    private fun decideRouting(request: PaymentRequest): RoutingDecision {
        // Platinum clients get Wolf first
        if (request.clientTier == Tier.PLATINUM && 
            featureFlags.isEnabled("wolf.payments.platinum")) {
            return RouteToWolf
        }
        
        // Percentage rollout for others
        val percentage = featureFlags.getPercentage("wolf.payments.rollout")
        if (Random.nextInt(100) < percentage) {
            return RouteToWolf
        }
        
        // Shadow mode for validation
        if (featureFlags.isEnabled("wolf.payments.shadow")) {
            return ShadowBoth
        }
        
        return RouteToKApp
    }
}
```

## Traffic Migration Schedule

| Week | Platinum | Gold | Standard | Shadow |
|------|----------|------|----------|--------|
| 1-2 | Shadow | - | - | 100% |
| 3-4 | 10% | Shadow | - | 100% |
| 5-6 | 50% | 10% | Shadow | - |
| 7-8 | 100% | 50% | 10% | - |
| 9-10 | 100% | 100% | 50% | - |
| 11-12 | 100% | 100% | 100% | - |

## Constraints

```yaml
constraints:
  - operator: must_use
    target: code
    pattern: "StranglerFacade|@Strangler"
    appliesTo: ["services/*-facade"]
    severity: block
    message: "Facade services must use Strangler pattern"
  
  - operator: must_match
    target: code
    pattern: "metrics\\.counter\\(\"strangler\\."
    severity: warn
    message: "Strangler facades should emit routing metrics"
```

## Related
- ADR-001: Strangler Fig Migration Strategy
- ADR-002: Feature Flag Strategy
- Pattern: Shadow Testing
- Pattern: Reconciler Bot
```

### Pattern: Brick Specification

```markdown
# knowledge/tenant/wolf-corp/patterns/brick-specification.md

# Pattern: Brick Specification

## Problem
How do you ensure 200+ microservices across 12 teams maintain 
consistency in contracts, versioning, and observability?

## Solution
Every service function is a **Brick**: a versioned, atomic, 
observable unit of business capability.

## Brick Anatomy

```kotlin
@Brick(
    version = "2.3",
    domain = "payments",
    capability = "initiate-payment",
    owner = "payments-core-team",
    sla = Sla(p99 = 100.ms, availability = 99.9)
)
@Idempotent(key = "request.idempotencyKey")
@CircuitBreaker(name = "payment-initiate")
@Metered
@Traced
suspend fun initiatePayment(
    @Valid request: PaymentInitiateRequest
): PaymentInitiateResponse {
    // Implementation
}
```

## Required Annotations

| Annotation | Purpose | Required |
|------------|---------|----------|
| `@Brick` | Version, domain, ownership | ✅ Yes |
| `@Idempotent` | Exactly-once semantics | ✅ For mutations |
| `@CircuitBreaker` | Fault isolation | ✅ For external calls |
| `@Metered` | Business metrics | ✅ Yes |
| `@Traced` | Distributed tracing | ✅ Yes |
| `@Cached` | Response caching | Optional |
| `@RateLimited` | Throttling | Optional |

## Version Compatibility Matrix

```
Brick Version Format: MAJOR.MINOR

MAJOR bump (breaking):
  - Request schema changes
  - Response schema changes  
  - Semantic behavior changes

MINOR bump (compatible):
  - Performance improvements
  - Bug fixes
  - New optional fields
```

## Registry Entry

Every Brick is registered in the Wolf Registry:

```yaml
# registry/payments/initiate-payment.yaml
brick:
  id: payments.initiate-payment
  version: "2.3"
  domain: payments
  owner: payments-core-team
  
  contract:
    request: PaymentInitiateRequest
    response: PaymentInitiateResponse
    errors:
      - INVALID_AMOUNT
      - INSUFFICIENT_FUNDS
      - DUPLICATE_REQUEST
  
  dependencies:
    - ledger.create-transfer@2.x
    - risk.evaluate-transaction@1.x
    - connectivity.send-to-rail@3.x
  
  sla:
    p50: 20ms
    p99: 100ms
    availability: 99.9%
  
  migration:
    strangler_facade: payment-strangler-facade
    legacy_module: com.legacy.kapp.payments.PaymentProcessor
    traffic_percentage: 75
```

## Constraints

```yaml
constraints:
  - operator: must_match
    target: code
    pattern: "@Brick\\(.*version.*domain.*\\)"
    appliesTo: ["services/*/src/**/*.kt"]
    severity: block
    message: "All service functions must be annotated as Bricks"
  
  - operator: must_match
    target: code
    pattern: "@Idempotent"
    appliesTo: ["**/*Command.kt", "**/*Mutation.kt"]
    severity: block
    message: "All mutations must be idempotent"
  
  - operator: must_match
    target: file
    pattern: "registry/.*/.*\\.yaml"
    severity: warn
    message: "Bricks should be registered in the Wolf Registry"
```
```

### Pattern: Anti-Corruption Layer

```markdown
# knowledge/tenant/wolf-corp/patterns/anti-corruption-layer.md

# Pattern: Anti-Corruption Layer

## Problem
How do you prevent legacy domain models from "corrupting" the clean
design of new microservices during migration?

## Solution
Implement a translation layer that converts between legacy and 
modern domain models, keeping them strictly separated.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Wolf Service (Clean Domain)                   │
│                                                                  │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │                  Domain Model                            │   │
│   │                                                          │   │
│   │   Payment {                                              │   │
│   │     id: PaymentId                                        │   │
│   │     amount: Money                                        │   │
│   │     status: PaymentStatus                                │   │
│   │     initiatedAt: Instant                                 │   │
│   │   }                                                      │   │
│   │                                                          │   │
│   └─────────────────────────────────────────────────────────┘   │
│                              │                                   │
│                              ▼                                   │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │              Anti-Corruption Layer                       │   │
│   │                                                          │   │
│   │   - Translates KApp → Wolf models                        │   │
│   │   - Translates Wolf → KApp models                        │   │
│   │   - Handles missing/extra fields                         │   │
│   │   - Normalizes enums and codes                           │   │
│   │                                                          │   │
│   └─────────────────────────────────────────────────────────┘   │
│                              │                                   │
└──────────────────────────────┼───────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                    KApp (Legacy Domain)                          │
│                                                                  │
│   PAYMENT_RECORD {                                               │
│     PAY_ID: VARCHAR(20)                                          │
│     AMT: DECIMAL(15,2)                                           │
│     CCY: CHAR(3)                                                 │
│     STAT_CD: CHAR(2)                                             │
│     CRE_DT: DATE                                                 │
│     CRE_TM: TIME                                                 │
│   }                                                              │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation

```kotlin
@Component
class PaymentAntiCorruptionLayer {
    
    /**
     * Translate legacy KApp payment to Wolf domain model
     */
    fun fromLegacy(record: KAppPaymentRecord): Payment {
        return Payment(
            id = PaymentId(record.payId),
            amount = Money(
                value = record.amt,
                currency = Currency.getInstance(record.ccy)
            ),
            status = translateStatus(record.statCd),
            initiatedAt = combineDateTime(record.creDt, record.creTm)
        )
    }
    
    /**
     * Translate Wolf payment to legacy KApp format
     */
    fun toLegacy(payment: Payment): KAppPaymentRecord {
        return KAppPaymentRecord(
            payId = payment.id.value.take(20),  // Truncate to legacy limit
            amt = payment.amount.value,
            ccy = payment.amount.currency.currencyCode,
            statCd = reverseTranslateStatus(payment.status),
            creDt = payment.initiatedAt.toLocalDate(),
            creTm = payment.initiatedAt.toLocalTime()
        )
    }
    
    private fun translateStatus(legacyCode: String): PaymentStatus {
        return when (legacyCode) {
            "01" -> PaymentStatus.PENDING
            "02" -> PaymentStatus.PROCESSING
            "03" -> PaymentStatus.COMPLETED
            "04" -> PaymentStatus.FAILED
            "99" -> PaymentStatus.CANCELLED
            else -> {
                logger.warn("Unknown legacy status: $legacyCode")
                PaymentStatus.UNKNOWN
            }
        }
    }
}
```

## Translation Registry

Document all translations for team reference:

| Wolf Field | KApp Field | Transformation |
|------------|------------|----------------|
| `id: PaymentId` | `PAY_ID: VARCHAR(20)` | Truncate to 20 chars |
| `amount.value` | `AMT: DECIMAL(15,2)` | Direct mapping |
| `amount.currency` | `CCY: CHAR(3)` | ISO 4217 code |
| `status` | `STAT_CD: CHAR(2)` | Enum translation |
| `initiatedAt` | `CRE_DT + CRE_TM` | Combine date+time |

## Constraints

```yaml
constraints:
  - operator: must_not_match
    target: code
    pattern: "KApp.*Record|PAYMENT_RECORD"
    appliesTo: ["services/*/domain/**"]
    severity: block
    message: |
      Legacy types must not leak into domain layer.
      Use Anti-Corruption Layer for translation.
  
  - operator: must_exist
    target: file
    pattern: "**/acl/*Translator.kt"
    appliesTo: ["services/*-service"]
    severity: warn
    message: "Services should have ACL translators"
```
```

---

## Memory Layer: Team Learnings

### How AI Agents Use Migration Knowledge

```rust
// Agent working on payments-service asks about migration
let context = TenantContext::new("wolf-corp")
    .with_org("platform-engineering")
    .with_team("payments-core")
    .with_project("payments-service");

// Search memory for migration learnings
let learnings = memory.search(SearchQuery {
    query: "strangler facade gotchas payment migration",
    layers: vec![
        MemoryLayer::Project,  // Project-specific issues
        MemoryLayer::Team,     // Team discoveries
        MemoryLayer::Org,      // Org-wide patterns
        MemoryLayer::Tenant,  // Global lessons
    ],
    context: context.clone(),
}).await?;

// Results (ordered by layer precedence):
// 1. [Project] "Payment facade race condition with KApp - use distributed lock"
// 2. [Team] "TigerBeetle connection pooling: max 10 connections per service"
// 3. [Org] "Always shadow test for 2 weeks before traffic shift"
// 4. [Tenant] "Feature flags: use LaunchDarkly SDK v7+ for Kotlin"

// Check constraints before writing code
let violations = knowledge.check_constraints(
    ConstraintContext::new()
        .with_file("src/main/kotlin/PaymentService.kt")
        .with_code(proposed_code)
        .with_dependencies(&["postgresql-client"])  // Trying to use PostgreSQL
).await?;

// Result: BLOCKED
// "Payment services must use TigerBeetle for ledger operations.
//  See ADR-023: Ledger Technology Selection"
```

### Memory Examples by Layer

```
COMPANY LAYER (Global Lessons)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
• "Strangler facades need circuit breakers on BOTH backends"
• "Shadow testing found 3% discrepancy rate is normal during migration"
• "Never migrate during quarter-end close periods"
• "LaunchDarkly flags: always set default to legacy path"

ORG LAYER (Platform Engineering)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
• "TigerBeetle: batch transfers for >100 operations"
• "Temporal workflows: use 30-day retention for audit"
• "ClickHouse: partition by week for payment analytics"
• "CockroachDB: set transaction priority for critical paths"

TEAM LAYER (Payments Core)
━━━━━━━━━━━━━━━━━━━━━━━━━━
• "Payment idempotency: hash(client_id + reference + amount)"
• "KApp STAT_CD '05' is undocumented - means 'pending_review'"
• "Reconciler bot: run at 3am UTC to avoid peak load"
• "Legacy API timeout: 30s (not 10s as documented)"

PROJECT LAYER (payments-service)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
• "CircuitBreaker: 50% threshold, 60s timeout for KApp calls"
• "PaymentInitiateRequest: amount must be positive (legacy allows 0)"
• "Shadow mode enabled for Platinum clients since 2024-06-01"
• "Deployment: blue-green with 10min bake time"
```

### Memory Promotion (Memory-R1)

When a developer discovers something valuable, agents can promote it:

```rust
// Developer finds a critical gotcha
memory.add(MemoryEntry {
    content: "KApp payment module has 20-char limit on payment ID. 
              Wolf uses UUIDs (36 chars). ACL must truncate or hash.",
    layer: MemoryLayer::Project,
    metadata: MemoryMetadata {
        tags: vec!["gotcha", "acl", "migration"],
        source: MemorySource::Conversation { message_id: "msg_123" },
        ..Default::default()
    },
}).await?;

// Later: This memory gets positive feedback from multiple team members
memory.feedback(MemoryFeedback {
    memory_id: mem_id,
    reward: 0.9,  // Very helpful
    feedback_type: FeedbackType::Positive,
}).await?;

// Memory-R1 autonomous optimization promotes it to team layer
// Now ALL payments-core team members benefit from this discovery
```

---

## Agent Collaboration Scenarios

### Scenario 1: New Developer Onboarding

```
Developer: "I need to add a new payment validation rule"

AI Agent (with Aeterna):
━━━━━━━━━━━━━━━━━━━━━━━━━

1. [Knowledge Check] Searching ADRs and policies...
   
   Found:
   • ADR-001: No new features in legacy KApp
   • Policy: All new code must be in Wolf services
   • Pattern: Brick Specification required

2. [Memory Search] Looking for similar work...
   
   Found (Team Memory):
   • "Validation rules go in payments-validator service"
   • "Use @ValidationBrick annotation"
   • "Legacy PaymentValidator.java is frozen - don't touch"

3. [Constraint Check] Validating your approach...
   
   ⚠️ WARNING: You're editing src/main/java/com/legacy/kapp/
   This is BLOCKED by tenant policy.
   
   ✅ SUGGESTED: Create new Brick in services/payments-validator/

4. [Generate Code] Based on patterns and team standards...

   @Brick(version = "1.0", domain = "payments")
   @ValidationBrick
   class AmountRangeValidator : PaymentValidator {
       override fun validate(payment: Payment): ValidationResult {
           // Implementation following team patterns
       }
   }
```

### Scenario 2: Cross-Team Dependency

```
Liquidity Team: "We need payment status for forecasting"

AI Agent (with Aeterna):
━━━━━━━━━━━━━━━━━━━━━━━━━

1. [Knowledge Check] Checking API contracts...
   
   Found (Registry):
   • payments.get-status@v2 - Available
   • payments.subscribe-status@v1 - Available (Event-driven)
   
   ADR-015: Use v2 APIs (v1 deprecated)

2. [Memory Search] Cross-team integration patterns...
   
   Found (Org Memory):
   • "Prefer event subscription over polling for status"
   • "Payment events on Kafka topic: wolf.payments.status"
   • "Use Avro schemas from schema-registry"

3. [Constraint Check] Validating integration...
   
   ✅ OK: Using approved v2 API
   ✅ OK: Event-driven pattern preferred
   ⚠️ WARNING: Liquidity services should not call payment 
      ledger directly (ADR-023)

4. [Generate Integration]
   
   @Brick(version = "1.0", domain = "liquidity")
   class PaymentStatusSubscriber(
       private val kafka: KafkaConsumer<PaymentStatusEvent>
   ) {
       @EventHandler
       suspend fun onPaymentStatus(event: PaymentStatusEvent) {
           // Update forecast based on payment status
       }
   }
```

### Scenario 3: Migration Decision

```
Tech Lead: "Should we migrate the FX module next?"

AI Agent (with Aeterna):
━━━━━━━━━━━━━━━━━━━━━━━━━

1. [Knowledge Check] Migration status and dependencies...
   
   Current State:
   • Payments: 75% migrated (Wolf primary)
   • Liquidity: 40% migrated (Shadow testing)
   • FX: 0% migrated (Legacy only)
   • Risk: 0% migrated (Legacy only)
   
   Dependency Graph:
   • FX depends on: Payments (✅), Risk (❌)
   • Risk depends on: Payments (✅), Liquidity (⚠️)

2. [Memory Search] Previous migration learnings...
   
   Found (Tenant Memory):
   • "Always migrate dependencies first"
   • "FX has complex KApp stored procedures - 3 month estimate"
   • "Risk module: Shadow testing revealed 5% discrepancy"

3. [ADR Recommendation]
   
   Based on analysis, I recommend:
   
   ❌ NOT ready to migrate FX
   
   Reasons:
   1. Risk module (dependency) not migrated
   2. Liquidity (transitive dependency) still in shadow
   3. FX stored procedures need refactoring first
   
   Suggested sequence:
   1. Complete Liquidity migration (4 weeks)
   2. Migrate Risk module (6 weeks)
   3. Then migrate FX (12 weeks)
   
   Want me to draft an ADR for the migration sequence?
```

---

## Governance Dashboard

### Policy Compliance by Team

```
┌─────────────────────────────────────────────────────────────────┐
│                 WOLF MIGRATION GOVERNANCE                        │
│                 Week 24 / 2024                                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  POLICY COMPLIANCE                                               │
│  ━━━━━━━━━━━━━━━━━                                              │
│                                                                  │
│  payments-core     ████████████████████░░░░  85%                │
│  liquidity-team    ██████████████████░░░░░░  75%                │
│  connectivity      ████████████████████████  98%                │
│  risk-team         ██████████████░░░░░░░░░░  60%                │
│  fx-team           ████████░░░░░░░░░░░░░░░░  35%  ⚠️            │
│                                                                  │
│  TOP VIOLATIONS                                                  │
│  ━━━━━━━━━━━━━━━                                                │
│                                                                  │
│  1. [BLOCK] Legacy code modifications (fx-team: 12 instances)   │
│  2. [WARN]  Missing @Brick annotations (risk-team: 8 instances) │
│  3. [INFO]  Outdated API versions (various: 23 instances)       │
│                                                                  │
│  MIGRATION PROGRESS                                              │
│  ━━━━━━━━━━━━━━━━━━                                             │
│                                                                  │
│  Payments     [██████████████████░░]  90%  → Target: 100% Q3    │
│  Liquidity    [████████████░░░░░░░░]  60%  → Target: 100% Q4    │
│  Connectivity [████████████████████]  100% ✅ Complete           │
│  Risk         [████████░░░░░░░░░░░░]  40%  → Target: 100% Q1'25 │
│  FX           [████░░░░░░░░░░░░░░░░]  20%  → Target: 100% Q2'25 │
│                                                                  │
│  KNOWLEDGE HEALTH                                                │
│  ━━━━━━━━━━━━━━━━                                               │
│                                                                  │
│  ADRs:      45 (+3 this week)                                   │
│  Policies:  28 (2 violations pending)                           │
│  Patterns:  52 (+5 this week)                                   │
│  Memories:  1,247 (89 promoted this week)                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Getting Started

### 1. Initialize Aeterna for Migration Project

```bash
# Clone and setup
git clone https://github.com/your-org/aeterna
cd aeterna
cargo build --release

# Initialize knowledge repository
aeterna init --template strangler-fig \
  --tenant "wolf-corp" \
  --orgs "platform-engineering,product,security"
```

### 2. Import Existing ADRs

```bash
# Import from existing docs
aeterna knowledge import \
  --source ./legacy-docs/adrs \
  --type adr \
  --layer tenant

# Validate constraints
aeterna knowledge validate --strict
```

### 3. Configure CI/CD Integration

```yaml
# .github/workflows/aeterna-check.yml
name: Aeterna Policy Check

on: [pull_request]

jobs:
  policy-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Check Aeterna Constraints
        uses: aeterna/policy-check-action@v1
        with:
          config: ./aeterna.toml
          fail-on: block  # block, warn, or info
          
      - name: Post Violations to PR
        if: failure()
        uses: aeterna/pr-comment-action@v1
        with:
          template: policy-violation
```

### 4. Enable AI Agent Integration

```rust
// In your AI coding assistant
let aeterna = AeternaClient::new(config)?;

// Before generating code
let context = aeterna.get_context(
    tenant,
    &["adrs", "policies", "patterns", "memories"]
).await?;

// Include in agent prompt
let prompt = format!(
    "{}\n\nRelevant context:\n{}",
    user_request,
    context.to_prompt_string()
);

// After generating code
let violations = aeterna.check_constraints(
    &generated_code,
    &file_path
).await?;

if violations.has_blocking() {
    // Regenerate with constraint awareness
}
```

---

## Summary

Aeterna enables successful strangler fig migrations by:

1. **Codifying Decisions**: ADRs capture the "why" behind migration choices
2. **Enforcing Policies**: Constraints prevent legacy patterns from spreading
3. **Sharing Patterns**: Reusable solutions accelerate team delivery
4. **Preserving Memory**: Learnings survive team changes and time
5. **Enabling Agents**: AI assistants have full migration context
6. **Tracking Progress**: Governance dashboard shows compliance trends

**The result**: 300 engineers can transform a monolith over 3 years without losing institutional knowledge, repeating mistakes, or violating architectural decisions.
