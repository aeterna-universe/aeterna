# Architecture Overview

## Overview

Aeterna combines hierarchical memory, Git-backed knowledge, tenant-aware governance, and a control plane for operating multi-tenant AI systems. The current architecture is organized around four boundaries:

1. **Tenant boundary** — each tenant has isolated governance, hierarchy, and knowledge-repository bindings
2. **Control-plane boundary** — PlatformAdmin can operate across tenants; TenantAdmin remains scoped to a single tenant
3. **Config/secrets boundary** — tenant config is structured and non-secret; secret values live separately and are referenced logically
4. **Deployment boundary** — the public Helm chart defines runtime structure, while environment-specific overlays and secret material live in a private deployment repository

## 7-Layer Memory Hierarchy
1. **Agent**: Private to the specific agent instance.
2. **User**: Private to the user interacting with the agent.
3. **Session**: Specific to a single interaction session.
4. **Project**: Shared across a specific project.
5. **Team**: Shared across a team.
6. **Org**: Shared across an organization.
7. **Tenant**: Shared across the entire tenant.

## Tenant control plane and knowledge binding

Each tenant has exactly one canonical repository binding for its knowledge repository. The binding is managed through the CLI/server control plane and can reference:

- a local path
- a generic Git remote
- GitHub with credential references
- a shared platform-owned Git provider connection for GitHub App connectivity

The resolver fails closed when no valid binding exists or when a tenant tries to reference an unapproved shared Git provider connection.

## Tenant config provider

Tenant runtime configuration is managed through a `TenantConfigProvider` abstraction.

The first implementation is Kubernetes-backed:

- ConfigMap: `aeterna-tenant-<tenant-id>`
- Secret: `aeterna-tenant-<tenant-id>-secret`

This separation keeps raw secret values out of persisted control-plane records and API responses while allowing Helm and deployment automation to materialize the same contract.

## Shared Git provider connections

GitHub App connectivity can be modeled once as a platform-owned shared connection and then granted to one or more tenants by explicit visibility rules.

- shared: connection metadata, PEM secret reference, webhook secret reference
- tenant-isolated: repository binding, tenant config, tenant secret entries, deployment artifacts

Tenants reference only a connection ID; they never receive PEM material.

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
- `cli`: Axum server and CLI control-plane implementation.
- `adapters`: Ecosystem integrations (e.g., OpenCode).
- `tools`: MCP tool interface for agents.
- `config`: System-wide configuration.
- `utils`: Shared utilities.
- `errors`: Unified error handling.
