# context-architect Specification

## Purpose
TBD - created by archiving change add-cca-capabilities. Update Purpose after archive.
## Requirements
### Requirement: Hierarchical Context Compression
The system SHALL maintain pre-computed summaries at multiple depths for every layer in the hierarchy (Tenant, Org, Team, Project, Multi-Session, Session).

#### Scenario: Generate summaries at three depths
- **WHEN** layer content is created or updated
- **AND** summary trigger conditions are met (time-based OR change-based)
- **THEN** system SHALL generate summaries at sentence (~50 tokens), paragraph (~200 tokens), and detailed (~500 tokens) depths
- **AND** system SHALL store summaries with generation timestamp and source content hash

#### Scenario: Skip summary generation when unchanged
- **WHEN** summary trigger fires
- **AND** source content hash matches last summary generation
- **AND** skip_if_unchanged is enabled
- **THEN** system SHALL skip summary regeneration
- **AND** system SHALL log skip reason for observability

### Requirement: Summary Update Triggers
The system SHALL support configurable triggers for summary updates combining time-based and change-based conditions.

#### Scenario: Time-based trigger
- **WHEN** configured update_interval duration has elapsed since last summary
- **THEN** system SHALL trigger summary regeneration for that layer

#### Scenario: Change-based trigger
- **WHEN** number of changes since last summary exceeds update_on_changes threshold
- **THEN** system SHALL trigger summary regeneration for that layer

#### Scenario: Combined trigger (whichever first)
- **WHEN** either time-based OR change-based condition is met
- **THEN** system SHALL trigger summary regeneration
- **AND** system SHALL reset both counters after regeneration

### Requirement: Summary Personalization
The system SHALL support personalized summaries that incorporate user-specific context for relevant layers.

#### Scenario: Personalized summary for session layer
- **WHEN** generating summary for session layer
- **AND** personalization is enabled for that layer
- **THEN** system SHALL include user's recent concerns and preferences in summarization prompt
- **AND** system SHALL tag summary as personalized with context reference

#### Scenario: Generic summary for tenant layer
- **WHEN** generating summary for tenant layer
- **AND** personalization is disabled (default for tenant/org)
- **THEN** system SHALL generate organization-wide generic summary
- **AND** system SHALL NOT include user-specific context

### Requirement: Context Vector Generation
The system SHALL generate embedding vectors for each layer's content to enable semantic relevance matching.

#### Scenario: Generate context vector on content change
- **WHEN** layer content changes
- **THEN** system SHALL generate embedding vector using configured embedding provider
- **AND** system SHALL store vector alongside summaries for fast retrieval

#### Scenario: Use context vector for relevance scoring
- **WHEN** assembling context for a query
- **THEN** system SHALL compute cosine similarity between query embedding and layer context vectors
- **AND** system SHALL use similarity scores for relevance-based selection

### Requirement: Adaptive Context Assembly
The system SHALL assemble context from multiple layers based on relevance scores and token budget constraints.

#### Scenario: Assemble context within token budget
- **WHEN** assembling context for a query with token_budget parameter
- **THEN** system SHALL compute relevance scores for all accessible layers
- **AND** system SHALL select appropriate summary depth per layer to fit budget
- **AND** system SHALL prioritize higher-relevance layers when budget is constrained

#### Scenario: Adaptive depth selection
- **WHEN** assembling context with limited token budget
- **AND** high-relevance layer would exceed remaining budget with detailed summary
- **THEN** system SHALL fall back to paragraph or sentence summary for that layer
- **AND** system SHALL include as many relevant layers as budget allows

#### Scenario: Default budget allocation
- **WHEN** no explicit token_budget is provided
- **THEN** system SHALL use configured default_token_budget (default: 8000 tokens)
- **AND** system SHALL allocate budget proportionally to relevance scores

### Requirement: Summary Storage Strategy
The system SHALL store summaries using a hybrid strategy with cache for fast access and persistent storage for durability.

#### Scenario: Store summary in cache and persistent storage
- **WHEN** summary is generated
- **THEN** system SHALL store in Redis cache with configured TTL
- **AND** system SHALL store in PostgreSQL as source of truth
- **AND** system SHALL include metadata (depth, timestamp, hash, personalization)

