import { tool, type ToolDefinition } from "@opencode-ai/plugin/tool";
import type { AeternaClient } from "../client.js";

const z = tool.schema;

export const createMemoryTools = (client: AeternaClient): Record<string, ToolDefinition> => ({
  aeterna_memory_add: tool({
    description: "Add a memory entry to Aeterna. Use this to capture learnings, solutions, or important context.",
    args: {
      content: z.string().describe("The content to remember"),
      layer: z.enum(["agent", "user", "session", "project", "team", "org", "tenant"] as const)
        .optional()
        .describe("Memory layer (default: working)"),
      tags: z.array(z.string()).optional()
        .describe("Tags for categorization"),
      importance: z.number().min(0).max(1).optional()
        .describe("Importance score 0-1 (default: auto-calculated)"),
      sessionId: z.string().optional()
        .describe("Session ID for context"),
      metadata: z.record(z.string(), z.unknown()).optional()
        .describe("Additional metadata"),
    },
    async execute(args, context) {
      const result = await client.memoryAdd({
        content: args.content,
        layer: args.layer ?? "session",
        tags: args.tags,
        importance: args.importance,
        sessionId: args.sessionId ?? context.sessionID,
        metadata: args.metadata as Record<string, unknown> | undefined,
      });
      return `Memory added: ${result.id} (layer: ${result.layer}, importance: ${result.importance.toFixed(2)})`;
    },
  }),

  aeterna_memory_search: tool({
    description: "Search memories for relevant context. Returns semantically similar memories.",
    args: {
      query: z.string().describe("Search query"),
      layers: z.array(z.enum(["agent", "user", "session", "project", "team", "org", "tenant"] as const)).optional()
        .describe("Layers to search (default: all)"),
      limit: z.number().min(1).max(20).optional()
        .describe("Max results (default: 5)"),
      threshold: z.number().min(0).max(1).optional()
        .describe("Similarity threshold (default: 0.7)"),
      sessionId: z.string().optional()
        .describe("Session ID for context"),
      tags: z.array(z.string()).optional()
        .describe("Filter by tags"),
    },
    async execute(args, context) {
      const results = await client.memorySearch({
        query: args.query,
        layers: args.layers,
        limit: args.limit ?? 5,
        threshold: args.threshold ?? 0.7,
        sessionId: args.sessionId ?? context.sessionID,
        tags: args.tags,
      });

      if (results.length === 0) {
        return `No memories found for query: "${args.query}"`;
      }

      const hasStaleResults = results.some((r) => r.memory.metadata?.stale === true);
      const warning = hasStaleResults
        ? "⚠️ Some results from local cache may be stale (last synced >10 min ago)\n\n"
        : "";

      return warning + results
        .map((r) => {
          const source =
            typeof r.memory.metadata?.source === "string" ? r.memory.metadata.source : "unknown";
          return `- [${r.memory.id}] (${source}) ${r.score.toFixed(2)}: ${r.memory.content.slice(0, 100)}...`;
        })
        .join("\n");
    },
  }),

  aeterna_memory_get: tool({
    description: "Retrieve a specific memory by ID",
    args: {
      memoryId: z.string().describe("Memory ID to retrieve"),
    },
    async execute(args, _context) {
      const memory = await client.memoryGet(args.memoryId);
      if (!memory) {
        return `Memory not found: ${args.memoryId}`;
      }
      return `[${memory.id}] ${memory.layer}: ${memory.content}`;
    },
  }),

  aeterna_memory_promote: tool({
    description: "Promote a memory to a higher layer (e.g., session -> project -> team)",
    args: {
      memoryId: z.string().describe("Memory ID to promote"),
      targetLayer: z.enum(["session", "project", "team", "org", "tenant"] as const)
        .describe("Target layer (must be higher than current)"),
      reason: z.string().optional()
        .describe("Reason for promotion"),
    },
    async execute(args, _context) {
      const result = await client.memoryPromote({
        memoryId: args.memoryId,
        targetLayer: args.targetLayer,
        reason: args.reason,
      });
      return `Memory promoted from ${result.layer} to ${args.targetLayer}`;
    },
  }),
});
