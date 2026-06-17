import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const testState = vi.hoisted(() => {
  type MemoryLayer = "agent" | "user" | "session" | "project" | "team" | "org" | "tenant";
  type LocalOwnership = "local" | "cached";
  type SyncOperation = "upsert" | "delete";

  type MemoryRecord = {
    id: string;
    content: string;
    layer: MemoryLayer;
    ownership: LocalOwnership;
    embedding: Buffer | null;
    tags: string | null;
    metadata: string | null;
    importance_score: number;
    created_at: number;
    updated_at: number;
    synced_at: number | null;
    deleted_at: number | null;
  };

  type SyncQueueRecord = {
    id: number;
    memory_id: string;
    operation: SyncOperation;
    queued_at: number;
  };

  type CursorRecord = {
    cursor: string;
    updated_at: number;
  };

  type RunResult = { changes: number };

  const normalizeSql = (sql: string): string => sql.replace(/\s+/g, " ").trim().toLowerCase();

  const likeMatch = (content: string, pattern: string): boolean => {
    const needle = pattern.replace(/%/g, "").toLowerCase();
    return content.toLowerCase().includes(needle);
  };

  class FakeStatement {
    constructor(
      private readonly db: FakeSqliteDatabase,
      private readonly sql: string
    ) {}

    run(params?: unknown): RunResult {
      const sql = normalizeSql(this.sql);

      if (sql.includes("insert into memories") && sql.includes("'local'")) {
        const row = params as {
          id: string;
          content: string;
          layer: MemoryLayer;
          embedding: Buffer | null;
          tags: string;
          metadata: string | null;
          importance_score: number;
          created_at: number;
          updated_at: number;
        };
        this.db.memories.set(row.id, {
          id: row.id,
          content: row.content,
          layer: row.layer,
          ownership: "local",
          embedding: row.embedding,
          tags: row.tags,
          metadata: row.metadata,
          importance_score: row.importance_score,
          created_at: row.created_at,
          updated_at: row.updated_at,
          synced_at: null,
          deleted_at: null,
        });
        return { changes: 1 };
      }

      if (sql.includes("insert or replace into memories") && sql.includes("'cached'")) {
        const row = params as {
          id: string;
          content: string;
          layer: MemoryLayer;
          embedding: Buffer | null;
          tags: string;
          metadata: string | null;
          importance_score: number;
          created_at: number;
          updated_at: number;
          synced_at: number;
        };
        this.db.memories.set(row.id, {
          id: row.id,
          content: row.content,
          layer: row.layer,
          ownership: "cached",
          embedding: row.embedding,
          tags: row.tags,
          metadata: row.metadata,
          importance_score: row.importance_score,
          created_at: row.created_at,
          updated_at: row.updated_at,
          synced_at: row.synced_at,
          deleted_at: null,
        });
        return { changes: 1 };
      }

      if (sql.includes("insert into sync_queue")) {
        const record = params as {
          memory_id: string;
          operation: SyncOperation;
          queued_at: number;
        };
        this.db.syncQueue.push({
          id: this.db.nextSyncQueueId++,
          memory_id: record.memory_id,
          operation: record.operation,
          queued_at: record.queued_at,
        });
        return { changes: 1 };
      }

      if (sql.startsWith("update memories") && sql.includes("set content = @content")) {
        const update = params as {
          id: string;
          content: string;
          tags: string;
          metadata: string | null;
          embedding: Buffer | null;
          importance_score: number;
          updated_at: number;
        };
        const row = this.db.memories.get(update.id);
        if (!row || row.ownership !== "local" || row.deleted_at !== null) {
          return { changes: 0 };
        }
        row.content = update.content;
        row.tags = update.tags;
        row.metadata = update.metadata;
        row.embedding = update.embedding;
        row.importance_score = update.importance_score;
        row.updated_at = update.updated_at;
        return { changes: 1 };
      }

      if (sql.startsWith("update memories") && sql.includes("set deleted_at = @deleted_at")) {
        const update = params as { id: string; deleted_at: number; updated_at: number };
        const row = this.db.memories.get(update.id);
        if (!row || row.ownership !== "local" || row.deleted_at !== null) {
          return { changes: 0 };
        }
        row.deleted_at = update.deleted_at;
        row.updated_at = update.updated_at;
        return { changes: 1 };
      }

      if (sql.startsWith("update memories") && sql.includes("set embedding = @embedding")) {
        const update = params as { id: string; embedding: Buffer; updated_at: number };
        const row = this.db.memories.get(update.id);
        if (!row || row.deleted_at !== null) {
          return { changes: 0 };
        }
        row.embedding = update.embedding;
        row.updated_at = update.updated_at;
        return { changes: 1 };
      }

      if (sql.includes("delete from memories") && sql.includes("where id in")) {
        const row = params as { limit: number };
        const candidates = [...this.db.memories.values()]
          .filter((memory) => memory.ownership === "cached")
          .sort((a, b) => (a.synced_at ?? 0) - (b.synced_at ?? 0))
          .slice(0, row.limit);
        for (const candidate of candidates) {
          this.db.memories.delete(candidate.id);
        }
        return { changes: candidates.length };
      }

      if (sql.includes("delete from sync_queue where memory_id = ?")) {
        const memoryId = String(params ?? "");
        const before = this.db.syncQueue.length;
        this.db.syncQueue = this.db.syncQueue.filter((row) => row.memory_id !== memoryId);
        return { changes: before - this.db.syncQueue.length };
      }

      if (sql.includes("delete from memories where id = ?")) {
        const memoryId = String(params ?? "");
        const existed = this.db.memories.delete(memoryId);
        return { changes: existed ? 1 : 0 };
      }

      if (sql.includes("delete from sync_queue where id = ?")) {
        const queueId = Number(params);
        const before = this.db.syncQueue.length;
        this.db.syncQueue = this.db.syncQueue.filter((row) => row.id !== queueId);
        return { changes: before - this.db.syncQueue.length };
      }

      if (sql.includes("insert into sync_cursors") && sql.includes("on conflict")) {
        const row = params as {
          server_url: string;
          direction: string;
          cursor: string;
          updated_at: number;
        };
        this.db.syncCursors.set(`${row.server_url}|${row.direction}`, {
          cursor: row.cursor,
          updated_at: row.updated_at,
        });
        return { changes: 1 };
      }

      return { changes: 0 };
    }

    get(params?: unknown): unknown {
      const sql = normalizeSql(this.sql);

      if (sql.includes("pragma user_version")) {
        return { user_version: this.db.userVersion };
      }

      if (sql.includes("where id = @id") && sql.includes("ownership = 'local'")) {
        const id = (params as { id: string }).id;
        const row = this.db.memories.get(id);
        if (!row || row.ownership !== "local" || row.deleted_at !== null) {
          return undefined;
        }
        return row;
      }

      if (sql.includes("where id = @id") && sql.includes("ownership = 'cached'")) {
        const id = (params as { id: string }).id;
        const row = this.db.memories.get(id);
        if (!row || row.ownership !== "cached" || row.deleted_at !== null) {
          return undefined;
        }
        return row;
      }

      if (sql.includes("where id = @id") && sql.includes("limit 1") && sql.includes("from memories")) {
        const id = (params as { id: string }).id;
        return this.db.memories.get(id);
      }

      if (sql.includes("select count(*) as count") && sql.includes("from memories") && sql.includes("ownership = 'cached'")) {
        return {
          count: [...this.db.memories.values()].filter((row) => row.ownership === "cached").length,
        };
      }

      if (sql.includes("select count(*) as count") && sql.includes("from sync_queue")) {
        return { count: this.db.syncQueue.length };
      }

      if (sql.includes("select count(*) as count") && sql.includes("from memories")) {
        return { count: this.db.memories.size };
      }

      if (sql.includes("select max(updated_at) as updated_at") && sql.includes("direction = 'push'")) {
        const values = [...this.db.syncCursors.entries()]
          .filter(([key]) => key.endsWith("|push") && !key.startsWith("_device|"))
          .map(([, value]) => value.updated_at);
        return { updated_at: values.length > 0 ? Math.max(...values) : null };
      }

      if (sql.includes("select max(updated_at) as updated_at") && sql.includes("direction = 'pull'")) {
        const values = [...this.db.syncCursors.entries()]
          .filter(([key]) => key.endsWith("|pull") && !key.startsWith("_device|"))
          .map(([, value]) => value.updated_at);
        return { updated_at: values.length > 0 ? Math.max(...values) : null };
      }

      if (sql.includes("select cursor") && sql.includes("from sync_cursors")) {
        const p = params as { server_url: string; direction: string };
        const row = this.db.syncCursors.get(`${p.server_url}|${p.direction}`);
        if (!row) {
          return undefined;
        }
        return { cursor: row.cursor };
      }

      return undefined;
    }

    all(params?: unknown): unknown[] {
      const sql = normalizeSql(this.sql);

      if (sql.includes("where ownership = 'local'") && sql.includes("content like @pattern")) {
        const p = params as { pattern: string; limit: number };
        return [...this.db.memories.values()]
          .filter(
            (row) =>
              row.ownership === "local" &&
              row.deleted_at === null &&
              ["agent", "user", "session"].includes(row.layer) &&
              likeMatch(row.content, p.pattern)
          )
          .sort((a, b) => b.updated_at - a.updated_at)
          .slice(0, p.limit);
      }

      if (sql.includes("where ownership = 'local'") && sql.includes("order by updated_at desc")) {
        const p = params as { limit: number };
        return [...this.db.memories.values()]
          .filter(
            (row) =>
              row.ownership === "local" &&
              row.deleted_at === null &&
              ["agent", "user", "session"].includes(row.layer)
          )
          .sort((a, b) => b.updated_at - a.updated_at)
          .slice(0, p.limit);
      }

      if (sql.includes("where ownership = 'cached'") && sql.includes("content like @pattern")) {
        const p = params as { pattern: string; limit: number };
        return [...this.db.memories.values()]
          .filter(
            (row) =>
              row.ownership === "cached" &&
              row.deleted_at === null &&
              ["project", "team", "org", "tenant"].includes(row.layer) &&
              likeMatch(row.content, p.pattern)
          )
          .sort((a, b) => b.updated_at - a.updated_at)
          .slice(0, p.limit);
      }

      if (sql.includes("where ownership = 'cached'") && sql.includes("order by updated_at desc")) {
        const p = params as { limit: number };
        return [...this.db.memories.values()]
          .filter(
            (row) =>
              row.ownership === "cached" &&
              row.deleted_at === null &&
              ["project", "team", "org", "tenant"].includes(row.layer)
          )
          .sort((a, b) => b.updated_at - a.updated_at)
          .slice(0, p.limit);
      }

      if (sql.includes("select id") && sql.includes("where ownership = 'local'") && sql.includes("layer = 'session'")) {
        const p = params as { cutoff: number };
        return [...this.db.memories.values()]
          .filter((row) => row.ownership === "local" && row.layer === "session" && row.created_at < p.cutoff)
          .map((row) => ({ id: row.id }));
      }

      if (sql.includes("select id, memory_id, operation, queued_at") && sql.includes("from sync_queue")) {
        const p = params as { limit: number };
        return [...this.db.syncQueue].sort((a, b) => a.queued_at - b.queued_at).slice(0, p.limit);
      }

      if (sql.includes("select 'layer:' || layer as key")) {
        const counts = new Map<string, number>();
        for (const row of this.db.memories.values()) {
          const key = `layer:${row.layer}`;
          counts.set(key, (counts.get(key) ?? 0) + 1);
        }
        return [...counts.entries()].map(([key, count]) => ({ key, count }));
      }

      if (sql.includes("select 'ownership:' || ownership as key")) {
        const counts = new Map<string, number>();
        for (const row of this.db.memories.values()) {
          const key = `ownership:${row.ownership}`;
          counts.set(key, (counts.get(key) ?? 0) + 1);
        }
        return [...counts.entries()].map(([key, count]) => ({ key, count }));
      }

      return [];
    }
  }

  class FakeSqliteDatabase {
    public readonly path: string;
    public readonly options: { strict: boolean };
    public readonly execCalls: string[] = [];
    public readonly pragmaCalls: string[] = [];
    public readonly prepareCalls: string[] = [];
    public readonly memories = new Map<string, MemoryRecord>();
    public syncQueue: SyncQueueRecord[] = [];
    public readonly syncCursors = new Map<string, CursorRecord>();
    public nextSyncQueueId = 1;
    public closed = false;
    public journalMode: string | null = null;
    public busyTimeout: number | null = null;
    public userVersion: number;
    public setUserVersionCalls = 0;

    constructor(path: string, options: { strict: boolean }) {
      this.path = path;
      this.options = options;
      this.userVersion = state.persistedUserVersions.get(path) ?? state.initialUserVersion;
      state.instances.push(this);
    }

    exec(sql: string): void {
      this.execCalls.push(sql);

      const normalized = normalizeSql(sql);
      if (normalized === "pragma journal_mode = wal") {
        this.journalMode = "WAL";
      }

      if (normalized === "pragma busy_timeout = 5000") {
        this.busyTimeout = 5000;
      }

      if (normalized.startsWith("pragma user_version =")) {
        const parsed = Number.parseInt(normalized.replace("pragma user_version =", "").trim(), 10);
        if (Number.isFinite(parsed)) {
          this.userVersion = parsed;
          this.setUserVersionCalls += 1;
          state.persistedUserVersions.set(this.path, parsed);
        }
      }
    }

    prepare(sql: string): FakeStatement {
      this.prepareCalls.push(sql);
      return new FakeStatement(this, sql);
    }

    query(sql: string): FakeStatement {
      this.prepareCalls.push(sql);
      return new FakeStatement(this, sql);
    }

    transaction<TArgs extends readonly unknown[], TReturn>(
      fn: (...args: TArgs) => TReturn
    ): (...args: TArgs) => TReturn {
      return (...args: TArgs) => fn(...args);
    }

    close(): void {
      this.closed = true;
    }
  }

  const state = {
    instances: [] as FakeSqliteDatabase[],
    initialUserVersion: 0,
    persistedUserVersions: new Map<string, number>(),
  };

  const mockBunSqlite = vi.fn((path: string, options: { strict: boolean }) => {
    return new FakeSqliteDatabase(path, options);
  });

  const mockMkdirSync = vi.fn();
  const mockExistsSync = vi.fn(() => false);
  const mockReadFileSync = vi.fn(() => "");

  const reset = (): void => {
    state.instances = [];
    state.initialUserVersion = 0;
    state.persistedUserVersions = new Map<string, number>();
    mockBunSqlite.mockClear();
    mockMkdirSync.mockClear();
    mockExistsSync.mockReset();
    mockReadFileSync.mockReset();
    mockExistsSync.mockReturnValue(false);
    mockReadFileSync.mockReturnValue("");
  };

  const lastInstance = (): FakeSqliteDatabase => {
    const instance = state.instances.at(-1);
    if (!instance) {
      throw new Error("No fake sqlite instance created");
    }
    return instance;
  };

  return {
    state,
    reset,
    lastInstance,
    mockBunSqlite,
    mockMkdirSync,
    mockExistsSync,
    mockReadFileSync,
  };
});

