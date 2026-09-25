import { afterEach, describe, expect, it } from 'vitest';
import legacy from './fixtures/configuration-v1.json';
import fixture from './fixtures/configuration.json';
import {
  clone,
  migrateConfiguration,
  validateConfiguration,
  type Configuration,
} from '../src/domain/model';
import { AppStore } from '../src/state/store';
import { FakeOutput, MemoryPersistence, deferred } from './helpers';
import {
  NativeHardwareClient,
  type DevicesSnapshot,
  type HardwareClient,
  type LightPreview,
  type NativeTransport,
} from '../src/hardware/client';

const id = 'esp32-020000a1b2c3';
export class FakeHardware implements HardwareClient {
  available = true;
  received?: (snapshot: DevicesSnapshot) => void;
  identified: string[] = [];
  retries = 0;
  previews: (LightPreview | null)[] = [];
  async preview(request: LightPreview | null) {
    this.previews.push(request);
  }
  async connect(receive: (snapshot: DevicesSnapshot) => void) {
    this.received = receive;
    this.update(true);
  }
  update(online: boolean) {
    this.received?.({
      discoveryError: null,
      devices: [
        {
          deviceId: id,
          shortId: 'IOT-A1B2C3',
          model: 'esp32-rgb',
          online,
          streaming: false,
          message: online ? 'Online' : 'Offline',
          boundLightId: null,
        },
      ],
    });
  }
  async identify(deviceId: string) {
    this.identified.push(deviceId);
  }
  async retryDiscovery() {
    this.retries++;
  }
  dispose() {}
}
const stores: AppStore[] = [];
afterEach(() => stores.splice(0).forEach((s) => s.dispose()));
describe('physical identity and configuration', () => {
  it('migrates v1 without mutation or lossy defaults and strictly rejects transient fields', () => {
    const original = clone(legacy);
    const result = migrateConfiguration(legacy);
    expect(result.schemaVersion).toBe(2);
    expect(
      result.rooms[0].lights.every((light) => light.output.kind === 'virtual'),
    ).toBe(true);
    expect(result.rooms[0].lights.map((l) => l.id)).toEqual(
      legacy.rooms[0].lights.map((l) => l.id),
    );
    expect(legacy).toEqual(original);
    for (const output of [
      { kind: 'esp32', deviceId: 'IOT-A1B2C3' },
      { kind: 'esp32', deviceId: id, ip: '192.168.1.20' },
      { kind: 'virtual', online: false },
      { kind: 'unknown' },
    ]) {
      const invalid = clone(fixture);
      Object.assign(invalid.rooms[0].lights[0], { output });
      expect(() => validateConfiguration(invalid)).toThrow();
    }
    const invalid = clone(legacy);
    Object.assign(invalid.rooms[0].lights[0], { output: { kind: 'virtual' } });
    expect(() => migrateConfiguration(invalid)).toThrow();
    expect(() =>
      migrateConfiguration({ ...legacy, schemaVersion: 99 }),
    ).toThrow('version');
  });
  it('binds once using full identity; offline, reload, and discovery never mutate room IDs', async () => {
    const persistence = new MemoryPersistence();
    const hardware = new FakeHardware();
    const store = new AppStore(persistence, new FakeOutput(), hardware);
    stores.push(store);
    await store.load();
    store.addLight();
    const logical = store.getSnapshot().selectedLightId!;
    expect(store.bindLight(logical, id)).toBe(true);
    expect(store.addPhysicalLight(id)).toBe(false);
    await store.identifyDevice(id);
    expect(hardware.identified).toEqual([id]);
    expect(await store.saveRoom()).toBe(true);
    hardware.update(false);
    expect(store.dirty).toBe(false);
    expect(store.getSnapshot().draft!.lights[0]).toMatchObject({
      id: logical,
      output: { kind: 'esp32', deviceId: id },
    });
    expect(JSON.stringify(persistence.config)).not.toMatch(
      /online|streaming|sessionId|192\.168/,
    );
    const reload = new AppStore(
      persistence,
      new FakeOutput(),
      new FakeHardware(),
    );
    stores.push(reload);
    await reload.load();
    expect(reload.getSnapshot().draft!.lights[0].id).toBe(logical);
    expect(reload.getSnapshot().running).toBe(false);
    expect(store.bindLight(logical, null)).toBe(true);
    expect(store.dirty).toBe(true);
    expect(store.bindLight(logical, id)).toBe(true);
    expect(store.dirty).toBe(false);
  });
  it('keeps failed assignments in the draft and prevents changes during acknowledgement', async () => {
    const persistence = new MemoryPersistence();
    const store = new AppStore(
      persistence,
      new FakeOutput(),
      new FakeHardware(),
    );
    stores.push(store);
    await store.load();
    expect(store.addPhysicalLight(id)).toBe(true);
    store.addLight();
    const virtual = store.getSnapshot().selectedLightId!;
    expect(store.bindLight(virtual, id)).toBe(false);
    persistence.error = new Error('disk full');
    expect(await store.saveRoom()).toBe(false);
    expect(store.getSnapshot().saved!.rooms[0].lights).toHaveLength(0);
    expect(store.getSnapshot().draft!.lights).toHaveLength(2);
    persistence.error = null;
    const gate = deferred();
    persistence.gate = gate.promise;
    const saving = store.saveRoom();
    expect(store.bindLight(store.getSnapshot().draft!.lights[0].id, null)).toBe(
      false,
    );
    gate.resolve();
    expect(await saving).toBe(true);
  });
  it('rejects a full ID bound in two different rooms but permits colliding short IDs', () => {
    const config = clone(fixture) as Configuration;
    const second = clone(config.rooms[0]);
    second.id = 'second';
    second.lights.forEach((l) => (l.id += '-second'));
    config.rooms.push(second);
    expect(() => validateConfiguration(config)).toThrow('bound');
    second.lights[3].output = { kind: 'esp32', deviceId: 'esp32-123456a1b2c3' };
    expect(() => validateConfiguration(config)).not.toThrow();
  });
});
describe('native hardware boundary', () => {
  it('coalesces rapid placement edits and sends cancellation after an in-flight command', async () => {
    const gate = deferred();
    const calls: unknown[] = [];
    const transport: NativeTransport = {
      listen: async () => () => {},
      invoke: async <T>(command: string, args?: Record<string, unknown>) => {
        calls.push([command, structuredClone(args)]);
        if (calls.length === 1) await gate.promise;
        return undefined as T;
      },
    };
    const client = new NativeHardwareClient(true, transport);
    const request: LightPreview = {
      deviceId: id,
      position: { x: -3, y: 1.2, z: 1 },
      mode: 'location',
    };
    const first = client.preview(request);
    await Promise.resolve();
    for (let i = 0; i < 100; i++)
      void client.preview({
        ...request,
        position: { ...request.position, x: i / 100 },
      });
    const clear = client.preview(null);
    gate.resolve();
    await Promise.all([first, clear]);
    expect(calls).toEqual([
      ['set_light_preview', { preview: request }],
      ['set_light_preview', { preview: null }],
    ]);
    await client.preview(request);
    expect(calls).toHaveLength(3);
    client.dispose();
    await client.preview(null);
    expect(calls.at(-1)).toEqual(['set_light_preview', { preview: null }]);
  });
  it('uses a hardware ID for Identify and never lets an old snapshot regress discovery', async () => {
    const gate = deferred();
    let receive!: (value: DevicesSnapshot) => void;
    const calls: unknown[] = [];
    const transport: NativeTransport = {
      listen: async <T>(_event: string, callback: (value: T) => void) => {
        receive = callback as (value: DevicesSnapshot) => void;
        return () => {};
      },
      invoke: async <T>(command: string, args?: Record<string, unknown>) => {
        calls.push([command, args]);
        if (command === 'hardware_snapshot') await gate.promise;
        return { devices: [], discoveryError: null } as T;
      },
    };
    const client = new NativeHardwareClient(true, transport);
    const events: DevicesSnapshot[] = [];
    const connecting = client.connect((snapshot) => events.push(snapshot));
    await Promise.resolve();
    receive({ devices: [], discoveryError: 'New event' });
    gate.resolve();
    await connecting;
    expect(events).toEqual([{ devices: [], discoveryError: 'New event' }]);
    await client.identify(id);
    await client.retryDiscovery();
    expect(calls).toContainEqual(['identify_device', { deviceId: id }]);
    expect(calls).toContainEqual(['retry_hardware_discovery', undefined]);
    client.dispose();
  });
});

