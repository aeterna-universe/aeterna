---
sidebar_position: 12
---

# OpenCode Daily Usage Playbook

This guide shows how a human should use Aeterna inside OpenCode day to day: when to rely on automatic context injection, when to store memory, when to query knowledge, and when to promote something into governed knowledge.

## Mental Model

- **Memory** = practical retained context
- **Knowledge** = governed, reusable truth

Use memory for:
- local facts discovered during work
- team/project conventions
- repeated troubleshooting lessons
- user preferences and current-session context

Use knowledge for:
- ADRs
- reusable implementation patterns
- policies and constraints
- stable references other people should discover later

## Memory and Knowledge Structure

### Memory layers

The OpenCode plugin uses Aeterna's 7-layer memory hierarchy:

1. `agent`
2. `user`
3. `session`
4. `project`
5. `team`
6. `org`
7. `tenant`

Practical usage:

| Layer | Use for |
|------|---------|
| `session` | current task context, debugging breadcrumbs, temporary decisions |
| `user` | personal preferences and working style |
| `project` | repo conventions, architecture facts, implementation norms |
| `team` / `org` / `tenant` | broader conventions, standards, compliance guidance |

### Knowledge types

The plugin exposes structured knowledge items as:

- `adr`
- `pattern`
- `policy`
- `reference`

Scopes are:

- `project`
- `team`
- `org`
- `tenant`

## First Rule: Let OpenCode Pull Context First

The plugin already injects relevant knowledge and session memory into chat automatically. Start by asking normal working questions:

- "What do we already know about tenant provisioning here?"
- "Search prior decisions about OAuth callback handling."
- "What governance or policy constraints apply to this change?"

That works well when you need retrieval.

## When to Add Memory Explicitly

Tell OpenCode to remember something when you discover a fact that should survive this message and help later work.

Good examples:

- "Remember that this repo uses Postgres read replicas for analytics queries."
- "Remember that admin bootstrap env vars must stay outside plugin-auth gating."
- "Remember that this project prefers fail-closed auth behavior."

Bad examples:

- giant copied logs
- vague opinions
- full ADR text dumps
- transient noise with no future value

### Good memory style

Keep it:
- short
- factual
- scoped
- operational

Prefer:

> "Project uses structured errors and avoids stringly typed error matching."

Over:

> "There was a long discussion and people generally felt error handling should be nicer."

## When to Query Knowledge

Query knowledge before proposing a new pattern or decision.

Use prompts like:

- "Do we already have an ADR about this?"
- "Search knowledge for multi-tenant webhook rules."
- "Find project or team patterns for GitHub App auth."

This prevents duplicate decisions and helps anchor the session in existing norms.

## When to Promote Memory Into Knowledge

Promote only when a memory becomes reusable across people or across time.

Good promotion triggers:

- you used the same lesson more than once
- it affects more than one engineer or repo area
- it should be reviewed/governed
- it belongs in an ADR/pattern/policy/reference document

Examples:

- repeated migration workflow lesson → `pattern`
- architectural decision with tradeoffs → `adr`
- mandatory constraint → `policy`
- operational runbook-style fact → `reference`

## Daily Workflow

### 1. Start the session by retrieving context

Ask:

- "What matters in this repo today?"
- "Any existing decisions about auth / migrations / tenant config?"
- "What recent memories are relevant to this task?"

### 2. Work normally and let automatic context injection help

The plugin will enrich prompts with recent memory and relevant knowledge. You do not need to manually ask for retrieval on every turn.

### 3. Capture useful discoveries explicitly

When you learn something reusable:

- "Remember that this deployment requires cnpg disabled until the label fix lands."
- "Remember that this team uses top-level defaultTenantId in chart 0.6.0+."

### 4. Re-check knowledge before making a new formal recommendation

Ask:

- "Do we already have a policy or pattern for this?"
- "Search org knowledge before we propose a new approach."

### 5. Promote only the things that should become shared truth

Ask:

- "Promote that to project memory."
- "Turn that into a team-level pattern proposal."
- "Create an ADR proposal from this decision."

## Example Prompts That Work Well

### Retrieval

- "Search my memory for database preferences."
- "What do we already know about OAuth callback handling?"
- "Check project knowledge for CI deployment rules."

### Capture

- "Remember that we pin chart version 0.6.0 for this environment."
- "Remember that this repo uses OpenSpec for non-trivial changes."
- "Remember that these E2E folders require plugin auth enabled."

### Promotion

- "This should be promoted to project memory."
- "Create a pattern proposal from that."
- "This belongs in org knowledge, not just session memory."

## Anti-Patterns

Avoid using memory as:

- a dumping ground for long documents
- a replacement for formal knowledge
- a place for secrets or credentials
- a place for every tiny transient observation

Avoid using knowledge for:

- half-formed tactical notes
- one-off debugging breadcrumbs
- personal-only preferences

## Best Practice Summary

Use this rule of thumb:

- **helps me right now** → memory
- **should help others repeatedly** → knowledge
- **needs review and governance** → propose knowledge

For most OpenCode sessions, the best pattern is:

1. retrieve first
2. capture sparingly
3. promote intentionally

## Related Docs

- [OpenCode Integration Guide](../integrations/opencode-integration)
- [CLI Quick Reference](./cli-quick-reference)
- [Tenant Admin Control Plane](./tenant-admin-control-plane)