#### Scenario: Retrieve summary with cache fallback
- **WHEN** retrieving summary for context assembly
- **THEN** system SHALL first check Redis cache
- **AND** system SHALL fall back to PostgreSQL if cache miss
- **AND** system SHALL repopulate cache on fallback retrieval

### Requirement: Layer Configuration
The system SHALL support per-layer configuration for summary generation behavior.

#### Scenario: Configure layer with custom settings
- **WHEN** layer configuration specifies custom update_interval and update_on_changes
- **THEN** system SHALL use those values instead of defaults
- **AND** system SHALL validate configuration on load

#### Scenario: Default layer configurations
- **WHEN** no custom configuration exists for a layer
- **THEN** system SHALL use default configurations:
- **AND** Tenant/Org: hourly OR every 10 changes, generic
- **AND** Team: hourly OR every 5 changes, personalized
- **AND** Project: every 30 min OR every 3 changes, personalized
- **AND** Session: every 5 min OR every change, highly personalized

### Requirement: AX/UX/DX View Separation
The system SHALL provide separate views of context for Agent Experience (AX), User Experience (UX), and Developer Experience (DX).

#### Scenario: Agent Experience view
- **WHEN** agent requests context
- **THEN** system SHALL return compressed summaries optimized for token efficiency
- **AND** system SHALL NOT include raw content or debug information

#### Scenario: User Experience view
- **WHEN** user requests activity or context information
- **THEN** system SHALL return rendered, human-readable format
- **AND** system SHALL include full trajectory logs if requested

#### Scenario: Developer Experience view
- **WHEN** developer requests debug information
- **THEN** system SHALL return full state dumps and traces
- **AND** system SHALL include metrics, timing, and decision explanations

### Requirement: Summary Quality Metrics
The system SHALL track and emit metrics for summary generation quality and performance.

#### Scenario: Emit generation metrics
- **WHEN** summary generation completes
- **THEN** system SHALL emit histogram: context_architect.summary.generation_time_ms
- **AND** system SHALL emit counter: context_architect.summary.generations_total with layer and depth labels

#### Scenario: Emit cache metrics
- **WHEN** summary retrieval occurs
- **THEN** system SHALL emit counter: context_architect.cache.hits
- **AND** system SHALL emit counter: context_architect.cache.misses

#### Scenario: Emit assembly metrics
- **WHEN** context assembly completes
- **THEN** system SHALL emit histogram: context_architect.assembly.tokens_used
- **AND** system SHALL emit histogram: context_architect.assembly.layers_included

### Requirement: Context Assembly Latency Control
The system SHALL ensure context assembly meets latency requirements for real-time agent interactions.

#### Scenario: Pre-computed relevance scores
- **WHEN** layer content is updated
- **THEN** system SHALL pre-compute relevance scores against common query patterns
- **AND** system SHALL store scores for fast retrieval during assembly
- **AND** system SHALL invalidate scores when content changes significantly

#### Scenario: Cached context assembly
- **WHEN** same query pattern is received within cache window (configurable, default: 5 minutes)
- **THEN** system SHALL return cached assembled context
- **AND** system SHALL invalidate cache on layer content changes
- **AND** system SHALL emit metric: context_architect.assembly.cache_hit_rate

#### Scenario: Assembly timeout fallback
- **WHEN** context assembly exceeds timeout (configurable, default: 500ms)
- **THEN** system SHALL return partial context with available layers
- **AND** system SHALL log timeout with assembly progress
- **AND** system SHALL continue assembly asynchronously for next request

### Requirement: LLM Summarization Failure Handling
The system SHALL gracefully handle LLM API failures during summarization.

#### Scenario: Retry with exponential backoff
- **WHEN** LLM API call fails
- **THEN** system SHALL retry with exponential backoff (1s, 2s, 4s, max 3 attempts)
- **AND** system SHALL log each retry attempt
- **AND** system SHALL emit metric: context_architect.llm.retries

#### Scenario: Fallback to cached summary
- **WHEN** all LLM retry attempts fail
- **THEN** system SHALL return cached summary if available
- **AND** system SHALL mark summary as stale but usable
- **AND** system SHALL alert on repeated failures

#### Scenario: Graceful degradation
- **WHEN** no cached summary available and LLM fails
- **THEN** system SHALL fall back to raw content truncation
- **AND** system SHALL log degraded operation
- **AND** system SHALL include warning in response metadata

