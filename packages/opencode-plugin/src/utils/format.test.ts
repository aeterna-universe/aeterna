import { describe, it, expect } from "vitest";
import {
  formatKnowledgeContext,
  formatMemoryContext,
  formatNote,
  formatHindsightResults,
  formatAssembledContext,
  formatGraphResults,
} from "./format.js";
import type {
  KnowledgeSearchResult,
  MemorySearchResult,
  GeneratedNote,
  HindsightMatch,
  AssembledContext,
  GraphQueryResult,
} from "../types.js";

describe("formatKnowledgeContext", () => {
  it("returns empty string for empty array", () => {
    expect(formatKnowledgeContext([])).toBe("");
  });

  it("filters out results with score <= 0.8", () => {
    const results: KnowledgeSearchResult[] = [
      {
        knowledge: {
          id: "k1",
          type: "adr",
          title: "Low Score",
          content: "Should be filtered",
          scope: "project",
          tags: [],
          createdAt: "",
          updatedAt: "",
          status: "approved",
        },
        score: 0.5,
      },
    ];
    expect(formatKnowledgeContext(results)).toBe("📚 Relevant Knowledge:\n\n");
  });

  it("formats high-score results with title and type", () => {
    const results: KnowledgeSearchResult[] = [
      {
        knowledge: {
          id: "k1",
          type: "pattern",
          title: "Error Handling",
          content: "Always use Result types for operations that can fail in production code to ensure proper error propagation",
          scope: "team",
          tags: ["rust"],
          createdAt: "",
          updatedAt: "",
          status: "approved",
        },
        score: 0.95,
        summary: "Use Result types",
      },
    ];
    const formatted = formatKnowledgeContext(results);
    expect(formatted).toContain("📚 Relevant Knowledge:");
    expect(formatted).toContain("Error Handling");
    expect(formatted).toContain("PATTERN");
    expect(formatted).toContain("95%");
    expect(formatted).toContain("Use Result types");
  });
});

describe("formatMemoryContext", () => {
  it("returns empty string for empty array", () => {
    expect(formatMemoryContext([])).toBe("");
  });

  it("filters out results with score <= 0.75", () => {
    const results: MemorySearchResult[] = [
      {
        memory: {
          id: "m1",
          content: "Low relevance",
          layer: "session",
          importance: 0.5,
          tags: [],
          createdAt: "",
          updatedAt: "",
        },
        score: 0.3,
      },
    ];
    expect(formatMemoryContext(results)).toBe("💭 Relevant Memories:\n\n");
  });

  it("formats high-score memories with layer and importance", () => {
    const results: MemorySearchResult[] = [
      {
        memory: {
          id: "m1",
          content: "Use PostgreSQL for all new services per ADR-042, this is an important decision",
          layer: "project",
          importance: 0.9,
          tags: ["db"],
          createdAt: "",
          updatedAt: "",
        },
        score: 0.88,
      },
    ];
    const formatted = formatMemoryContext(results);
    expect(formatted).toContain("💭 Relevant Memories:");
    expect(formatted).toContain("project");
    expect(formatted).toContain("0.90");
  });
});

describe("formatNote", () => {
  it("returns empty string for empty array", () => {
    expect(formatNote([])).toBe("");
  });

  it("formats notes with title, content, and tags", () => {
    const notes: GeneratedNote[] = [
      {
        id: "n1",
        title: "Database Migration",
        content: "Migrated from MySQL to PostgreSQL",
        tags: ["migration", "db"],
        createdAt: "2025-01-01",
        trajectoryCount: 5,
      },
    ];
    const formatted = formatNote(notes);
    expect(formatted).toContain("Database Migration");
    expect(formatted).toContain("Migrated from MySQL to PostgreSQL");
    expect(formatted).toContain("#migration");
    expect(formatted).toContain("#db");
  });

  it("separates multiple notes with dividers", () => {
    const notes: GeneratedNote[] = [
      { id: "n1", title: "Note 1", content: "Content 1", tags: [], createdAt: "", trajectoryCount: 1 },
      { id: "n2", title: "Note 2", content: "Content 2", tags: [], createdAt: "", trajectoryCount: 2 },
    ];
    const formatted = formatNote(notes);
    expect(formatted).toContain("---");
    expect(formatted).toContain("Note 1");
    expect(formatted).toContain("Note 2");
  });
});

