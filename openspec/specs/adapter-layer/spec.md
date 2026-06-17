---
title: Adapter Architecture Specification
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 00-overview.md
  - 02-memory-system.md
  - 06-tool-interface.md
---

# Adapter Architecture Specification

This document specifies the two-layer adapter architecture: **Provider Adapters** (storage backends) and **Ecosystem Adapters** (AI agent frameworks).

## Table of Contents

1. [Overview](#overview)
2. [Provider Adapters](#provider-adapters)
3. [Ecosystem Adapters](#ecosystem-adapters)
4. [Adapter Registration](#adapter-registration)
5. [Adapter Lifecycle](#adapter-lifecycle)
6. [Testing Adapters](#testing-adapters)
7. [Reference Implementations](#reference-implementations)

---

## Overview

The adapter architecture enables pluggable integration:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│                     AI AGENT ECOSYSTEMS                          │
│                                                                  │
│  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐        │
│  │ LangChain │ │  AutoGen  │ │  CrewAI   │ │ OpenCode  │        │
│  └─────┬─────┘ └─────┬─────┘ └─────┬─────┘ └─────┬─────┘        │
│        │             │             │             │               │
│        │    Ecosystem Adapter Interface          │               │
│        └─────────────┴─────────────┴─────────────┘               │
│                             │                                    │
│                             ▼                                    │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                                                          │    │
│  │              MEMORY-KNOWLEDGE CORE                       │    │
│  │                                                          │    │
│  │  • Unified API                                           │    │
│  │  • Layer resolution                                      │    │
│  │  • Constraint evaluation                                 │    │
│  │  • Sync orchestration                                    │    │
│  │                                                          │    │
│  └─────────────────────────────────────────────────────────┘    │
│                             │                                    │
│        ┌─────────────┬──────┴──────┬─────────────┐              │
│        │             │             │             │               │
│        │    Provider Adapter Interface           │               │
│        ▼             ▼             ▼             ▼               │
│  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐        │
│  │   Mem0    │ │   Letta   │ │  Chroma   │ │ Pinecone  │        │
│  └───────────┘ └───────────┘ └───────────┘ └───────────┘        │
│                                                                  │
│                     STORAGE PROVIDERS                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Design Goals

| Goal | Description |
|------|-------------|
| **Pluggability** | Add new providers/ecosystems without core changes |
| **Isolation** | Adapter failures don't crash the core |
| **Testability** | Mock adapters for testing |
| **Discoverability** | Auto-detect available adapters |
| **Versioning** | Adapters specify compatible core versions |

---

## Provider Adapters

### Provider Adapter Interface

```typescript
/**
 * Interface for memory storage providers.
 * Implement this to add support for a new storage backend.
 */
interface MemoryProviderAdapter {
  // ─────────────────────────────────────────────────────────────
  // METADATA
  // ─────────────────────────────────────────────────────────────
  
  /** Provider identifier (e.g., "mem0", "letta", "chroma") */
  readonly id: string;
  
  /** Human-readable name */
  readonly name: string;
  
  /** Provider version */
  readonly version: string;
  
  /** Compatible core versions (semver range) */
  readonly coreCompatibility: string;
  
  /** Provider capabilities */
  readonly capabilities: ProviderCapabilities;
  
  // ─────────────────────────────────────────────────────────────
  // LIFECYCLE
  // ─────────────────────────────────────────────────────────────
  
  /** Initialize provider with configuration */
  initialize(config: ProviderConfig): Promise<void>;
  
  /** Graceful shutdown */
  shutdown(): Promise<void>;
  
  /** Health check */
  healthCheck(): Promise<HealthCheckResult>;
  
  // ─────────────────────────────────────────────────────────────
  // CORE OPERATIONS
  // ─────────────────────────────────────────────────────────────
  
  /** Add a memory entry */
  add(input: AddMemoryInput): Promise<AddMemoryOutput>;
  
  /** Search memories semantically */
  search(input: SearchMemoryInput): Promise<SearchMemoryOutput>;
  
  /** Get a specific memory */
  get(input: GetMemoryInput): Promise<GetMemoryOutput>;
  
  /** Update a memory */
  update(input: UpdateMemoryInput): Promise<UpdateMemoryOutput>;
  
  /** Delete a memory */
  delete(input: DeleteMemoryInput): Promise<DeleteMemoryOutput>;
  
  /** List memories with pagination */
  list(input: ListMemoriesInput): Promise<ListMemoriesOutput>;
  
  // ─────────────────────────────────────────────────────────────
  // EMBEDDING OPERATIONS
  // ─────────────────────────────────────────────────────────────
  
  /** Generate embedding for content */
  generateEmbedding(content: string): Promise<number[]>;
  
  // ─────────────────────────────────────────────────────────────
  // OPTIONAL BULK OPERATIONS
  // ─────────────────────────────────────────────────────────────
  
  /** Bulk add memories (optional) */
  bulkAdd?(inputs: AddMemoryInput[]): Promise<AddMemoryOutput[]>;
  
  /** Bulk delete memories (optional) */
  bulkDelete?(ids: string[]): Promise<BulkDeleteResult>;
  
  // ─────────────────────────────────────────────────────────────
  // OPTIONAL ADVANCED OPERATIONS
  // ─────────────────────────────────────────────────────────────
  
  /** Export all memories (optional) */
  export?(): Promise<ExportResult>;
  
  /** Import memories (optional) */
  import?(data: ImportData): Promise<ImportResult>;
  
  /** Clear all memories in a layer (optional) */
  clearLayer?(layer: MemoryLayer, identifiers: LayerIdentifiers): Promise<void>;
}

interface ProviderCapabilities {
  /** Supports vector similarity search */
  vectorSearch: boolean;
  
  /** Supports metadata filtering */
  metadataFiltering: boolean;
  
  /** Supports bulk operations */
  bulkOperations: boolean;
  
  /** Supports export/import */
  dataPortability: boolean;
  
  /** Supports real-time updates */
  realTimeUpdates: boolean;
  
  /** Maximum content length (characters) */
  maxContentLength: number;
  
  /** Maximum metadata size (bytes) */
  maxMetadataSize: number;
  
  /** Embedding dimensions */
  embeddingDimensions: number;
  
  /** Supported distance metrics */
  distanceMetrics: ('cosine' | 'euclidean' | 'dot_product')[];
}

interface ProviderConfig {
  /** API endpoint URL */
  endpoint?: string;
  
  /** API key or token */
  apiKey?: string;
  
  /** Connection timeout (ms) */
  timeout?: number;
  
  /** Retry configuration */
  retry?: RetryConfig;
  
  /** Provider-specific options */
  options?: Record<string, unknown>;
}
```

### Provider Adapter Examples

#### Mem0 Provider Adapter

```typescript
class Mem0ProviderAdapter implements MemoryProviderAdapter {
  readonly id = 'mem0';
  readonly name = 'Mem0';
  readonly version = '1.0.0';
  readonly coreCompatibility = '>=0.1.0';
  
  readonly capabilities: ProviderCapabilities = {
    vectorSearch: true,
    metadataFiltering: true,
    bulkOperations: true,
    dataPortability: true,
    realTimeUpdates: false,
    maxContentLength: 100000,
    maxMetadataSize: 65536,
    embeddingDimensions: 1536,
    distanceMetrics: ['cosine']
  };
  
  private client: Mem0Client | null = null;
  
  async initialize(config: ProviderConfig): Promise<void> {
    this.client = new Mem0Client({
      apiKey: config.apiKey,
      baseUrl: config.endpoint ?? 'https://api.mem0.ai',
      timeout: config.timeout ?? 30000
    });
    
    // Verify connection
    await this.healthCheck();
  }
  
  async shutdown(): Promise<void> {
    this.client = null;
  }
  
  async healthCheck(): Promise<HealthCheckResult> {
    const start = Date.now();
    try {
      await this.client!.ping();
      return {
        status: 'healthy',
        latencyMs: Date.now() - start
      };
    } catch (error) {
      return {
        status: 'unhealthy',
        latencyMs: Date.now() - start,
        error: error.message
      };
    }
  }
  
  async add(input: AddMemoryInput): Promise<AddMemoryOutput> {
    const response = await this.client!.add({
      messages: [{ role: 'user', content: input.content }],
      user_id: this.buildUserId(input.layer, input.identifiers),
      metadata: input.metadata
    });
    
    return {
      memory: this.mapToMemoryEntry(response, input),
      embeddingGenerated: true
    };
  }
  
  async search(input: SearchMemoryInput): Promise<SearchMemoryOutput> {
    const results: MemorySearchResult[] = [];
    
    for (const layer of input.layers ?? ['user']) {
      const userId = this.buildUserId(layer, input.identifiers);
      
      const response = await this.client!.search({
        query: input.query,
        user_id: userId,
        limit: input.limit ?? 10,
        threshold: input.threshold ?? 0.7
      });
      
      for (const item of response.results) {
        results.push({
          memory: this.mapToMemoryEntry(item, { layer, identifiers: input.identifiers }),
          score: item.score,
          layer
        });
      }
    }
    
    return {
      results: this.sortByPrecedence(results),
      totalCount: results.length,
      searchedLayers: input.layers ?? ['user']
    };
  }
  
  // ... other method implementations
  
  private buildUserId(layer: MemoryLayer, ids: LayerIdentifiers): string {
    switch (layer) {
      case 'agent': return `agent:${ids.agentId}:${ids.userId}`;
      case 'user': return `user:${ids.userId}`;
      case 'session': return `session:${ids.userId}:${ids.sessionId}`;
      case 'project': return `project:${ids.projectId}`;
      case 'team': return `team:${ids.teamId}`;
      case 'org': return `org:${ids.orgId}`;
      case 'tenant': return `tenant:${ids.companyId}`;
    }
  }
}
```

#### Chroma Provider Adapter

```typescript
class ChromaProviderAdapter implements MemoryProviderAdapter {
  readonly id = 'chroma';
  readonly name = 'Chroma';
  readonly version = '1.0.0';
  readonly coreCompatibility = '>=0.1.0';
  
  readonly capabilities: ProviderCapabilities = {
    vectorSearch: true,
    metadataFiltering: true,
    bulkOperations: true,
    dataPortability: true,
    realTimeUpdates: false,
    maxContentLength: 50000,
    maxMetadataSize: 32768,
    embeddingDimensions: 384, // default, configurable
    distanceMetrics: ['cosine', 'euclidean', 'dot_product']
  };
  
  private client: ChromaClient | null = null;
  private collections: Map<string, Collection> = new Map();
  
  async initialize(config: ProviderConfig): Promise<void> {
    this.client = new ChromaClient({
      path: config.endpoint ?? 'http://localhost:8000'
    });
  }
  
  async add(input: AddMemoryInput): Promise<AddMemoryOutput> {
    const collection = await this.getOrCreateCollection(input.layer, input.identifiers);
    
    const id = crypto.randomUUID();
    await collection.add({
      ids: [id],
      documents: [input.content],
      metadatas: [{ ...input.metadata, layer: input.layer }]
    });
    
    return {
      memory: {
        id,
        content: input.content,
        layer: input.layer,
        identifiers: input.identifiers,
        metadata: input.metadata ?? {},
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString()
      },
      embeddingGenerated: true
    };
  }
  
  async search(input: SearchMemoryInput): Promise<SearchMemoryOutput> {
    const results: MemorySearchResult[] = [];
    
    for (const layer of input.layers ?? ['user']) {
      try {
        const collection = await this.getCollection(layer, input.identifiers);
        if (!collection) continue;
        
        const response = await collection.query({
          queryTexts: [input.query],
          nResults: input.limit ?? 10,
          where: input.filter ? this.buildWhereClause(input.filter) : undefined
        });
        
        for (let i = 0; i < response.ids[0].length; i++) {
          results.push({
            memory: {
              id: response.ids[0][i],
              content: response.documents[0][i],
              layer,
              identifiers: input.identifiers,
              metadata: response.metadatas[0][i] as MemoryMetadata,
              createdAt: '', // Chroma doesn't store timestamps by default
              updatedAt: ''
            },
            score: 1 - (response.distances?.[0][i] ?? 0), // Convert distance to similarity
            layer
          });
        }
      } catch (error) {
        // Collection doesn't exist - skip
      }
    }
    
    return {
      results: this.sortByPrecedence(results),
      totalCount: results.length,
      searchedLayers: input.layers ?? ['user']
    };
  }
  
  private async getOrCreateCollection(
    layer: MemoryLayer, 
    ids: LayerIdentifiers
  ): Promise<Collection> {
    const name = this.buildCollectionName(layer, ids);
    
    if (!this.collections.has(name)) {
      const collection = await this.client!.getOrCreateCollection({ name });
      this.collections.set(name, collection);
    }
    
    return this.collections.get(name)!;
  }
  
  private buildCollectionName(layer: MemoryLayer, ids: LayerIdentifiers): string {
    // Chroma collection names must be 3-63 chars, alphanumeric with underscores
    const parts = [layer];
    switch (layer) {
      case 'agent': parts.push(ids.agentId!, ids.userId!); break;
      case 'user': parts.push(ids.userId!); break;
      case 'session': parts.push(ids.userId!, ids.sessionId!); break;
      case 'project': parts.push(ids.projectId!); break;
      case 'team': parts.push(ids.teamId!); break;
      case 'org': parts.push(ids.orgId!); break;
      case 'tenant': parts.push(ids.companyId!); break;
    }
    return parts.join('_').substring(0, 63);
  }
}
```

---

## Ecosystem Adapters

### Ecosystem Adapter Interface

```typescript
/**
 * Interface for AI agent framework integration.
 * Implement this to add support for a new ecosystem.
 */
interface EcosystemAdapter {
  // ─────────────────────────────────────────────────────────────
  // METADATA
  // ─────────────────────────────────────────────────────────────
  
  /** Ecosystem identifier (e.g., "langchain", "autogen", "opencode") */
  readonly id: string;
  
  /** Human-readable name */
  readonly name: string;
  
  /** Adapter version */
  readonly version: string;
  
  /** Compatible ecosystem versions (semver range) */
  readonly ecosystemCompatibility: string;
  
  /** Compatible core versions (semver range) */
  readonly coreCompatibility: string;
  
  // ─────────────────────────────────────────────────────────────
  // LIFECYCLE
  // ─────────────────────────────────────────────────────────────
  
  /** Initialize adapter with core instance */
  initialize(core: MemoryKnowledgeCore, config: EcosystemConfig): Promise<void>;
  
  /** Graceful shutdown */
  shutdown(): Promise<void>;
  
  // ─────────────────────────────────────────────────────────────
  // TOOL GENERATION
  // ─────────────────────────────────────────────────────────────
  
  /** Generate ecosystem-native tools for memory operations */
  getMemoryTools(): EcosystemTool[];
  
  /** Generate ecosystem-native tools for knowledge operations */
  getKnowledgeTools(): EcosystemTool[];
  
  // ─────────────────────────────────────────────────────────────
  // CONTEXT INJECTION
  // ─────────────────────────────────────────────────────────────
  
  /** Get context to inject at session start */
  getSessionContext(identifiers: LayerIdentifiers): Promise<SessionContext>;
  
  /** Get relevant memories for current context */
  getRelevantMemories(query: string, identifiers: LayerIdentifiers): Promise<MemoryEntry[]>;
  
  /** Get applicable constraints for current context */
  getActiveConstraints(identifiers: LayerIdentifiers): Promise<Constraint[]>;
  
  // ─────────────────────────────────────────────────────────────
  // EVENT HOOKS
  // ─────────────────────────────────────────────────────────────
  
  /** Called when agent starts a session */
  onSessionStart?(sessionId: string, identifiers: LayerIdentifiers): Promise<void>;
  
  /** Called when agent ends a session */
  onSessionEnd?(sessionId: string): Promise<void>;
  
  /** Called when agent receives a message */
  onMessage?(message: AgentMessage): Promise<void>;
  
  /** Called when agent uses a tool */
  onToolUse?(toolName: string, input: unknown, output: unknown): Promise<void>;
}

interface EcosystemConfig {
  /** Ecosystem-specific configuration */
  [key: string]: unknown;
}

interface EcosystemTool {
  /** Tool name */
  name: string;
  
  /** Tool description */
  description: string;
  
  /** Input schema (JSON Schema) */
  inputSchema: object;
  
  /** The tool function */
  execute: (input: unknown) => Promise<unknown>;
}

interface SessionContext {
  /** Memories to inject */
  memories: MemoryEntry[];
  
  /** Active constraints */
  constraints: Constraint[];
  
  /** System prompt additions */
  systemPromptAdditions?: string;
}
```

### Ecosystem Adapter Examples

#### LangChain Adapter

```typescript
import { DynamicStructuredTool } from '@langchain/core/tools';
import { z } from 'zod';

class LangChainEcosystemAdapter implements EcosystemAdapter {
  readonly id = 'langchain';
  readonly name = 'LangChain';
  readonly version = '1.0.0';
  readonly ecosystemCompatibility = '>=0.1.0';
  readonly coreCompatibility = '>=0.1.0';
  
  private core: MemoryKnowledgeCore | null = null;
  
  async initialize(core: MemoryKnowledgeCore, config: EcosystemConfig): Promise<void> {
    this.core = core;
  }
  
  async shutdown(): Promise<void> {
    this.core = null;
  }
  
  getMemoryTools(): EcosystemTool[] {
    return [
      this.createMemoryAddTool(),
      this.createMemorySearchTool()
    ];
  }
  
  private createMemoryAddTool(): EcosystemTool {
    const tool = new DynamicStructuredTool({
      name: 'memory_add',
      description: 'Store a new memory for future reference',
      schema: z.object({
        content: z.string().describe('The content to remember'),
        layer: z.enum(['agent', 'user', 'session', 'project', 'team', 'org', 'tenant'])
          .optional()
          .default('user')
          .describe('Memory scope'),
        tags: z.array(z.string()).optional().describe('Tags for categorization')
      }),
      func: async (input) => {
        const result = await this.core!.memory.add({
          content: input.content,
          layer: input.layer,
          identifiers: this.getCurrentIdentifiers(),
          metadata: { tags: input.tags }
        });
        return `Memory stored with ID: ${result.memory.id}`;
      }
    });
    
    return {
      name: tool.name,
      description: tool.description,
      inputSchema: tool.schema,
      execute: tool.func
    };
  }
  
  private createMemorySearchTool(): EcosystemTool {
    const tool = new DynamicStructuredTool({
      name: 'memory_search',
      description: 'Search memories for relevant information',
      schema: z.object({
        query: z.string().describe('Search query'),
        layers: z.array(z.enum(['agent', 'user', 'session', 'project', 'team', 'org', 'tenant']))
          .optional()
          .describe('Layers to search'),
        limit: z.number().optional().default(5).describe('Maximum results')
      }),
      func: async (input) => {
        const result = await this.core!.memory.search({
          query: input.query,
          layers: input.layers,
          identifiers: this.getCurrentIdentifiers(),
          limit: input.limit
        });
        
        return result.results
          .map(r => `[${r.layer}] ${r.memory.content}`)
          .join('\n\n');
      }
    });
    
    return {
      name: tool.name,
      description: tool.description,
      inputSchema: tool.schema,
      execute: tool.func
    };
  }
  
  getKnowledgeTools(): EcosystemTool[] {
    return [
      this.createKnowledgeQueryTool(),
      this.createKnowledgeCheckTool()
    ];
  }
  
  async getSessionContext(identifiers: LayerIdentifiers): Promise<SessionContext> {
    // Fetch relevant memories
    const memories = await this.core!.memory.search({
      query: 'session context user preferences project conventions',
      identifiers,
      limit: 10
    });
    
    // Fetch active constraints
    const knowledge = await this.core!.knowledge.query({
      status: 'accepted',
      identifiers
    });
    
    const constraints = knowledge.items
      .flatMap(item => item.constraints ?? [])
      .filter(c => c.severity === 'block' || c.severity === 'warn');
    
    return {
      memories: memories.results.map(r => r.memory),
      constraints,
      systemPromptAdditions: this.buildSystemPrompt(constraints)
    };
  }
  
  private buildSystemPrompt(constraints: Constraint[]): string {
    if (constraints.length === 0) return '';
    
    const lines = ['## Active Constraints', ''];
    for (const c of constraints) {
      lines.push(`- [${c.severity.toUpperCase()}] ${c.message ?? formatConstraint(c)}`);
    }
    return lines.join('\n');
  }
  
  // Hook implementations
  async onSessionStart(sessionId: string, identifiers: LayerIdentifiers): Promise<void> {
    // Trigger incremental sync
    await this.core!.sync.incrementalSync({ identifiers });
  }
  
  async onToolUse(toolName: string, input: unknown, output: unknown): Promise<void> {
    // Optionally log tool usage to memory
    if (this.shouldLogToolUsage(toolName)) {
      await this.core!.memory.add({
        content: `Used tool ${toolName} with result: ${JSON.stringify(output).substring(0, 200)}`,
        layer: 'session',
        identifiers: this.getCurrentIdentifiers(),
        metadata: {
          source: { type: 'tool_result', reference: toolName }
        }
      });
    }
  }
}
```

#### OpenCode Adapter

```typescript
class OpenCodeEcosystemAdapter implements EcosystemAdapter {
  readonly id = 'opencode';
  readonly name = 'OpenCode';
  readonly version = '1.0.0';
  readonly ecosystemCompatibility = '>=1.0.150';
  readonly coreCompatibility = '>=0.1.0';
  
  private core: MemoryKnowledgeCore | null = null;
  
  async initialize(core: MemoryKnowledgeCore, config: EcosystemConfig): Promise<void> {
    this.core = core;
  }
  
  getMemoryTools(): EcosystemTool[] {
    return [
      {
        name: 'memory_add',
        description: 'Store information in long-term memory for future sessions',
        inputSchema: {
          type: 'object',
          properties: {
            content: { type: 'string', description: 'What to remember' },
            layer: { 
              type: 'string', 
              enum: ['agent', 'user', 'session', 'project', 'team', 'org', 'tenant'],
              default: 'user'
            },
            tags: { type: 'array', items: { type: 'string' } }
          },
          required: ['content']
        },
        execute: async (input: any) => {
          return this.core!.memory.add({
            content: input.content,
            layer: input.layer ?? 'user',
            identifiers: this.getCurrentIdentifiers(),
            metadata: { tags: input.tags }
          });
        }
      },
      {
        name: 'memory_search',
        description: 'Search memories for relevant past information',
        inputSchema: {
          type: 'object',
          properties: {
            query: { type: 'string', description: 'Search query' },
            layers: { 
              type: 'array', 
              items: { type: 'string' }
            },
            limit: { type: 'number', default: 10 }
          },
          required: ['query']
        },
        execute: async (input: any) => {
          return this.core!.memory.search({
            query: input.query,
            layers: input.layers,
            identifiers: this.getCurrentIdentifiers(),
            limit: input.limit ?? 10
          });
        }
      }
    ];
  }
  
  getKnowledgeTools(): EcosystemTool[] {
    return [
      {
        name: 'knowledge_query',
        description: 'Search organizational knowledge (ADRs, policies, patterns)',
        inputSchema: {
          type: 'object',
          properties: {
            query: { type: 'string' },
            type: { type: 'string', enum: ['adr', 'policy', 'pattern', 'spec'] },
            tags: { type: 'array', items: { type: 'string' } }
          }
        },
        execute: async (input: any) => {
          return this.core!.knowledge.query({
            query: input.query,
            type: input.type,
            tags: input.tags,
            identifiers: this.getCurrentIdentifiers()
          });
        }
      },
      {
        name: 'knowledge_check',
        description: 'Check if current action violates any constraints',
        inputSchema: {
          type: 'object',
          properties: {
            files: {
              type: 'array',
              items: {
                type: 'object',
                properties: {
                  path: { type: 'string' },
                  content: { type: 'string' }
                }
              }
            },
            dependencies: {
              type: 'array',
              items: {
                type: 'object',
                properties: {
                  name: { type: 'string' },
                  version: { type: 'string' }
                }
              }
            }
          }
        },
        execute: async (input: any) => {
          return this.core!.knowledge.checkConstraints({
            files: input.files,
            dependencies: input.dependencies,
            identifiers: this.getCurrentIdentifiers()
          });
        }
      }
    ];
  }
  
  async getSessionContext(identifiers: LayerIdentifiers): Promise<SessionContext> {
    // OpenCode-specific context enrichment
    const memories = await this.core!.memory.search({
      query: 'coding preferences project patterns user style',
      identifiers,
      limit: 15,
      layers: ['agent', 'user', 'project', 'team', 'org', 'tenant']
    });
    
    const knowledge = await this.core!.knowledge.query({
      status: 'accepted',
      identifiers,
      limit: 20
    });
    
    const constraints = knowledge.items
      .flatMap(item => item.constraints ?? []);
    
    return {
      memories: memories.results.map(r => r.memory),
      constraints,
      systemPromptAdditions: this.buildOpenCodeSystemPrompt(memories.results, constraints)
    };
  }
  
  private buildOpenCodeSystemPrompt(
    memories: MemorySearchResult[], 
    constraints: Constraint[]
  ): string {
    const sections: string[] = [];
    
    // Memory context
    if (memories.length > 0) {
      sections.push('## Relevant Context from Memory\n');
      for (const m of memories.slice(0, 5)) {
        sections.push(`- ${m.memory.content}`);
      }
    }
    
    // Blocking constraints
    const blocking = constraints.filter(c => c.severity === 'block');
    if (blocking.length > 0) {
      sections.push('\n## BLOCKING Constraints (Must Follow)\n');
      for (const c of blocking) {
        sections.push(`- ${c.message ?? `${c.operator}: ${c.pattern}`}`);
      }
    }
    
    // Warning constraints
    const warnings = constraints.filter(c => c.severity === 'warn');
    if (warnings.length > 0) {
      sections.push('\n## Warnings (Should Follow)\n');
      for (const c of warnings) {
        sections.push(`- ${c.message ?? `${c.operator}: ${c.pattern}`}`);
      }
    }
    
    return sections.join('\n');
  }
}
```

---

## Adapter Registration

### Registry Interface

```typescript
interface AdapterRegistry {
  // Provider adapters
  registerProvider(adapter: MemoryProviderAdapter): void;
  getProvider(id: string): MemoryProviderAdapter | undefined;
  listProviders(): MemoryProviderAdapter[];
  
  // Ecosystem adapters
  registerEcosystem(adapter: EcosystemAdapter): void;
  getEcosystem(id: string): EcosystemAdapter | undefined;
  listEcosystems(): EcosystemAdapter[];
  
  // Auto-discovery
  discoverAdapters(searchPaths?: string[]): Promise<void>;
}
```

### Registration Example

```typescript
// Manual registration
const registry = new AdapterRegistry();

registry.registerProvider(new Mem0ProviderAdapter());
registry.registerProvider(new ChromaProviderAdapter());
registry.registerProvider(new LettaProviderAdapter());

registry.registerEcosystem(new LangChainEcosystemAdapter());
registry.registerEcosystem(new OpenCodeEcosystemAdapter());

// Auto-discovery
await registry.discoverAdapters([
  './node_modules/@memory-knowledge/adapter-*',
  './custom-adapters'
]);
```

### Adapter Package Convention

```
@memory-knowledge/adapter-mem0/
├── package.json
│   {
│     "name": "@memory-knowledge/adapter-mem0",
│     "memory-knowledge": {
│       "type": "provider",
│       "id": "mem0"
│     }
│   }
├── index.ts
└── README.md
```

---

## Adapter Lifecycle

### Lifecycle Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│     UNINITIALIZED                                               │
│           │                                                      │
│           │ register()                                          │
│           ▼                                                      │
│     REGISTERED                                                   │
│           │                                                      │
│           │ initialize(config)                                  │
│           ▼                                                      │
│     INITIALIZING                                                 │
│           │                                                      │
│           ├─────────────────► ERROR (on failure)                │
│           │                      │                               │
│           │                      │ retry()                      │
│           │                      ▼                               │
│           │                   INITIALIZING                       │
│           │                                                      │
│           │ (on success)                                        │
│           ▼                                                      │
│     READY ◄───────────────────┐                                 │
│           │                    │                                 │
│           │ healthCheck()      │ reconnect()                    │
│           │                    │                                 │
│           ▼                    │                                 │
│     HEALTHY / DEGRADED ────────┘                                │
│           │                                                      │
│           │ shutdown()                                          │
│           ▼                                                      │
│     SHUTTING_DOWN                                                │
│           │                                                      │
│           ▼                                                      │
│     SHUTDOWN                                                     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### State Management

```typescript
type AdapterState = 
  | 'uninitialized'
  | 'registered'
  | 'initializing'
  | 'ready'
  | 'healthy'
  | 'degraded'
  | 'error'
  | 'shutting_down'
  | 'shutdown';

interface AdapterStateManager {
  getState(): AdapterState;
  
  onStateChange(callback: (state: AdapterState) => void): void;
  
  waitForReady(timeoutMs?: number): Promise<void>;
}
```

---

## Testing Adapters

### Test Interface

```typescript
/**
 * Compliance test suite for adapters.
 */
interface AdapterTestSuite {
  /** Run all compliance tests */
  runAll(): Promise<TestResults>;
  
  /** Test basic CRUD operations */
  testCrud(): Promise<TestResult>;
  
  /** Test search functionality */
  testSearch(): Promise<TestResult>;
  
  /** Test layer isolation */
  testLayerIsolation(): Promise<TestResult>;
  
  /** Test error handling */
  testErrorHandling(): Promise<TestResult>;
  
  /** Test concurrent operations */
  testConcurrency(): Promise<TestResult>;
}
```

### Mock Adapter for Testing

```typescript
/**
 * In-memory mock adapter for testing.
 */
class MockProviderAdapter implements MemoryProviderAdapter {
  readonly id = 'mock';
  readonly name = 'Mock (In-Memory)';
  readonly version = '1.0.0';
  readonly coreCompatibility = '*';
  
  readonly capabilities: ProviderCapabilities = {
    vectorSearch: true,
    metadataFiltering: true,
    bulkOperations: true,
    dataPortability: true,
    realTimeUpdates: true,
    maxContentLength: Infinity,
    maxMetadataSize: Infinity,
    embeddingDimensions: 384,
    distanceMetrics: ['cosine']
  };
  
  private memories: Map<string, MemoryEntry> = new Map();
  
  async initialize(): Promise<void> {}
  
  async shutdown(): Promise<void> {
    this.memories.clear();
  }
  
  async healthCheck(): Promise<HealthCheckResult> {
    return { status: 'healthy', latencyMs: 0 };
  }
  
  async add(input: AddMemoryInput): Promise<AddMemoryOutput> {
    const id = crypto.randomUUID();
    const memory: MemoryEntry = {
      id,
      content: input.content,
      layer: input.layer,
      identifiers: input.identifiers,
      metadata: input.metadata ?? {},
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      embedding: await this.generateEmbedding(input.content)
    };
    
    this.memories.set(id, memory);
    
    return { memory, embeddingGenerated: true };
  }
  
  async search(input: SearchMemoryInput): Promise<SearchMemoryOutput> {
    const queryEmbedding = await this.generateEmbedding(input.query);
    const results: MemorySearchResult[] = [];
    
    for (const memory of this.memories.values()) {
      if (!input.layers?.includes(memory.layer)) continue;
      
      const score = this.cosineSimilarity(queryEmbedding, memory.embedding!);
      if (score >= (input.threshold ?? 0.7)) {
        results.push({ memory, score, layer: memory.layer });
      }
    }
    
    return {
      results: results.sort((a, b) => b.score - a.score).slice(0, input.limit ?? 10),
      totalCount: results.length,
      searchedLayers: input.layers ?? []
    };
  }
  
  async generateEmbedding(content: string): Promise<number[]> {
    // Simple hash-based mock embedding
    const embedding = new Array(384).fill(0);
    for (let i = 0; i < content.length; i++) {
      embedding[i % 384] += content.charCodeAt(i) / 1000;
    }
    return this.normalize(embedding);
  }
  
  private cosineSimilarity(a: number[], b: number[]): number {
    let dot = 0, normA = 0, normB = 0;
    for (let i = 0; i < a.length; i++) {
      dot += a[i] * b[i];
      normA += a[i] * a[i];
      normB += b[i] * b[i];
    }
    return dot / (Math.sqrt(normA) * Math.sqrt(normB));
  }
  
  private normalize(v: number[]): number[] {
    const norm = Math.sqrt(v.reduce((sum, x) => sum + x * x, 0));
    return v.map(x => x / norm);
  }
  
  // ... other methods
}
```

---

## Reference Implementations

### Provider Adapters

| Provider | Package | Status |
|----------|---------|--------|
| Mem0 | `@memory-knowledge/adapter-mem0` | Reference |
| Letta | `@memory-knowledge/adapter-letta` | Planned |
| Chroma | `@memory-knowledge/adapter-chroma` | Planned |
| Pinecone | `@memory-knowledge/adapter-pinecone` | Planned |
| Qdrant | `@memory-knowledge/adapter-qdrant` | Planned |
| PostgreSQL | `@memory-knowledge/adapter-postgres` | Planned |

### Ecosystem Adapters

| Ecosystem | Package | Status |
|-----------|---------|--------|
| OpenCode | `@memory-knowledge/adapter-opencode` | Reference |
| LangChain | `@memory-knowledge/adapter-langchain` | Planned |
| AutoGen | `@memory-knowledge/adapter-autogen` | Planned |
| CrewAI | `@memory-knowledge/adapter-crewai` | Planned |

---

**Next**: [06-tool-interface.md](./06-tool-interface.md) - Tool Interface Specification
