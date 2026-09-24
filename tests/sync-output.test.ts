import { describe, expect, it, vi } from 'vitest';
import { NativeSyncOutput, type SyncSnapshot } from '../src/sync/output';
import { deferred } from './helpers';

const initial: SyncSnapshot = {
  sequence: 0,
  source: 'test',
  status: 'stopped',
  message: 'Ready',
  colors: [],
};
describe('passive native color output', () => {
  it('subscribes once, ignores old snapshots, and paints final RGB8 without a second brightness or React update', async () => {
    let receive!: (snapshot: SyncSnapshot) => void;
    const unlisten = vi.fn();
    const listen = vi.fn(async (callback) => {
      receive = callback;
      return unlisten;
    });
    const output = new NativeSyncOutput(true, {
      listen,
      invoke: async <T>() => initial as T,
    });
    const changed = vi.fn();
    output.subscribe(changed);
    await Promise.all([output.connect(), output.connect()]);
    expect(listen).toHaveBeenCalledTimes(1);
    receive({
      ...initial,
      sequence: 2,
      status: 'running',
      colors: [{ id: 'a', rgb: [188, 0, 255] }],
    });
    expect(output.getColor('a')).toEqual([188 / 255, 0, 1]);
    const count = changed.mock.calls.length;
    receive({
      ...initial,
      sequence: 3,
      status: 'running',
      colors: [{ id: 'a', rgb: [200, 0, 255] }],
    });
    expect(changed).toHaveBeenCalledTimes(count);
    receive({ ...initial, sequence: 1 });
    expect(output.getColor('a')).toEqual([200 / 255, 0, 1]);
    output.dispose();
    expect(unlisten).toHaveBeenCalledOnce();
  });
  it('start only sends a source: draft positions and camera transforms cannot cross this boundary', async () => {
    const invoke = vi.fn(
      async <T>(command: string) =>
        ({ ...initial, sequence: command === 'start_sync' ? 1 : 2 }) as T,
    );
    const output = new NativeSyncOutput(true, {
      listen: async () => () => {},
      invoke: invoke as import('../src/sync/output').SyncTransport['invoke'],
    });
    await output.start('test');
    await output.stop();
    expect(invoke).toHaveBeenCalledWith('start_sync', {
      source: 'test',
      reducedMotion: false,
    });
    expect(invoke).toHaveBeenCalledWith('stop_sync');
  });
  it('cleans up a listener that arrives after unmount', async () => {
    const gate = deferred();
    const unlisten = vi.fn();
    const invoke = vi.fn();
    const output = new NativeSyncOutput(true, {
      listen: async () => {
        await gate.promise;
        return unlisten;
      },
      invoke: invoke as import('../src/sync/output').SyncTransport['invoke'],
    });
    const connecting = output.connect();
    output.dispose();
    gate.resolve();
    await connecting;
    expect(unlisten).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
  });
  it('does not pretend browser preview implements native sync', async () => {
    const output = new NativeSyncOutput(false);
    await output.connect();
    await expect(output.start('test')).rejects.toThrow('desktop');
  });
});
