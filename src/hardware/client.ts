import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

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
  dispose(): void;
}
export class NativeHardwareClient implements HardwareClient {
  private unlisten?: UnlistenFn;
  private disposed = false;
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
  dispose() {
    this.disposed = true;
    this.unlisten?.();
  }
}
