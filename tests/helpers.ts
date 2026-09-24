import {
  clone,
  defaultConfiguration,
  type Configuration,
} from '../src/domain/model';
import type { Persistence } from '../src/persistence/client';
import { NativeSyncOutput, type SyncSnapshot } from '../src/sync/output';

export class MemoryPersistence implements Persistence {
  readonly kind = 'native' as const;
  config: Configuration;
  calls: Configuration[] = [];
  error: Error | null = null;
  gate: Promise<void> | null = null;
  constructor(config = defaultConfiguration()) {
    this.config = clone(config);
  }
  async load() {
    if (this.error) throw this.error;
    return clone(this.config);
  }
  async save(config: Configuration, expectedRevision: number) {
    this.calls.push(clone(config));
    if (this.gate) await this.gate;
    if (this.error) throw this.error;
    if (expectedRevision !== this.config.revision)
      throw new Error('Stale revision');
    this.config = { ...clone(config), revision: expectedRevision + 1 };
    // Match Rust's field order, which is intentionally different from createLight.
    this.config.rooms.forEach((room) => {
      room.lights = room.lights.map(
        ({ id, name, position, iconKind, output }) => ({
          id,
          name,
          position,
          iconKind,
          output,
        }),
      );
    });
    return clone(this.config);
  }
}
export class FakeOutput extends NativeSyncOutput {
  constructor() {
    let snapshot: SyncSnapshot = {
      sequence: 0,
      source: 'test',
      status: 'stopped',
      message: 'Ready',
      colors: [],
    };
    super(true, {
      listen: async () => () => {},
      invoke: async <T>(command: string) => {
        if (command === 'start_sync')
          snapshot = {
            ...snapshot,
            sequence: snapshot.sequence + 1,
            status: 'running',
          };
        if (command === 'stop_sync')
          snapshot = {
            ...snapshot,
            sequence: snapshot.sequence + 1,
            status: 'stopped',
          };
        return snapshot as T;
      },
    });
  }
}
export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
