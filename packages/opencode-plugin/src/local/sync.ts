import type { AeternaClient } from "../client.js";
import type { SyncPushEntry } from "../types.js";
import type { LocalConfig } from "./config.js";
import type { LocalMemoryManager } from "./manager.js";

const SHARED_LAYERS = ["project", "team", "org", "tenant"] as const;

export class SyncEngine {
  private pushInterval: NodeJS.Timeout | null = null;
  private pullInterval: NodeJS.Timeout | null = null;
  private running = false;
  private readonly deviceId: string;
  private consecutivePushFailures = 0;
  private nextPushAllowedAt = 0;
  private serverConnectivity = false;

  constructor(
    private readonly manager: LocalMemoryManager,
    private readonly client: AeternaClient,
    private readonly config: LocalConfig
  ) {
    const existingDeviceId = this.manager.getSyncCursor("_device", "id");
    if (existingDeviceId) {
      this.deviceId = existingDeviceId;
    } else {
      this.deviceId = crypto.randomUUID();
      this.manager.setSyncCursor("_device", "id", this.deviceId);
    }
  }

  getDeviceId(): string {
    return this.deviceId;
  }

  getServerConnectivity(): boolean {
    return this.serverConnectivity;
  }

  async pushCycle(options?: { signal?: AbortSignal }): Promise<void> {
    const now = Date.now();
    if (now < this.nextPushAllowedAt) {
      return;
    }

    try {
      const queueItems = this.manager.listSyncQueue(100);
      if (queueItems.length === 0) {
        this.serverConnectivity = true;
        this.consecutivePushFailures = 0;
        this.nextPushAllowedAt = 0;
        return;
      }

      const entries: SyncPushEntry[] = [];
      const queueIds: number[] = [];

      for (const item of queueItems) {
        const snapshot = this.manager.getSyncMemorySnapshot(item.memoryId);
        if (!snapshot) {
          continue;
        }

        entries.push({
          id: snapshot.id,
          content: snapshot.content,
          layer: snapshot.layer,
          tags: snapshot.tags,
          metadata: snapshot.metadata,
          importance: snapshot.importance,
          created_at: snapshot.createdAt,
          updated_at: snapshot.updatedAt,
          deleted_at: snapshot.deletedAt,
        });
        queueIds.push(item.queueId);
      }

      if (entries.length === 0) {
        return;
      }

      const response = await this.client.syncPush(
        {
          entries,
          device_id: this.deviceId,
        },
        options
      );

      if (response.cursor) {
        this.manager.setSyncCursor(this.client.getServerUrl(), "push", response.cursor);
      }

      if (queueIds.length > 0) {
        this.manager.removeSyncQueueItems(queueIds);
      }

      for (const [memoryId, embedding] of Object.entries(response.embeddings ?? {})) {
        if (embedding.length > 0) {
          this.manager.updateEmbedding(memoryId, embedding);
        }
      }

      this.serverConnectivity = true;
      this.consecutivePushFailures = 0;
      this.nextPushAllowedAt = 0;
    } catch (error) {
      this.serverConnectivity = false;
      this.consecutivePushFailures += 1;
      const delay = this.getBackoffDelayMs(this.consecutivePushFailures);
      this.nextPushAllowedAt = Date.now() + delay;
      void error;
    }
  }

  async pullCycle(options?: { signal?: AbortSignal }): Promise<void> {
    try {
      let cursor = this.manager.getSyncCursor(this.client.getServerUrl(), "pull") ?? undefined;
      let page = 0;
      let latestCursor = cursor;

      while (page < 10) {
        page += 1;
        const response = await this.client.syncPull(
          {
            sinceCursor: cursor,
            layers: [...SHARED_LAYERS],
            limit: 100,
          },
          options
        );

        for (const entry of response.entries) {
          this.manager.upsertCached({
            id: entry.id,
            content: entry.content,
            layer: entry.layer,
            embedding: entry.embedding,
            tags: entry.tags,
            metadata: entry.metadata,
            importance: entry.importance,
            createdAt: entry.created_at,
            updatedAt: entry.updated_at,
          });
        }

        latestCursor = response.cursor;
        cursor = response.cursor;
        if (!response.has_more) {
          break;
        }
      }

      if (latestCursor) {
        this.manager.setSyncCursor(this.client.getServerUrl(), "pull", latestCursor);
      }

      this.manager.evictOldCached();
      this.manager.expireSessionMemories();
      this.serverConnectivity = true;
    } catch (error) {
      this.serverConnectivity = false;
      void error;
    }
  }

  start(): void {
    if (this.running) {
      return;
    }

    this.running = true;
    void this.pushCycle();
    void this.pullCycle();

    this.pushInterval = setInterval(() => {
      void this.pushCycle();
    }, this.config.sync_push_interval_ms);

    this.pullInterval = setInterval(() => {
      void this.pullCycle();
    }, this.config.sync_pull_interval_ms);
  }

  stop(): void {
    if (!this.running) {
      return;
    }

    this.running = false;

    if (this.pushInterval) {
      clearInterval(this.pushInterval);
      this.pushInterval = null;
    }

    if (this.pullInterval) {
      clearInterval(this.pullInterval);
      this.pullInterval = null;
    }
  }

  async flushOnShutdown(): Promise<void> {
    this.stop();

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);
    try {
      await this.pushCycle({ signal: controller.signal });
    } catch {
    } finally {
      clearTimeout(timeout);
      this.manager.close();
    }
  }

  private getBackoffDelayMs(failures: number): number {
    if (failures <= 1) return 30000;
    if (failures === 2) return 60000;
    if (failures === 3) return 120000;
    return 300000;
  }
}
