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
  type NativeTransport,
} from '../src/hardware/client';

const id = 'esp32-020000a1b2c3';
export class FakeHardware implements HardwareClient {
  available = true;
  received?: (snapshot: DevicesSnapshot) => void;
  identified: string[] = [];
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
    expect(calls).toContainEqual(['identify_device', { deviceId: id }]);
    client.dispose();
  });
});