describe('physical placement feedback', () => {
  async function setupPreview() {
    const persistence = new MemoryPersistence();
    const hardware = new FakeHardware();
    const store = new AppStore(persistence, new FakeOutput(), hardware);
    stores.push(store);
    await store.load();
    await store.requestTransition('rooms');
    store.addPhysicalLight(id);
    return {
      persistence,
      hardware,
      store,
      logicalId: store.getSnapshot().selectedLightId!,
    };
  }

  it('previews a new physical light and clamped edits without committing the draft', async () => {
    const { persistence, hardware, store, logicalId } = await setupPreview();
    expect(hardware.previews.at(-1)).toEqual({
      deviceId: id,
      position: { x: -1.7, y: 1.2, z: 1 },
      mode: 'location',
    });
    store.moveLight(logicalId, { x: 9, y: 3, z: 2 });
    expect(hardware.previews.at(-1)).toEqual({
      deviceId: id,
      position: { x: 3, y: 1.2, z: 2 },
      mode: 'location',
    });
    store.setEditMode('height');
    store.moveLight(logicalId, { x: 0, y: 9 });
    expect(hardware.previews.at(-1)).toEqual({
      deviceId: id,
      position: { x: 3, y: 3, z: 2 },
      mode: 'height',
    });
    expect(persistence.calls).toHaveLength(0);
    expect(store.getSnapshot().saved!.rooms[0].lights).toHaveLength(0);
    expect(store.getSnapshot().running).toBe(false);
  });

  it('clears feedback on deselect, virtual selection, save, guards, discard, navigation and stop', async () => {
    const { hardware, store, logicalId } = await setupPreview();
    await store.saveRoom();
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    store.setEditMode('height');
    expect(store.dirty).toBe(false);
    store.selectLight(null);
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    store.addLight();
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    await store.requestTransition('sync');
    expect(hardware.previews.at(-1)).toBeNull();
    expect(store.getSnapshot().pending).toBe('sync');
    await store.resolveTransition('stay');
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    await store.requestTransition('discard');
    await store.resolveTransition('discard');
    expect(hardware.previews.at(-1)).toBeNull();
    expect(store.dirty).toBe(false);
    store.selectLight(logicalId);
    await store.stop();
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    await store.requestTransition('sync');
    expect(hardware.previews.at(-1)).toBeNull();
    store.selectLight(logicalId);
    expect(hardware.previews.at(-1)).toBeNull(); // The Sync page never starts editor feedback.
  });

  it('clears on unbinding/deletion and never previews an offline light or a frozen edit', async () => {
    const { hardware, store, persistence, logicalId } = await setupPreview();
    store.bindLight(logicalId, null);
    expect(hardware.previews.at(-1)).toBeNull();
    store.bindLight(logicalId, id);
    hardware.update(false);
    store.selectLight(logicalId);
    expect(hardware.previews.at(-1)).toBeNull();
    hardware.update(true);
    store.selectLight(logicalId);
    const gate = deferred();
    persistence.gate = gate.promise;
    const saving = store.saveRoom();
    store.moveLight(logicalId, { x: 3 });
    expect(hardware.previews.at(-1)).toBeNull();
    gate.resolve();
    await saving;
    store.selectLight(logicalId);
    store.deleteSelected();
    expect(hardware.previews.at(-1)).toBeNull();
  });
});
