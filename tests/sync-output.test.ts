import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useOutputInvalidation } from '../src/scene/invalidation';
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
  it('requests a demand-rendered scene frame only when a newer snapshot changes colors', async () => {
    let receive!: (snapshot: SyncSnapshot) => void;
    const output = new NativeSyncOutput(true, {
      listen: async (callback) => {
        receive = callback;
        return () => {};
      },
      invoke: async <T>() => initial as T,
    });
    await output.connect();
    const invalidate = vi.fn();
    const { unmount } = renderHook(() =>
      useOutputInvalidation(output, invalidate),
    );
    const colors = [
      { id: 'a', rgb: [188, 0, 255] as [number, number, number] },
    ];
    receive({ ...initial, sequence: 1, status: 'running', colors });
    expect(invalidate).toHaveBeenCalledTimes(1);
    // Static colors repeat with fresh sequences; old snapshots are rejected.
    receive({ ...initial, sequence: 2, status: 'running', colors });
    receive({ ...initial, sequence: 1, colors: [] });
    expect(invalidate).toHaveBeenCalledTimes(1);
    receive({
      ...initial,
      sequence: 3,
      status: 'running',
      colors: [{ id: 'a', rgb: [188, 1, 255] }],
    });
    expect(invalidate).toHaveBeenCalledTimes(2);
    unmount();
    receive({ ...initial, sequence: 4, colors: [] });
    expect(invalidate).toHaveBeenCalledTimes(2);
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
  it('orders live Reduce Motion and Stop after an in-flight Start', async () => {
    const gate = deferred();
    const calls: unknown[] = [];
    const output = new NativeSyncOutput(true, {
      listen: async () => () => {},
      invoke: async <T>(command: string, args?: Record<string, unknown>) => {
        calls.push([command, args]);
        if (command === 'start_sync') await gate.promise;
        return initial as T;
      },
    });
    await output.connect();
    const starting = output.start('simulation');
    const changing = output.setReducedMotion(true);
    const stopping = output.stop();
    await Promise.resolve();
    await Promise.resolve();
    expect(calls).toEqual([
      ['sync_snapshot', undefined],
      ['start_sync', { source: 'simulation', reducedMotion: false }],
    ]);
    gate.resolve();
    await Promise.all([starting, changing, stopping]);
    expect(calls.slice(2)).toEqual([
      ['set_reduced_motion', { reducedMotion: true }],
      ['stop_sync', undefined],
    ]);
  });
});
