---
title: Configuration Specification
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 05-adapter-architecture.md
  - 08-deployment.md
---

# Configuration Specification

This document specifies the configuration schema, environment variables, and validation rules for the Memory-Knowledge system.

## Purpose

Define the supported configuration model, loading behavior, and validation rules for Aeterna runtimes.
## Requirements
### Requirement: Configuration Loading

The system SHALL load configuration from supported sources using a deterministic precedence order.

#### Scenario: Higher-precedence sources override lower-precedence sources
- **WHEN** the runtime resolves configuration
- **THEN** environment variables SHALL override project configuration files
- **AND** project configuration files SHALL override user configuration files
- **AND** built-in defaults SHALL apply only when no higher-precedence source provides a value

### Requirement: Environment Variable Configuration

The system SHALL document and support environment variables for configuration overrides.

#### Scenario: Environment variables override persisted configuration
- **WHEN** a supported configuration environment variable is set
- **THEN** the runtime SHALL apply that value over persisted file configuration
- **AND** the variable naming contract SHALL be documented consistently

### Requirement: Configuration Validation

The system SHALL validate configuration for schema correctness and semantic consistency before use.

#### Scenario: Invalid configuration is rejected
- **WHEN** configuration contains invalid schema or incompatible semantic combinations
- **THEN** the runtime SHALL return a validation error
- **AND** it SHALL NOT continue with silently accepted invalid configuration

### Requirement: CLI Configuration Management Surface
The system SHALL provide a supported CLI configuration management surface for control-plane usage.

#### Scenario: User inspects effective CLI configuration
- **WHEN** a user runs the supported CLI config display command
- **THEN** the CLI SHALL show the effective control-plane configuration after applying precedence rules
- **AND** the output SHALL identify the selected profile, target server URL, and source files or environment overrides in effect

#### Scenario: User updates a target profile
- **WHEN** a user updates a named target profile through the supported CLI configuration surface
- **THEN** the CLI SHALL persist the new settings to the canonical configuration location
- **AND** the CLI SHALL validate that the resulting profile remains usable

### Requirement: Canonical Aeterna Configuration Precedence
The system SHALL define and implement one canonical configuration precedence model for the current `AETERNA_*` runtime.

#### Scenario: User-level and project-level config coexist
- **WHEN** both user-level and project-level CLI configuration files exist
- **THEN** the system SHALL apply a documented precedence order between project config, user config, environment variables, and CLI overrides
- **AND** the same precedence model SHALL be used consistently by runtime commands and auth/config commands

#### Scenario: Canonical config file locations are documented
- **WHEN** the CLI reads or writes persistent control-plane configuration
- **THEN** it SHALL use the documented canonical file locations for user-level and project-level config
- **AND** documentation and error messages SHALL reference those same canonical paths consistently

### Requirement: CLI Canonical Configuration Loading
Configuration loading for the current Aeterna CLI runtime SHALL use the supported Aeterna configuration sources and precedence.

#### Scenario: Aeterna CLI/server configuration precedence
- **WHEN** the CLI or server resolves configuration for control-plane or runtime usage
- **THEN** CLI flags or explicit command overrides SHALL have the highest precedence
- **AND** `AETERNA_*` environment variables SHALL override persisted config files
- **AND** project-level Aeterna config SHALL override user-level config
- **AND** built-in defaults SHALL apply only when no higher-precedence source provides a value

#### Scenario: Unsupported legacy variable set is not treated as canonical
- **WHEN** configuration documentation describes environment-variable control of the current Aeterna runtime
- **THEN** the documented canonical variable namespace SHALL match the implemented `AETERNA_*` runtime variables
- **AND** unsupported legacy namespaces SHALL NOT be documented as the primary current contract

## Table of Contents

