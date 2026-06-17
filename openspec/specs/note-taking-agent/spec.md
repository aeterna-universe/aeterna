# note-taking-agent Specification

## Purpose
TBD - created by archiving change add-cca-capabilities. Update Purpose after archive.
## Requirements
### Requirement: Trajectory Capture
The system SHALL capture tool execution trajectories during agent sessions for later distillation into notes.

#### Scenario: Capture tool execution sequence
- **WHEN** agent executes a sequence of tools during a session
- **THEN** system SHALL record each tool call with input, output, timestamp, and duration
- **AND** system SHALL maintain ordering and parent-child relationships
- **AND** system SHALL associate trajectory with session and user identifiers

#### Scenario: Capture trajectory metadata
- **WHEN** capturing tool execution
- **THEN** system SHALL include tool name, arguments, return value, and error (if any)
- **AND** system SHALL include context snapshot at time of execution
- **AND** system SHALL compute and store trajectory hash for deduplication

### Requirement: Trajectory Distillation
The system SHALL distill captured trajectories into structured Markdown notes using LLM analysis.

#### Scenario: Distill successful trajectory
- **WHEN** session ends with successful outcome
- **AND** trajectory contains 3+ tool executions
- **THEN** system SHALL invoke LLM to analyze trajectory
- **AND** system SHALL extract: problem context, solution approach, key patterns
- **AND** system SHALL generate structured Markdown note

#### Scenario: Distillation prompt structure
- **WHEN** distilling trajectory to note
- **THEN** system SHALL use structured prompt requesting:
- **AND** ## Context: What problem was being solved?
- **AND** ## Solution: What approach worked?
- **AND** ## Key Patterns: What reusable patterns emerged?
- **AND** ## Code Examples: Relevant code snippets (if applicable)
- **AND** ## Tags: Suggested categorization tags

#### Scenario: Skip distillation for trivial trajectories
- **WHEN** trajectory contains fewer than 3 tool executions
- **OR** trajectory duration is less than 30 seconds
- **THEN** system SHALL skip distillation
- **AND** system SHALL log skip reason

### Requirement: Note Storage
The system SHALL store distilled notes as knowledge items in the knowledge repository.

#### Scenario: Store note as pattern knowledge item
- **WHEN** distillation produces a valid note
- **THEN** system SHALL create knowledge item with type='pattern'
- **AND** system SHALL set layer based on trajectory scope (session → project default)
- **AND** system SHALL set status='draft' for review
- **AND** system SHALL include source trajectory reference in metadata

#### Scenario: Note metadata requirements
- **WHEN** storing note
- **THEN** system SHALL include: source_trajectory_id, distillation_timestamp, llm_model_used
- **AND** system SHALL include: session_id, user_id, project_id
- **AND** system SHALL generate semantic embedding for note content

### Requirement: Note Retrieval
The system SHALL provide semantic search over distilled notes for context enrichment.

#### Scenario: Search notes by semantic similarity
- **WHEN** searching notes with query string
- **THEN** system SHALL generate query embedding
- **AND** system SHALL search note embeddings with cosine similarity
- **AND** system SHALL return top-k results above threshold

#### Scenario: Filter notes by scope
- **WHEN** searching notes with layer filter
- **THEN** system SHALL only return notes from specified layer and parent layers
- **AND** system SHALL apply layer precedence rules (project > team > org > tenant)

### Requirement: Note Quality Scoring
The system SHALL track note quality through usage metrics and feedback signals.

#### Scenario: Track note retrieval
- **WHEN** note is retrieved during context assembly
- **THEN** system SHALL increment retrieval_count for that note
- **AND** system SHALL record retrieval context (query, session)

#### Scenario: Track note usefulness
- **WHEN** user provides positive feedback on session outcome
- **AND** notes were included in that session's context
- **THEN** system SHALL increment usefulness_score for those notes

#### Scenario: Deprecate low-quality notes
- **WHEN** note has retrieval_count > 10
- **AND** usefulness_score / retrieval_count < 0.1
- **THEN** system SHALL flag note for review
- **AND** system SHALL reduce note ranking in future retrievals

### Requirement: Note Deduplication
The system SHALL detect and handle duplicate or near-duplicate notes.

#### Scenario: Detect duplicate during distillation
- **WHEN** distilling new trajectory
- **AND** generated note embedding has >0.95 similarity with existing note
- **THEN** system SHALL skip creating new note
- **AND** system SHALL update existing note's reference count
- **AND** system SHALL log duplicate detection

#### Scenario: Merge similar notes
- **WHEN** two notes have >0.9 similarity
- **AND** both have status='accepted'
- **THEN** system MAY suggest merge to maintainer
- **AND** system SHALL NOT auto-merge without approval

### Requirement: Note Lifecycle
The system SHALL manage note lifecycle from draft through acceptance to potential deprecation.

#### Scenario: Note lifecycle states
- **WHEN** note is created
- **THEN** note SHALL have status='draft'
- **AND** note MAY transition to: proposed, accepted, deprecated, rejected

#### Scenario: Auto-propose high-quality notes
- **WHEN** draft note has usefulness_score > 0.8
- **AND** note has been retrieved > 5 times
- **THEN** system SHALL auto-transition to status='proposed'
- **AND** system SHALL notify maintainers for review

### Requirement: Observability
The system SHALL emit metrics and logs for note-taking operations.

#### Scenario: Emit distillation metrics
- **WHEN** distillation completes
- **THEN** system SHALL emit histogram: note_taking.distillation.duration_ms
- **AND** system SHALL emit counter: note_taking.distillation.total with status label (success/skip/error)

#### Scenario: Emit retrieval metrics
- **WHEN** note retrieval completes
- **THEN** system SHALL emit histogram: note_taking.retrieval.results_count
- **AND** system SHALL emit histogram: note_taking.retrieval.latency_ms

### Requirement: Trajectory Capture Latency Control
The system SHALL minimize latency overhead when capturing tool execution trajectories.

#### Scenario: Asynchronous trajectory capture
- **WHEN** tool execution completes
- **THEN** system SHALL capture trajectory asynchronously (non-blocking)
- **AND** system SHALL NOT add latency to tool execution response
- **AND** system SHALL buffer captures for batch write

#### Scenario: Batch trajectory writes
- **WHEN** trajectory buffer reaches threshold (configurable, default: 10 events or 5 seconds)
- **THEN** system SHALL flush buffer to storage
- **AND** system SHALL emit metric: note_taking.capture.batch_size
- **AND** system SHALL handle flush failures with retry queue

#### Scenario: Sampling for high-volume operations
- **WHEN** tool execution rate exceeds threshold (configurable, default: 100/minute)
- **THEN** system SHALL sample captures (configurable rate, default: 10%)
- **AND** system SHALL log sampling active status
- **AND** system SHALL capture all failed executions regardless of sampling

