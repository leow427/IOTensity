import {
  clone,
  defaultConfiguration,
  type Configuration,
} from '../src/domain/model';
import type { Persistence } from '../src/persistence/client';
import type { AnimationClock } from '../src/simulation/engine';

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
      room.lights = room.lights.map(({ id, name, position, iconKind }) => ({
        id,
        name,
        position,
        iconKind,
      }));
    });
    return clone(this.config);
  }
}
export class FakeClock implements AnimationClock {
  time = 0;
  next = 0;
  callbacks = new Map<number, (time: number) => void>();
  now = () => this.time;
  request = (callback: (time: number) => void) => {
    const id = ++this.next;
    this.callbacks.set(id, callback);
    return id;
  };
  cancel = (id: number) => {
    this.callbacks.delete(id);
  };
  advance(ms: number) {
    this.time += ms;
    const callbacks = [...this.callbacks.values()];
    this.callbacks.clear();
    callbacks.forEach((callback) => callback(this.time));
  }
}
export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