vi.mock("bun:sqlite", () => ({
  Database: testState.mockBunSqlite,
}));

vi.mock("node:fs", async () => {
  const actual = await vi.importActual<typeof import("node:fs")>("node:fs");
  return {
    ...actual,
    mkdirSync: testState.mockMkdirSync,
    existsSync: testState.mockExistsSync,
    readFileSync: testState.mockReadFileSync,
  };
});

import { AeternaClient } from "../client.js";
import { LocalDatabase } from "./db.js";
import { LocalMemoryManager } from "./manager.js";
import { MemoryRouter } from "./router.js";
import { SyncEngine } from "./sync.js";
import { DEFAULT_LOCAL_CONFIG, parseLocalConfig } from "./config.js";
import { SCHEMA_STATEMENTS, SCHEMA_VERSION } from "./schema.js";
import type {
  MemoryAddParams,
  MemoryEntry,
  MemoryLayer,
  MemorySearchResult,
  SyncPullResponse,
  SyncPushResponse,
} from "../types.js";

function mockFetch(response: unknown, options: { ok?: boolean; status?: number } = {}) {
  const { ok = true, status = 200 } = options;
  return vi.fn().mockResolvedValue({
    ok,
    status,
    statusText: ok ? "OK" : "Error",
    json: () => Promise.resolve(response),
  });
}

