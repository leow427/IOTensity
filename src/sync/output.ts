import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { RGB } from '../domain/colors';

export type SyncSource = 'simulation' | 'test' | 'display';
export type SyncStatus =
  'stopped' | 'starting' | 'running' | 'stopping' | 'error';
export type SyncSnapshot = {
  sequence: number;
  source: SyncSource;
  status: SyncStatus;
  message: string;
  colors: { id: string; rgb: [number, number, number] }[];
};
export interface SyncOutput {
  readonly available: boolean;
  connect(): Promise<void>;
  subscribe(listener: () => void): () => void;
  getSnapshot(): SyncSnapshot;
  getColor(id: string): RGB | undefined;
  start(source: SyncSource, reducedMotion?: boolean): Promise<void>;
  stop(): Promise<void>;
  dispose(): void;
}
export type SyncTransport = {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen(callback: (snapshot: SyncSnapshot) => void): Promise<UnlistenFn>;
};
const nativeTransport: SyncTransport = {
  invoke,
  listen: (callback) =>
    listen<SyncSnapshot>('sync-output', ({ payload }) => callback(payload)),
};

// A passive cache of final native RGB8 values. No image processing, brightness,
// smoothing or animation is duplicated here. Paint loops read without React updates.
export class NativeSyncOutput implements SyncOutput {
  private snapshot: SyncSnapshot = {
    sequence: -1,
    source: 'test',
    status: 'stopped',
    message: 'Ready to start.',
    colors: [],
  };
  private colors = new Map<string, RGB>();
  private listeners = new Set<() => void>();
  private unlisten?: UnlistenFn;
  private connecting?: Promise<void>;
  private disposed = false;
  constructor(
    readonly available = isTauri(),
    private transport = nativeTransport,
  ) {}
  getSnapshot = () => this.snapshot;
  getColor = (id: string) => this.colors.get(id);
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private receive = (next: SyncSnapshot) => {
    if (this.disposed || next.sequence <= this.snapshot.sequence) return;
    const changed =
      next.status !== this.snapshot.status ||
      next.source !== this.snapshot.source ||
      next.message !== this.snapshot.message;
    this.snapshot = next;
    this.colors = new Map(
      next.colors.map(({ id, rgb }) => [
        id,
        rgb.map((c) => c / 255) as unknown as RGB,
      ]),
    );
    if (changed) this.listeners.forEach((listener) => listener());
  };
  connect() {
    if (!this.available) return Promise.resolve();
    this.connecting ??= (async () => {
      const unlisten = await this.transport.listen(this.receive);
      if (this.disposed) {
        unlisten();
        return;
      }
      this.unlisten = unlisten;
      this.receive(await this.transport.invoke<SyncSnapshot>('sync_snapshot'));
    })().catch((error) => {
      this.unlisten?.();
      this.unlisten = undefined;
      this.connecting = undefined;
      throw error;
    });
    return this.connecting;
  }
  async start(source: SyncSource, reducedMotion = false) {
    if (!this.available)
      throw new Error('Screen sync requires the desktop application.');
    await this.connect();
    this.receive(
      await this.transport.invoke<SyncSnapshot>('start_sync', {
        source,
        reducedMotion,
      }),
    );
  }
  async stop() {
    if (this.available)
      this.receive(await this.transport.invoke<SyncSnapshot>('stop_sync'));
  }
  dispose() {
    this.disposed = true;
    this.unlisten?.();
    this.listeners.clear();
  }
}
