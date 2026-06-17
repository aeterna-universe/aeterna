# Architecture: Memory-Knowledge System

## Overview
The Memory-Knowledge System is a hierarchical storage and governed knowledge framework for AI agents. It provides a structured way for agents to store and retrieve memory across different scopes (from session to organization) while adhering to organizational knowledge (ADRs, Policies, Patterns, Specs).

## 7-Layer Memory Hierarchy
1. **Agent**: Private to the specific agent instance.
2. **User**: Private to the user interacting with the agent.
3. **Session**: Specific to a single interaction session.
4. **Project**: Shared across a specific project.
5. **Team**: Shared across a team.
6. **Org**: Shared across an organization.
7. **Tenant**: Shared across the entire tenant.

## 4-Type Knowledge Repository
1. **ADR (Architectural Decision Record)**: Captures technical decisions.
2. **Policy**: Governs behavior and compliance.
3. **Pattern**: Best practices and reusable solutions.
4. **Spec**: Functional and technical specifications.

## Crate Structure
- `core`: Fundamental types and traits.
- `memory`: Implementation of the 7-layer memory system.
- `knowledge`: Implementation of the knowledge repository.
- `sync`: Coordination between memory and knowledge.
- `storage`: Physical storage adapters (PostgreSQL, Qdrant, Redis).
- `adapters`: Ecosystem integrations (e.g., OpenCode).
- `tools`: MCP tool interface for agents.
- `config`: System-wide configuration.
- `utils`: Shared utilities.
- `errors`: Unified error handling.
