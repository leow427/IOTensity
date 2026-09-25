import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { EditMode, Position } from '../domain/model';

export type LightPreview = {
  deviceId: string;
  position: Position;
  mode: EditMode;
};

export type Device = {
  deviceId: string;
  shortId: string;
  model: string;
  online: boolean;
  streaming: boolean;
  message: string;
  boundLightId: string | null;
};
export type DevicesSnapshot = {
  devices: Device[];
  discoveryError: string | null;
};
export interface NativeTransport {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, callback: (payload: T) => void): Promise<UnlistenFn>;
}
export const nativeTransport: NativeTransport = {
  invoke,
  listen: <T>(event: string, callback: (payload: T) => void) =>
    listen<T>(event, ({ payload }) => callback(payload)),
};
export interface HardwareClient {
  readonly available: boolean;
  connect(onChange: (snapshot: DevicesSnapshot) => void): Promise<void>;
  identify(deviceId: string): Promise<void>;
  retryDiscovery(): Promise<void>;
  preview(request: LightPreview | null): Promise<void>;
  dispose(): void;
}
export class NativeHardwareClient implements HardwareClient {
  private unlisten?: UnlistenFn;
  private disposed = false;
  private pendingPreview: LightPreview | null | undefined;
  private previewWrite?: Promise<void>;
  constructor(
    readonly available = isTauri(),
    private transport = nativeTransport,
  ) {}
  async connect(onChange: (snapshot: DevicesSnapshot) => void) {
    if (!this.available) return;
    this.unlisten?.();
    // A snapshot request can complete after an event; do not overwrite newer discovery.
    let receivedEvent = false;
    const unlisten = await this.transport.listen<DevicesSnapshot>(
      'hardware-devices',
      (snapshot) => {
        receivedEvent = true;
        if (!this.disposed) onChange(snapshot);
      },
    );
    if (this.disposed) {
      unlisten();
      return;
    }
    this.unlisten = unlisten;
    const snapshot =
      await this.transport.invoke<DevicesSnapshot>('hardware_snapshot');
    if (!this.disposed && !receivedEvent) onChange(snapshot);
  }
  identify(deviceId: string) {
    if (!this.available)
      return Promise.reject(
        new Error('Physical lights require the desktop application.'),
      );
    return this.transport.invoke<void>('identify_device', { deviceId });
  }
  retryDiscovery() {
    if (!this.available) return Promise.resolve();
    return this.transport.invoke<void>('retry_hardware_discovery');
  }
  preview(request: LightPreview | null): Promise<void> {
    if (!this.available || (this.disposed && request !== null))
      return Promise.resolve();
    // One IPC request in flight and one latest value. Dragging cannot build up
    // a queue of old colors, and clearing replaces any pending position.
    this.pendingPreview = request;
    this.previewWrite ??= Promise.resolve().then(() => this.flushPreview());
    return this.previewWrite;
  }
  private async flushPreview() {
    try {
      let failure: unknown;
      while (this.pendingPreview !== undefined) {
        const preview = this.pendingPreview;
        this.pendingPreview = undefined;
        try {
          await this.transport.invoke<void>('set_light_preview', { preview });
          failure = undefined;
        } catch (error) {
          failure = error;
        }
      }
      if (failure !== undefined) throw failure;
    } finally {
      this.previewWrite = undefined;
    }
  }
  dispose() {
    void this.preview(null).catch(() => undefined);
    this.disposed = true;
    this.unlisten?.();
  }
}