describe("formatHindsightResults", () => {
  it("returns fallback message for empty array", () => {
    expect(formatHindsightResults([])).toBe("No error patterns found.");
  });

  it("formats matches with error signature and resolution", () => {
    const matches: HindsightMatch[] = [
      {
        note: {
          id: "h1",
          errorSignature: "ECONNREFUSED",
          errorType: "network",
          resolution: "Check if the service is running",
          successRate: 0.85,
          occurrences: 12,
          lastSeen: "2025-01-01",
          tags: ["network"],
        },
        score: 0.9,
        matchedPatterns: ["connection", "refused"],
      },
    ];
    const formatted = formatHindsightResults(matches);
    expect(formatted).toContain("ECONNREFUSED");
    expect(formatted).toContain("network");
    expect(formatted).toContain("Check if the service is running");
    expect(formatted).toContain("12");
    expect(formatted).toContain("85.0%");
    expect(formatted).toContain("connection, refused");
  });
});

describe("formatAssembledContext", () => {
  it("formats context with token usage and layer breakdown", () => {
    const context: AssembledContext = {
      context: "assembled context here",
      tokensUsed: 4000,
      tokenBudget: 8000,
      layerBreakdown: { agent: 500, user: 500, session: 1000, project: 1000, team: 500, org: 300, tenant: 200 },
      truncated: false,
      sources: [
        { id: "s1", layer: "project", relevance: 0.95 },
      ],
    };
    const formatted = formatAssembledContext(context);
    expect(formatted).toContain("4000/8000");
    expect(formatted).toContain("50.0%");
    expect(formatted).toContain("No");
    expect(formatted).toContain("project");
    expect(formatted).toContain("0.95");
  });

  it("indicates truncation when context was truncated", () => {
    const context: AssembledContext = {
      context: "truncated",
      tokensUsed: 8000,
      tokenBudget: 8000,
      layerBreakdown: { agent: 0, user: 0, session: 0, project: 4000, team: 4000, org: 0, tenant: 0 },
      truncated: true,
      sources: [],
    };
    const formatted = formatAssembledContext(context);
    expect(formatted).toContain("Yes");
  });

  it("handles empty sources gracefully", () => {
    const context: AssembledContext = {
      context: "",
      tokensUsed: 0,
      tokenBudget: 8000,
      layerBreakdown: { agent: 0, user: 0, session: 0, project: 0, team: 0, org: 0, tenant: 0 },
      truncated: false,
      sources: [],
    };
    const formatted = formatAssembledContext(context);
    expect(formatted).toContain("0/8000");
    expect(formatted).not.toContain("Top Sources");
  });
});

describe("formatGraphResults", () => {
  it("formats nodes and edges", () => {
    const result: GraphQueryResult = {
      nodes: [
        { id: "n1", label: "Auth Service", nodeType: "memory", properties: { type: "service" } },
        { id: "n2", label: "User DB", nodeType: "knowledge", properties: { type: "database" } },
      ],
      edges: [
        { source: "n1", target: "n2", relation: "uses", weight: 1.0 },
      ],
    };
    const formatted = formatGraphResults(result);
    expect(formatted).toContain("2 nodes");
    expect(formatted).toContain("1 relationships");
    expect(formatted).toContain("Auth Service");
    expect(formatted).toContain("User DB");
    expect(formatted).toContain("memory");
    expect(formatted).toContain("knowledge");
  });

  it("includes paths when present", () => {
    const result: GraphQueryResult = {
      nodes: [
        { id: "n1", label: "A", nodeType: "memory", properties: {} },
        { id: "n2", label: "B", nodeType: "memory", properties: {} },
      ],
      edges: [{ source: "n1", target: "n2", relation: "ref" }],
      paths: [
        { nodes: ["n1", "n2"], edges: ["e1"], totalWeight: 1.5 },
      ],
    };
    const formatted = formatGraphResults(result);
    expect(formatted).toContain("Path 1");
    expect(formatted).toContain("n1 → n2");
    expect(formatted).toContain("1.50");
  });

  it("handles empty graph result", () => {
    const result: GraphQueryResult = { nodes: [], edges: [] };
    const formatted = formatGraphResults(result);
    expect(formatted).toContain("0 nodes");
    expect(formatted).toContain("0 relationships");
  });
});
