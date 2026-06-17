import { tool, type ToolDefinition } from "@opencode-ai/plugin/tool";
import type { AeternaClient } from "../client.js";

const z = tool.schema;

export const createCcaTools = (client: AeternaClient): Record<string, ToolDefinition> => ({
  aeterna_context_assemble: tool({
    description: "Assemble hierarchical context from memory layers using CCA Context Architect",
    args: {
      query: z.string().describe("Query or topic for context assembly"),
      tokenBudget: z.number().min(100).max(32000).optional()
        .describe("Maximum tokens to include (default: 8000)"),
      layers: z.array(z.enum(["agent", "user", "session", "project", "team", "org", "tenant"] as const)).optional()
        .describe("Memory layers to include (default: all)"),
      viewMode: z.enum(["AX", "UX", "DX"] as const).optional()
        .describe("View mode for AX/UX/DX separation (default: AX)"),
      includeKnowledge: z.boolean().optional()
        .describe("Include knowledge repository (default: true)"),
    },
    async execute(args, _context) {
      const result = await client.contextAssemble({
        query: args.query,
        tokenBudget: args.tokenBudget,
        layers: args.layers,
        viewMode: args.viewMode ?? "AX",
        includeKnowledge: args.includeKnowledge ?? true,
      });

      const pctUsed = ((result.tokensUsed / result.tokenBudget) * 100).toFixed(1);
      const layersSummary = Object.entries(result.layerBreakdown)
        .map(([layer, count]) => `  ${layer}: ${count} tokens (${((count / result.tokensUsed) * 100).toFixed(1)}%)`)
        .join("\n");

      return `Context assembled (${result.tokensUsed}/${result.tokenBudget} tokens, ${pctUsed}% of budget)\n\nLayer breakdown:\n${layersSummary}${result.truncated ? "\n⚠️ Context was truncated" : ""}`;
    },
  }),

  aeterna_note_capture: tool({
    description: "Capture a trajectory event for note distillation by Note-Taking Agent",
    args: {
      description: z.string().describe("Event description (tool used, error, decision, etc.)"),
      toolName: z.string().optional()
        .describe("Tool that was used"),
      success: z.boolean().optional()
        .describe("Was the operation successful?"),
      tags: z.array(z.string()).optional()
        .describe("Tags for categorization"),
    },
    async execute(args, _context) {
      const result = await client.noteCapture({
        description: args.description,
        toolName: args.toolName,
        success: args.success ?? true,
        tags: args.tags,
      });

      return `Note captured: ${result.id} (trajectory count: ${result.trajectoryCount}) - "${result.title}"`;
    },
  }),

  aeterna_hindsight_query: tool({
    description: "Query hindsight learning for error patterns and resolutions from past failures",
    args: {
      errorType: z.string().optional()
        .describe("Filter by error type (e.g., 'timeout', 'auth-failed')"),
      messagePattern: z.string().optional()
        .describe("Filter by message pattern"),
      contextPatterns: z.array(z.string()).optional()
        .describe("Filter by context patterns"),
      limit: z.number().min(1).max(50).optional()
        .describe("Max results (default: 10)"),
    },
    async execute(args, _context) {
      const results = await client.hindsightQuery({
        errorType: args.errorType,
        messagePattern: args.messagePattern,
        contextPatterns: args.contextPatterns,
        limit: args.limit ?? 10,
      });

      if (results.length === 0) {
        return "No hindsight notes found for this error pattern";
      }

      return results
        .map((r) => {
          const matchStr = r.matchedPatterns.length > 0 ? `matches: ${r.matchedPatterns.join(", ")}` : "";
          return `- [${r.note.id}] ${r.note.errorSignature} (success rate: ${r.note.successRate.toFixed(1)}): ${matchStr}`;
        })
        .join("\n");
    },
  }),

  aeterna_meta_loop_status: tool({
    description: "Get status of meta-agent build-test-improve loops",
    args: {
      loopId: z.string().optional()
        .describe("Specific loop ID to check"),
      includeDetails: z.boolean().optional()
        .describe("Include detailed phase information"),
    },
    async execute(args, _context) {
      const status = await client.metaLoopStatus(args.loopId);

      const phaseEmoji = {
        build: "🔨",
        test: "🧪",
        improve: "✨",
        idle: "💤",
        completed: "✅",
      };

      const details = args.includeDetails
        ? `\n\nCurrent phase: ${phaseEmoji[status.phase]} ${status.phase}\nIteration: ${status.iteration}/${status.maxIterations}\nQuality score: ${status.qualityScore?.toFixed(2) ?? "N/A"}\n\nImprovements:\n${status.improvements.length > 0 ? status.improvements.map((i) => `  - ${i}`).join("\n") : "  None"}\n\nErrors:\n${status.errors.length > 0 ? status.errors.map((e) => `  - ${e}`).join("\n") : "  None"}`
        : "";

      return `Meta-loop status: ${status.phase}\nLoop ID: ${status.loopId}${details}`;
    },
  }),
});