1. [Overview](#overview)
2. [Configuration Schema](#configuration-schema)
3. [Environment Variables](#environment-variables)
4. [Configuration Loading](#configuration-loading)
5. [Validation](#validation)
6. [Configuration Examples](#configuration-examples)

---

## Overview

Configuration is loaded from multiple sources with precedence:

```
┌─────────────────────────────────────────────────────────────────┐
│                  CONFIGURATION PRECEDENCE                        │
│                  (highest to lowest)                             │
│                                                                  │
│  1. Environment variables                                        │
│     MK_MEMORY_PROVIDER=mem0                                     │
│                                                                  │
│  2. Runtime configuration (programmatic)                         │
│     core.configure({ memory: { provider: 'mem0' } })            │
│                                                                  │
│  3. Project configuration file                                   │
│     .memory-knowledge/config.json                               │
│                                                                  │
│  4. User configuration file                                      │
│     ~/.config/memory-knowledge/config.json                      │
│                                                                  │
│  5. System defaults                                              │
│     Built-in defaults                                           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Configuration Schema

### Root Configuration

```typescript
interface MemoryKnowledgeConfig {
  /**
   * Configuration version.
   * @required
   */
  version: '1.0';
  
  /**
   * Memory system configuration.
   */
  memory?: MemoryConfig;
  
  /**
   * Knowledge repository configuration.
   */
  knowledge?: KnowledgeConfig;
  
  /**
   * Sync bridge configuration.
   */
  sync?: SyncConfig;
  
  /**
   * Layer identifiers (for scoping).
   */
  identifiers?: IdentifiersConfig;
  
  /**
   * Logging configuration.
   */
  logging?: LoggingConfig;
  
  /**
   * Feature flags.
   */
  features?: FeaturesConfig;
}
```

### Memory Configuration

```typescript
interface MemoryConfig {
  /**
   * Memory provider to use.
   * @default "mem0"
   */
  provider: 'mem0' | 'letta' | 'chroma' | 'pinecone' | 'qdrant' | 'custom';
  
  /**
   * Provider-specific configuration.
   */
  providerConfig?: ProviderSpecificConfig;
  
  /**
   * Default layer for memory operations.
   * @default "user"
   */
  defaultLayer?: MemoryLayer;
  
  /**
   * Layers accessible in this context.
   * @default ["agent", "user", "session", "project"]
   */
  enabledLayers?: MemoryLayer[];
  
  /**
   * Search defaults.
   */
  search?: {
    /**
     * Default result limit.
     * @default 10
     */
    defaultLimit?: number;
    
    /**
     * Default similarity threshold.
     * @default 0.7
     */
    defaultThreshold?: number;
    
    /**
     * Maximum allowed limit.
     * @default 100
     */
    maxLimit?: number;
  };
  
  /**
   * Memory lifecycle configuration.
   */
  lifecycle?: {
    /**
     * Enable memory decay.
     * @default false
     */
    enableDecay?: boolean;
    
    /**
     * Decay rate per day (0.0 - 1.0).
     * @default 0.01
     */
    decayRate?: number;
    
    /**
     * Enable consolidation.
     * @default false
     */
    enableConsolidation?: boolean;
    
    /**
     * Similarity threshold for consolidation.
     * @default 0.95
     */
    consolidationThreshold?: number;
  };
  
  /**
   * Session memory configuration.
   */
  session?: {
    /**
     * Auto-delete session memories on end.
     * @default false
     */
    autoDelete?: boolean;
    
    /**
     * Retention period after session end.
     * @default "7d"
     */
    retentionPeriod?: string;
    
    /**
     * Promote important memories to user layer.
     * @default true
     */
    promoteImportant?: boolean;
  };
}
```

### Provider-Specific Configuration

```typescript
// Mem0 configuration
interface Mem0ProviderConfig {
  provider: 'mem0';
  apiKey: string;
  baseUrl?: string;
  organizationId?: string;
  projectId?: string;
}

// Letta configuration
interface LettaProviderConfig {
  provider: 'letta';
  baseUrl: string;
  apiKey?: string;
  agentId?: string;
}

// Chroma configuration
interface ChromaProviderConfig {
  provider: 'chroma';
  host?: string;
  port?: number;
  ssl?: boolean;
  tenant?: string;
  database?: string;
}

// Pinecone configuration
interface PineconeProviderConfig {
  provider: 'pinecone';
  apiKey: string;
  environment: string;
  indexName: string;
  namespace?: string;
}

// Qdrant configuration
interface QdrantProviderConfig {
  provider: 'qdrant';
  url: string;
  apiKey?: string;
  collectionPrefix?: string;
}

type ProviderSpecificConfig = 
  | Mem0ProviderConfig
  | LettaProviderConfig
  | ChromaProviderConfig
  | PineconeProviderConfig
  | QdrantProviderConfig;
```

### Knowledge Configuration

```typescript
interface KnowledgeConfig {
  /**
   * Path to knowledge repository.
   * Can be local path or Git URL.
   * @default ".knowledge"
   */
  repository?: string;
  
  /**
   * Git branch to use.
   * @default "main"
   */
  branch?: string;
  
  /**
   * Enable Git operations (push/pull).
   * @default true
   */
  enableGit?: boolean;
  
  /**
   * Auto-sync from upstream.
   * @default true
   */
  autoSync?: boolean;
  
  /**
   * Federation configuration.
   */
  federation?: {
    /**
     * Central hub repository URL.
     */
    centralHub?: string;
    
    /**
     * Upstream repositories.
     */
    upstreams?: Array<{
      id: string;
      url: string;
      branch?: string;
      layers?: KnowledgeLayer[];
    }>;
    
    /**
     * Sync interval.
     * @default "6h"
     */
    syncInterval?: string;
  };
  
  /**
   * Constraint enforcement.
   */
  constraints?: {
    /**
     * Enforcement mode.
     * - strict: Block on any violation
     * - warn: Warn but allow
     * - off: No enforcement
     * @default "strict"
     */
    mode?: 'strict' | 'warn' | 'off';
    
    /**
     * Exclude patterns from checking.
     */
    exclude?: string[];
  };
}
```

### Sync Configuration

```typescript
interface SyncConfig {
  /**
   * Enable automatic sync.
   * @default true
   */
  enabled?: boolean;
  
  /**
   * Sync triggers.
   */
  triggers?: {
    /**
     * Sync on session start.
     * @default true
     */
    onSessionStart?: boolean;
    
    /**
     * Sync on session end.
     * @default false
     */
    onSessionEnd?: boolean;
    
    /**
     * Sync on knowledge change webhook.
     * @default true
     */
    onKnowledgeChange?: boolean;
    
    /**
     * Scheduled sync interval.
     * @default "1h"
     */
    scheduledInterval?: string;
    
    /**
     * Staleness threshold for forced sync.
     * @default "24h"
     */
    stalenessThreshold?: string;
  };
  
  /**
   * Conflict resolution.
   */
  conflictResolution?: {
    /**
     * Default resolution strategy.
     * @default "knowledge-wins"
     */
    strategy?: 'knowledge-wins' | 'memory-wins' | 'manual';
    
    /**
     * Custom resolver function name.
     */
    customResolver?: string;
  };
  
  /**
   * Sync state storage.
   */
  stateStorage?: {
    /**
     * Storage type.
     * @default "file"
     */
    type?: 'file' | 'memory' | 'database';
    
    /**
     * File path (if type is 'file').
     */
    path?: string;
  };
}
```

### Identifiers Configuration

```typescript
interface IdentifiersConfig {
  /**
   * Tenant/tenant identifier.
   */
  companyId?: string;
  
  /**
   * Organization identifier.
   */
  orgId?: string;
  
  /**
   * Team identifier.
   */
  teamId?: string;
  
  /**
   * Project identifier.
   * @default Auto-detected from Git
   */
  projectId?: string;
  
  /**
   * User identifier.
   * @default Auto-detected from environment
   */
  userId?: string;
  
  /**
   * Agent identifier.
   */
  agentId?: string;
}
```

### Logging Configuration

```typescript
interface LoggingConfig {
  /**
   * Log level.
   * @default "info"
   */
  level?: 'debug' | 'info' | 'warn' | 'error';
  
  /**
   * Log format.
   * @default "json"
   */
  format?: 'json' | 'pretty' | 'minimal';
  
  /**
   * Include timestamps.
   * @default true
   */
  timestamps?: boolean;
  
  /**
   * Log destination.
   * @default "stderr"
   */
  destination?: 'stdout' | 'stderr' | 'file';
  
  /**
   * Log file path (if destination is 'file').
   */
  filePath?: string;
}
```

### Features Configuration

```typescript
interface FeaturesConfig {
  /**
   * Enable memory tools.
   * @default true
   */
  memoryTools?: boolean;
  
  /**
   * Enable knowledge tools.
   * @default true
   */
  knowledgeTools?: boolean;
  
  /**
   * Enable sync tools.
   * @default true
   */
  syncTools?: boolean;
  
  /**
   * Enable constraint checking.
   * @default true
   */
  constraintChecking?: boolean;
  
  /**
   * Enable session context injection.
   * @default true
   */
  contextInjection?: boolean;
  
  /**
   * Enable telemetry.
   * @default false
   */
  telemetry?: boolean;
}
```

---

## Environment Variables

### Variable Naming Convention

```
MK_<SECTION>_<PROPERTY>=value
```

### Complete Variable List

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `MK_MEMORY_PROVIDER` | string | `mem0` | Memory provider |
| `MK_MEMORY_API_KEY` | string | - | Provider API key |
| `MK_MEMORY_BASE_URL` | string | - | Provider base URL |
| `MK_MEMORY_DEFAULT_LAYER` | string | `user` | Default memory layer |
| `MK_KNOWLEDGE_REPOSITORY` | string | `.knowledge` | Knowledge repo path |
| `MK_KNOWLEDGE_BRANCH` | string | `main` | Git branch |
| `MK_KNOWLEDGE_CENTRAL_HUB` | string | - | Central hub URL |
| `MK_SYNC_ENABLED` | boolean | `true` | Enable sync |
| `MK_SYNC_INTERVAL` | string | `1h` | Sync interval |
| `MK_ID_COMPANY` | string | - | Tenant ID |
| `MK_ID_ORG` | string | - | Organization ID |
| `MK_ID_TEAM` | string | - | Team ID |
| `MK_ID_PROJECT` | string | - | Project ID |
| `MK_ID_USER` | string | - | User ID |
| `MK_ID_AGENT` | string | - | Agent ID |
| `MK_LOG_LEVEL` | string | `info` | Log level |
| `MK_LOG_FORMAT` | string | `json` | Log format |
| `MK_FEATURES_TELEMETRY` | boolean | `false` | Enable telemetry |

### Provider-Specific Variables

#### Mem0
```bash
MK_MEMORY_PROVIDER=mem0
MK_MEM0_API_KEY=m0-xxxxx
MK_MEM0_BASE_URL=https://api.mem0.ai
MK_MEM0_ORG_ID=org_123
MK_MEM0_PROJECT_ID=proj_456
```

#### Letta
```bash
MK_MEMORY_PROVIDER=letta
MK_LETTA_BASE_URL=http://localhost:8283
MK_LETTA_API_KEY=optional
MK_LETTA_AGENT_ID=agent_123
```

#### Chroma
```bash
MK_MEMORY_PROVIDER=chroma
MK_CHROMA_HOST=localhost
MK_CHROMA_PORT=8000
MK_CHROMA_SSL=false
```

#### Pinecone
```bash
MK_MEMORY_PROVIDER=pinecone
MK_PINECONE_API_KEY=xxxxx
MK_PINECONE_ENVIRONMENT=us-east1-gcp
MK_PINECONE_INDEX=memories
```

#### Qdrant
```bash
MK_MEMORY_PROVIDER=qdrant
MK_QDRANT_URL=http://localhost:6333
MK_QDRANT_API_KEY=optional
```

---

## Configuration Loading

### Loading Order

```typescript
async function loadConfiguration(): Promise<MemoryKnowledgeConfig> {
  // 1. Start with defaults
  let config = getDefaultConfig();
  
  // 2. Load system config (if exists)
  const systemConfig = await loadFile('/etc/memory-knowledge/config.json');
  if (systemConfig) {
    config = mergeConfig(config, systemConfig);
  }
  
  // 3. Load user config
  const userConfig = await loadFile(
    path.join(os.homedir(), '.config/memory-knowledge/config.json')
  );
  if (userConfig) {
    config = mergeConfig(config, userConfig);
  }
  
  // 4. Load project config
  const projectConfig = await loadFile('.memory-knowledge/config.json');
  if (projectConfig) {
    config = mergeConfig(config, projectConfig);
  }
  
  // 5. Apply environment variables
  config = applyEnvironmentVariables(config);
  
  // 6. Validate final config
  validateConfig(config);
  
  return config;
}
```

### Default Configuration

```typescript
function getDefaultConfig(): MemoryKnowledgeConfig {
  return {
    version: '1.0',
    memory: {
      provider: 'mem0',
      defaultLayer: 'user',
      enabledLayers: ['agent', 'user', 'session', 'project'],
      search: {
        defaultLimit: 10,
        defaultThreshold: 0.7,
        maxLimit: 100
      },
      lifecycle: {
        enableDecay: false,
        decayRate: 0.01,
        enableConsolidation: false,
        consolidationThreshold: 0.95
      },
      session: {
        autoDelete: false,
        retentionPeriod: '7d',
        promoteImportant: true
      }
    },
    knowledge: {
      repository: '.knowledge',
      branch: 'main',
      enableGit: true,
      autoSync: true,
      constraints: {
        mode: 'strict',
        exclude: []
      }
    },
    sync: {
      enabled: true,
      triggers: {
        onSessionStart: true,
        onSessionEnd: false,
        onKnowledgeChange: true,
        scheduledInterval: '1h',
        stalenessThreshold: '24h'
      },
      conflictResolution: {
        strategy: 'knowledge-wins'
      },
      stateStorage: {
        type: 'file'
      }
    },
    logging: {
      level: 'info',
      format: 'json',
      timestamps: true,
      destination: 'stderr'
    },
    features: {
      memoryTools: true,
      knowledgeTools: true,
      syncTools: true,
      constraintChecking: true,
      contextInjection: true,
      telemetry: false
    }
  };
}
```

### Config File Formats

#### JSON

```json
{
  "version": "1.0",
  "memory": {
    "provider": "mem0",
    "providerConfig": {
      "apiKey": "${MK_MEM0_API_KEY}"
    },
    "defaultLayer": "user"
  },
  "knowledge": {
    "repository": ".knowledge",
    "federation": {
      "centralHub": "https://github.com/example/knowledge-hub.git"
    }
  }
}
```

#### YAML (Alternative)

```yaml
version: "1.0"

memory:
  provider: mem0
  providerConfig:
    apiKey: ${MK_MEM0_API_KEY}
  defaultLayer: user

knowledge:
  repository: .knowledge
  federation:
    centralHub: https://github.com/example/knowledge-hub.git
```

---

## Validation

### Schema Validation

```typescript
import Ajv from 'ajv';

const configSchema = {
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  type: 'object',
  required: ['version'],
  properties: {
    version: {
      type: 'string',
      enum: ['1.0']
    },
    memory: {
      type: 'object',
      properties: {
        provider: {
          type: 'string',
          enum: ['mem0', 'letta', 'chroma', 'pinecone', 'qdrant', 'custom']
        },
        defaultLayer: {
          type: 'string',
          enum: ['agent', 'user', 'session', 'project', 'team', 'org', 'tenant']
        }
        // ... more properties
      }
    }
    // ... more sections
  }
};

function validateConfig(config: unknown): asserts config is MemoryKnowledgeConfig {
  const ajv = new Ajv();
  const validate = ajv.compile(configSchema);
  
  if (!validate(config)) {
    throw new ConfigValidationError(validate.errors);
  }
}
```

### Semantic Validation

```typescript
function validateConfigSemantics(config: MemoryKnowledgeConfig): void {
  // Validate provider config matches provider type
  if (config.memory?.provider === 'mem0' && !config.memory?.providerConfig?.apiKey) {
    throw new ConfigValidationError('Mem0 requires apiKey');
  }
  
  // Validate layer configuration
  if (config.memory?.defaultLayer && !config.memory?.enabledLayers?.includes(config.memory.defaultLayer)) {
    throw new ConfigValidationError('defaultLayer must be in enabledLayers');
  }
  
  // Validate sync configuration
  if (config.sync?.triggers?.scheduledInterval) {
    const interval = parseDuration(config.sync.triggers.scheduledInterval);
    if (interval < 60000) { // 1 minute minimum
      throw new ConfigValidationError('scheduledInterval must be at least 1m');
    }
  }
  
  // Validate federation configuration
  if (config.knowledge?.federation?.upstreams) {
    const ids = new Set<string>();
    for (const upstream of config.knowledge.federation.upstreams) {
      if (ids.has(upstream.id)) {
        throw new ConfigValidationError(`Duplicate upstream id: ${upstream.id}`);
      }
      ids.add(upstream.id);
    }
  }
}
```

### Configuration Errors

```typescript
class ConfigValidationError extends Error {
  constructor(
    public errors: Array<{
      path: string;
      message: string;
      value?: unknown;
    }>
  ) {
    const messages = errors.map(e => `${e.path}: ${e.message}`).join('; ');
    super(`Configuration validation failed: ${messages}`);
    this.name = 'ConfigValidationError';
  }
}
```

---

## Configuration Examples

### Minimal Configuration

```json
{
  "version": "1.0",
  "memory": {
    "provider": "mem0",
    "providerConfig": {
      "apiKey": "m0-xxxxx"
    }
  }
}
```

### Development Configuration

```json
{
  "version": "1.0",
  "memory": {
    "provider": "chroma",
    "providerConfig": {
      "host": "localhost",
      "port": 8000
    },
    "defaultLayer": "session"
  },
  "knowledge": {
    "repository": ".knowledge",
    "enableGit": false,
    "constraints": {
      "mode": "warn"
    }
  },
  "sync": {
    "enabled": false
  },
  "logging": {
    "level": "debug",
    "format": "pretty"
  }
}
```

### Production Configuration

```json
{
  "version": "1.0",
  "memory": {
    "provider": "pinecone",
    "providerConfig": {
      "apiKey": "${MK_PINECONE_API_KEY}",
      "environment": "us-east1-gcp",
      "indexName": "production-memories"
    },
    "defaultLayer": "user",
    "enabledLayers": ["user", "project", "team", "org", "tenant"],
    "lifecycle": {
      "enableDecay": true,
      "decayRate": 0.005
    }
  },
  "knowledge": {
    "repository": "git@github.com:example/knowledge-repo.git",
    "branch": "main",
    "federation": {
      "centralHub": "git@github.com:example/central-knowledge.git",
      "syncInterval": "1h"
    },
    "constraints": {
      "mode": "strict"
    }
  },
  "sync": {
    "enabled": true,
    "triggers": {
      "onSessionStart": true,
      "scheduledInterval": "30m",
      "stalenessThreshold": "6h"
    }
  },
  "identifiers": {
    "companyId": "acme-corp",
    "orgId": "${MK_ID_ORG}"
  },
  "logging": {
    "level": "info",
    "format": "json",
    "destination": "stdout"
  },
  "features": {
    "telemetry": true
  }
}
```

### Multi-Tenant Configuration

```json
{
  "version": "1.0",
  "memory": {
    "provider": "qdrant",
    "providerConfig": {
      "url": "https://qdrant.internal.example.com",
      "apiKey": "${MK_QDRANT_API_KEY}",
      "collectionPrefix": "${MK_ID_COMPANY}_"
    },
    "enabledLayers": ["agent", "user", "session", "project", "team", "org", "tenant"]
  },
  "knowledge": {
    "repository": ".knowledge",
    "federation": {
      "centralHub": "git@github.com:example/central-knowledge.git",
      "upstreams": [
        {
          "id": "tenant",
          "url": "git@github.com:example/tenant-policies.git",
          "layers": ["tenant"]
        },
        {
          "id": "org",
          "url": "git@github.com:example/${MK_ID_ORG}-knowledge.git",
          "layers": ["org"]
        }
      ]
    }
  },
  "identifiers": {
    "companyId": "${MK_ID_COMPANY}",
    "orgId": "${MK_ID_ORG}",
    "teamId": "${MK_ID_TEAM}",
    "projectId": "${MK_ID_PROJECT}"
  }
}
```

---

**Next**: [08-deployment.md](./08-deployment.md) - Deployment Specification