function createClient(overrides: Partial<{ serverUrl: string; token: string }> = {}) {
  return new AeternaClient({
    project: "test-project",
    directory: "/tmp/test",
    serverUrl: overrides.serverUrl ?? "http://localhost:8080",
    token: overrides.token ?? "test-token",
    team: "test-team",
    org: "test-org",
    userId: "test-user",
  });
}

const createManager = (
  overrides: Partial<typeof DEFAULT_LOCAL_CONFIG> = {}
): LocalMemoryManager => {
  return new LocalMemoryManager("/tmp/aeterna/local.db", {
    ...DEFAULT_LOCAL_CONFIG,
    ...overrides,
  });
};

describe("Local local-first components", () => {
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    testState.reset();
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  describe("7.1 LocalDatabase", () => {
    it("creates schema and sets pragmas on first open", () => {
      const db = new LocalDatabase("/tmp/aeterna/local.db");
      const sqlite = testState.lastInstance();

      expect(db.connection).toBeDefined();
      expect(testState.mockMkdirSync).toHaveBeenCalledWith("/tmp/aeterna", { recursive: true });
      expect(testState.mockBunSqlite).toHaveBeenCalledWith("/tmp/aeterna/local.db", {
        strict: true,
      });
      expect(sqlite.journalMode).toBe("WAL");
      expect(sqlite.busyTimeout).toBe(5000);
      expect(sqlite.execCalls).toHaveLength(SCHEMA_STATEMENTS.length + 3);
      expect(sqlite.userVersion).toBe(SCHEMA_VERSION);
    });

    it("is idempotent when reopening with current schema", () => {
      testState.state.initialUserVersion = SCHEMA_VERSION;

      const db1 = new LocalDatabase("/tmp/aeterna/local.db");
      const sqlite1 = testState.lastInstance();
      db1.close();

      const db2 = new LocalDatabase("/tmp/aeterna/local.db");
      const sqlite2 = testState.lastInstance();

      expect(sqlite1.setUserVersionCalls).toBe(0);
      expect(sqlite2.setUserVersionCalls).toBe(0);
      expect(sqlite2.execCalls).toHaveLength(SCHEMA_STATEMENTS.length + 2);
      db2.close();
    });

    it("runs migration path when schema is behind", () => {
      testState.state.initialUserVersion = SCHEMA_VERSION - 1;

      new LocalDatabase("/tmp/aeterna/local.db");
      const sqlite = testState.lastInstance();

      expect(sqlite.setUserVersionCalls).toBe(1);
      expect(sqlite.userVersion).toBe(SCHEMA_VERSION);
    });

    it("throws when database schema is newer than supported", () => {
      testState.state.initialUserVersion = SCHEMA_VERSION + 1;

      expect(() => new LocalDatabase("/tmp/aeterna/local.db")).toThrow(
        "Unsupported local schema version"
      );
    });
  });

  describe("7.2 LocalMemoryManager CRUD", () => {
    it("adds local memory and enqueues upsert sync operation", () => {
      vi.spyOn(Date, "now").mockReturnValue(1_700_000_000_000);
      vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000001");

      const manager = createManager();
      const entry = manager.add({
        content: "remember this",
        layer: "session",
        tags: ["note"],
      });

      expect(entry.id).toBe("00000000-0000-4000-8000-000000000001");
      expect(entry.content).toBe("remember this");
      expect(entry.layer).toBe("session");
      expect(entry.tags).toEqual(["note"]);

      const queue = manager.listSyncQueue();
      expect(queue).toHaveLength(1);
      expect(queue[0]).toMatchObject({
        memoryId: "00000000-0000-4000-8000-000000000001",
        operation: "upsert",
      });
    });

    it("updates existing local memory and enqueues another upsert", () => {
      vi.spyOn(Date, "now")
        .mockReturnValueOnce(1_700_000_000_000)
        .mockReturnValueOnce(1_700_000_100_000);
      vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000002");

      const manager = createManager();
      manager.add({ content: "before", layer: "user", tags: ["old"] });
      const updated = manager.update("00000000-0000-4000-8000-000000000002", {
        content: "after",
        tags: ["new"],
        importance: 0.8,
      });

      expect(updated.content).toBe("after");
      expect(updated.tags).toEqual(["new"]);
      expect(updated.importance).toBe(0.8);

      const queue = manager.listSyncQueue();
      expect(queue).toHaveLength(2);
      expect(queue[1]).toMatchObject({
        memoryId: "00000000-0000-4000-8000-000000000002",
        operation: "upsert",
      });
    });

    it("soft deletes local memory, enqueues delete, and hides getById", () => {
      vi.spyOn(Date, "now")
        .mockReturnValueOnce(1_700_000_000_000)
        .mockReturnValueOnce(1_700_000_200_000);
      vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000003");

      const manager = createManager();
      manager.add({ content: "to remove", layer: "agent" });
      manager.delete("00000000-0000-4000-8000-000000000003");

      expect(manager.getById("00000000-0000-4000-8000-000000000003")).toBeNull();
      const queue = manager.listSyncQueue();
      expect(queue).toHaveLength(2);
      expect(queue[1]).toMatchObject({
        memoryId: "00000000-0000-4000-8000-000000000003",
        operation: "delete",
      });
    });

    it("returns null for missing id and throws update on missing memory", () => {
      const manager = createManager();

      expect(manager.getById("does-not-exist")).toBeNull();
      expect(() => manager.update("does-not-exist", { content: "x" })).toThrow("Memory not found");
    });
  });

  describe("7.3 LocalMemoryManager.search", () => {
    it("ranks cosine similarity using embeddings", () => {
      vi.spyOn(crypto, "randomUUID")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000011")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000012");

      const manager = createManager();
      manager.add({ content: "first", layer: "session", embedding: [1, 0] });
      manager.add({ content: "second", layer: "session", embedding: [0.6, 0.8] });

      const results = manager.search("ignored", {
        queryEmbedding: [1, 0],
        threshold: 0,
        limit: 5,
      });

      expect(results).toHaveLength(2);
      expect(results[0].memory.id).toBe("00000000-0000-4000-8000-000000000011");
      expect(results[0].score).toBeGreaterThan(results[1].score);
    });

    it("falls back to text LIKE search when query embedding is missing", () => {
      vi.spyOn(crypto, "randomUUID")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000021")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000022");

      const manager = createManager();
      manager.add({ content: "docker compose setup", layer: "session" });
      manager.add({ content: "redis tuning", layer: "session" });

      const results = manager.search("docker", { limit: 10 });
      expect(results).toHaveLength(1);
      expect(results[0].memory.id).toBe("00000000-0000-4000-8000-000000000021");
      expect(results[0].score).toBe(1);
    });

    it("returns empty results for no vectors and for no text matches", () => {
      vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000031");

      const manager = createManager();
      manager.add({ content: "plain text only", layer: "session" });

      const vectorResults = manager.search("plain", {
        queryEmbedding: [1, 0],
        threshold: 0,
      });
      const textResults = manager.search("not-found");

      expect(vectorResults).toEqual([]);
      expect(textResults).toEqual([]);
    });
  });

  describe("7.4 shared cache operations", () => {
    it("upserts cached memories and exposes synced_at in cached search metadata", () => {
      vi.spyOn(Date, "now").mockReturnValue(1_700_100_000_000);
      const manager = createManager();

      manager.upsertCached({
        id: "cache-1",
        content: "team standards",
        layer: "team",
        tags: ["standards"],
        updatedAt: "2026-01-01T00:00:00.000Z",
        createdAt: "2025-12-01T00:00:00.000Z",
      });

      const results = manager.searchCached("team", { layers: ["team"], limit: 5 });

      expect(results).toHaveLength(1);
      expect(results[0].memory.id).toBe("cache-1");
      expect(results[0].memory.metadata?.synced_at).toBe(1_700_100_000_000);
    });

    it("searches cached data by vector similarity and layer", () => {
      const manager = createManager();

      manager.upsertCached({
        id: "cache-vec-1",
        content: "project architecture",
        layer: "project",
        embedding: [1, 0],
        updatedAt: "2026-01-01T00:00:00.000Z",
      });
      manager.upsertCached({
        id: "cache-vec-2",
        content: "org governance",
        layer: "org",
        embedding: [0, 1],
        updatedAt: "2026-01-01T00:00:00.000Z",
      });

      const results = manager.searchCached("ignored", {
        layers: ["project"],
        queryEmbedding: [1, 0],
        threshold: 0,
      });

      expect(results).toHaveLength(1);
      expect(results[0].memory.id).toBe("cache-vec-1");
    });

    it("evicts oldest cached entries when max is exceeded", () => {
      vi.spyOn(Date, "now")
        .mockReturnValueOnce(100)
        .mockReturnValueOnce(200)
        .mockReturnValueOnce(300);
      const manager = createManager({ max_cached_entries: 2 });

      manager.upsertCached({
        id: "cache-old",
        content: "old",
        layer: "project",
        updatedAt: "2026-01-01T00:00:00.000Z",
      });
      manager.upsertCached({
        id: "cache-mid",
        content: "mid",
        layer: "team",
        updatedAt: "2026-01-01T00:00:00.000Z",
      });
      manager.upsertCached({
        id: "cache-new",
        content: "new",
        layer: "org",
        updatedAt: "2026-01-01T00:00:00.000Z",
      });

      const deleted = manager.evictOldCached();
      const remaining = manager.searchCached("", { limit: 10 });

      expect(deleted).toBe(1);
      expect(remaining.map((r) => r.memory.id)).not.toContain("cache-old");
    });

    it("expires old session memories and removes their sync queue entries", () => {
      vi.spyOn(Date, "now")
        .mockReturnValueOnce(0)
        .mockReturnValueOnce(2 * 3600 * 1000)
        .mockReturnValueOnce(2 * 3600 * 1000);
      vi.spyOn(crypto, "randomUUID")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000041")
        .mockReturnValueOnce("00000000-0000-4000-8000-000000000042");

      const manager = createManager({ session_storage_ttl_hours: 1 });
      manager.add({ content: "old session", layer: "session" });
      manager.add({ content: "fresh session", layer: "session" });

      const expired = manager.expireSessionMemories();
      const queueIds = manager.listSyncQueue().map((item) => item.memoryId);

      expect(expired).toBe(1);
      expect(manager.getById("00000000-0000-4000-8000-000000000041")).toBeNull();
      expect(manager.getById("00000000-0000-4000-8000-000000000042")).not.toBeNull();
      expect(queueIds).not.toContain("00000000-0000-4000-8000-000000000041");
    });
  });

  describe("7.5 SyncEngine push cycle", () => {
    const createPushManager = () => {
      return {
        getSyncCursor: vi.fn((serverUrl: string, direction: string) => {
          if (serverUrl === "_device" && direction === "id") {
            return "device-1";
          }
          return null;
        }),
        setSyncCursor: vi.fn(),
        listSyncQueue: vi.fn(() => [
          {
            queueId: 1,
            memoryId: "mem-1",
            operation: "upsert",
            queuedAt: 10,
          },
        ]),
        getSyncMemorySnapshot: vi.fn(() => ({
          id: "mem-1",
          content: "hello",
          layer: "session" as MemoryLayer,
          tags: ["tag"],
          metadata: { source: "test" },
          importance: 0.5,
          createdAt: "2026-01-01T00:00:00.000Z",
          updatedAt: "2026-01-01T00:00:00.000Z",
        })),
        removeSyncQueueItems: vi.fn(),
        updateEmbedding: vi.fn(),
        upsertCached: vi.fn(),
        evictOldCached: vi.fn(),
        expireSessionMemories: vi.fn(),
        close: vi.fn(),
      };
    };

    it("pushes queued entries, drains queue, and updates push cursor", async () => {
      const manager = createPushManager();
      const client = createClient();
      const response: SyncPushResponse = {
        cursor: "push-cursor-1",
        conflicts: [],
        embeddings: {},
      };
      globalThis.fetch = mockFetch(response);

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pushCycle();

      expect(manager.removeSyncQueueItems).toHaveBeenCalledWith([1]);
      expect(manager.setSyncCursor).toHaveBeenCalledWith(
        "http://localhost:8080",
        "push",
        "push-cursor-1"
      );
      expect(engine.getServerConnectivity()).toBe(true);
    });

    it("stores embeddings returned from push response", async () => {
      const manager = createPushManager();
      const client = createClient();
      const response: SyncPushResponse = {
        cursor: "push-cursor-2",
        conflicts: [],
        embeddings: {
          "mem-1": [0.9, 0.1],
        },
      };
      globalThis.fetch = mockFetch(response);

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pushCycle();

      expect(manager.updateEmbedding).toHaveBeenCalledWith("mem-1", [0.9, 0.1]);
    });

    it("accepts conflict payload and still completes queue drain", async () => {
      const manager = createPushManager();
      const client = createClient();
      const response: SyncPushResponse = {
        cursor: "push-cursor-3",
        conflicts: [
          {
            id: "mem-1",
            remote_content: "remote value",
            remote_updated_at: "2026-01-01T00:00:10.000Z",
          },
        ],
        embeddings: {},
      };
      globalThis.fetch = mockFetch(response);

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pushCycle();

      expect(manager.removeSyncQueueItems).toHaveBeenCalledWith([1]);
      expect(engine.getServerConnectivity()).toBe(true);
    });

    it("applies backoff on push failure and skips immediate retry", async () => {
      const manager = createPushManager();
      const client = createClient();
      globalThis.fetch = vi.fn().mockRejectedValue(new Error("network down"));

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pushCycle();
      await engine.pushCycle();

      expect(globalThis.fetch).toHaveBeenCalledTimes(1);
      expect(engine.getServerConnectivity()).toBe(false);
      expect(manager.removeSyncQueueItems).not.toHaveBeenCalled();
    });
  });

  describe("7.6 SyncEngine pull cycle", () => {
    const createPullManager = () => {
      const cursorStore = new Map<string, string>([["_device|id", "device-1"]]);

      return {
        getSyncCursor: vi.fn((serverUrl: string, direction: string) => {
          return cursorStore.get(`${serverUrl}|${direction}`) ?? null;
        }),
        setSyncCursor: vi.fn((serverUrl: string, direction: string, cursor: string) => {
          cursorStore.set(`${serverUrl}|${direction}`, cursor);
        }),
        listSyncQueue: vi.fn(() => []),
        getSyncMemorySnapshot: vi.fn(),
        removeSyncQueueItems: vi.fn(),
        updateEmbedding: vi.fn(),
        upsertCached: vi.fn(),
        evictOldCached: vi.fn(() => 0),
        expireSessionMemories: vi.fn(() => 0),
        close: vi.fn(),
      };
    };

    it("pulls pages, upserts cache entries, and updates cursor", async () => {
      const manager = createPullManager();
      const client = createClient();

      const page1: SyncPullResponse = {
        entries: [
          {
            id: "shared-1",
            content: "project memory",
            layer: "project",
            embedding: [1, 0],
            tags: ["a"],
            importance: 0.4,
            created_at: "2026-01-01T00:00:00.000Z",
            updated_at: "2026-01-01T00:00:00.000Z",
          },
        ],
        cursor: "pull-c1",
        has_more: true,
      };

      const page2: SyncPullResponse = {
        entries: [
          {
            id: "shared-2",
            content: "team memory",
            layer: "team",
            embedding: [0, 1],
            tags: ["b"],
            importance: 0.5,
            created_at: "2026-01-01T00:00:00.000Z",
            updated_at: "2026-01-01T00:00:01.000Z",
          },
        ],
        cursor: "pull-c2",
        has_more: false,
      };

      globalThis.fetch = vi
        .fn()
        .mockResolvedValueOnce({ ok: true, status: 200, statusText: "OK", json: () => Promise.resolve(page1) })
        .mockResolvedValueOnce({ ok: true, status: 200, statusText: "OK", json: () => Promise.resolve(page2) });

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pullCycle();

      expect(manager.upsertCached).toHaveBeenCalledTimes(2);
      expect(manager.setSyncCursor).toHaveBeenCalledWith(
        "http://localhost:8080",
        "pull",
        "pull-c2"
      );
      expect(manager.evictOldCached).toHaveBeenCalledTimes(1);
      expect(manager.expireSessionMemories).toHaveBeenCalledTimes(1);
      expect(engine.getServerConnectivity()).toBe(true);
    });

    it("limits pagination to 10 pages even when has_more remains true", async () => {
      const manager = createPullManager();
      const client = createClient();

      const repeated: SyncPullResponse = {
        entries: [],
        cursor: "still-more",
        has_more: true,
      };
      globalThis.fetch = vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        statusText: "OK",
        json: () => Promise.resolve(repeated),
      });

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );
      await engine.pullCycle();

      expect(globalThis.fetch).toHaveBeenCalledTimes(10);
    });

    it("handles pull errors without throwing and marks connectivity false", async () => {
      const manager = createPullManager();
      const client = createClient();
      globalThis.fetch = vi.fn().mockRejectedValue(new Error("pull failed"));

      const engine = new SyncEngine(
        manager as unknown as LocalMemoryManager,
        client,
        DEFAULT_LOCAL_CONFIG
      );

      await expect(engine.pullCycle()).resolves.toBeUndefined();
      expect(engine.getServerConnectivity()).toBe(false);
    });
  });

  describe("7.7 MemoryRouter", () => {
    const localResult = (id: string, layer: MemoryLayer): MemorySearchResult => ({
      score: 0.9,
      memory: {
        id,
        content: "local memory",
        layer,
        importance: 0.5,
        tags: [],
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-01T00:00:00.000Z",
      },
    });

    const remoteResult = (id: string, layer: MemoryLayer): MemorySearchResult => ({
      score: 0.8,
      memory: {
        id,
        content: "remote memory",
        layer,
        importance: 0.4,
        tags: [],
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-01T00:00:00.000Z",
      },
    });

    const createRouter = () => {
      const localManager = {
        search: vi.fn(),
        searchCached: vi.fn(),
        add: vi.fn(),
      };

      const client = {
        memorySearchRemote: vi.fn(),
        memoryAddRemote: vi.fn(),
      };

      const router = new MemoryRouter(
        localManager as unknown as LocalMemoryManager,
        client as unknown as AeternaClient,
        DEFAULT_LOCAL_CONFIG
      );

      return { localManager, client, router };
    };

    it("routes personal-layer search to local manager", async () => {
      const { localManager, client, router } = createRouter();
      localManager.search.mockReturnValue([localResult("p-1", "session")]);
      localManager.searchCached.mockReturnValue([]);
      client.memorySearchRemote.mockResolvedValue([]);

      const results = await router.search({ query: "q", layers: ["session"], limit: 5 });

      expect(localManager.search).toHaveBeenCalledTimes(1);
      expect(client.memorySearchRemote).not.toHaveBeenCalled();
      expect(results[0].memory.metadata?.source).toBe("local");
    });

    it("uses fresh shared cache without remote fallback", async () => {
      const { localManager, client, router } = createRouter();
      localManager.search.mockReturnValue([]);
      localManager.searchCached.mockReturnValue([
        {
          ...localResult("c-1", "project"),
          memory: {
            ...localResult("c-1", "project").memory,
            metadata: { synced_at: Date.now() - 1_000 },
          },
        },
      ]);
      client.memorySearchRemote.mockResolvedValue([remoteResult("r-1", "project")]);

      const results = await router.search({ query: "q", layers: ["project"], limit: 5 });

      expect(client.memorySearchRemote).not.toHaveBeenCalled();
      expect(results[0].memory.metadata?.source).toBe("cache");
    });

    it("falls back to remote when shared cache is stale", async () => {
      const { localManager, client, router } = createRouter();
      localManager.search.mockReturnValue([]);
      localManager.searchCached.mockReturnValue([
        {
          ...localResult("c-stale", "team"),
          memory: {
            ...localResult("c-stale", "team").memory,
            metadata: { synced_at: Date.now() - 120_000 },
          },
        },
      ]);
      client.memorySearchRemote.mockResolvedValue([remoteResult("r-live", "team")]);

      const results = await router.search({ query: "q", layers: ["team"], limit: 5 });

      expect(client.memorySearchRemote).toHaveBeenCalledTimes(1);
      expect(results[0].memory.id).toBe("r-live");
      expect(results[0].memory.metadata?.source).toBe("remote");
    });

    it("returns stale cache with warning when remote fallback fails", async () => {
      const { localManager, client, router } = createRouter();
      localManager.search.mockReturnValue([]);
      localManager.searchCached.mockReturnValue([
        {
          ...localResult("c-fallback", "org"),
          memory: {
            ...localResult("c-fallback", "org").memory,
            metadata: { synced_at: Date.now() - 11 * 60_000 },
          },
        },
      ]);
      client.memorySearchRemote.mockRejectedValue(new Error("offline"));

      const results = await router.search({ query: "q", layers: ["org"], limit: 5 });

      expect(results[0].memory.metadata?.source).toBe("cache");
      expect(results[0].memory.metadata?.stale).toBe(true);
    });

    it("routes personal writes local and shared writes remote", async () => {
      const { localManager, client, router } = createRouter();
      const localEntry: MemoryEntry = {
        id: "local-write",
        content: "local",
        layer: "session",
        importance: 0,
        tags: [],
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-01T00:00:00.000Z",
      };
      const remoteEntry: MemoryEntry = {
        ...localEntry,
        id: "remote-write",
        layer: "project",
      };
      localManager.add.mockReturnValue(localEntry);
      client.memoryAddRemote.mockResolvedValue(remoteEntry);

      const personal = await router.add({ content: "local", layer: "session" } as MemoryAddParams);
      const shared = await router.add({ content: "remote", layer: "project" } as MemoryAddParams);

      expect(personal.id).toBe("local-write");
      expect(shared.id).toBe("remote-write");
      expect(localManager.add).toHaveBeenCalledTimes(1);
      expect(client.memoryAddRemote).toHaveBeenCalledTimes(1);
    });
  });

  describe("7.8 LocalConfig", () => {
    it("uses defaults when no env and no config file", () => {
      testState.mockExistsSync.mockReturnValue(false);
      const config = parseLocalConfig({}, "/workspace/test");

      expect(config.enabled).toBe(true);
      expect(config.sync_push_interval_ms).toBe(30000);
      expect(config.sync_pull_interval_ms).toBe(60000);
      expect(config.max_cached_entries).toBe(50000);
      expect(config.session_storage_ttl_hours).toBe(24);
      expect(config.db_path.endsWith("/.aeterna/local.db")).toBe(true);
    });

    it("parses [local] section from .aeterna/config.toml", () => {
      testState.mockExistsSync.mockReturnValueOnce(true).mockReturnValue(false);
      testState.mockReadFileSync.mockReturnValue(`
[local]
enabled = false
db_path = "~/custom/local.db"
sync_push_interval_ms = 45000
sync_pull_interval_ms = 90000
max_cached_entries = 1234
session_storage_ttl_hours = 48
`);

      const config = parseLocalConfig({}, "/workspace/test");

      expect(config.enabled).toBe(false);
      expect(config.sync_push_interval_ms).toBe(45000);
      expect(config.sync_pull_interval_ms).toBe(90000);
      expect(config.max_cached_entries).toBe(1234);
      expect(config.session_storage_ttl_hours).toBe(48);
      expect(config.db_path.endsWith("/custom/local.db")).toBe(true);
    });

    it("env vars override config file values", () => {
      testState.mockExistsSync.mockReturnValue(true);
      testState.mockReadFileSync.mockReturnValue(`
[local]
enabled = true
db_path = "~/from-file.db"
sync_push_interval_ms = 30000
`);

      const config = parseLocalConfig(
        {
          AETERNA_LOCAL_ENABLED: "false",
          AETERNA_LOCAL_DB_PATH: "~/from-env.db",
          AETERNA_LOCAL_SYNC_PUSH_INTERVAL_MS: "15000",
          AETERNA_LOCAL_SYNC_PULL_INTERVAL_MS: "20000",
          AETERNA_LOCAL_MAX_CACHED_ENTRIES: "250",
          AETERNA_LOCAL_SESSION_STORAGE_TTL_HOURS: "6",
        },
        "/workspace/test"
      );

      expect(config.enabled).toBe(false);
      expect(config.sync_push_interval_ms).toBe(15000);
      expect(config.sync_pull_interval_ms).toBe(20000);
      expect(config.max_cached_entries).toBe(250);
      expect(config.session_storage_ttl_hours).toBe(6);
      expect(config.db_path.endsWith("/from-env.db")).toBe(true);
    });
  });
});
