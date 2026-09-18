import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import fixture from './fixtures/configuration.json';
import { NativePersistence } from '../src/persistence/client';
import type { Configuration } from '../src/domain/model';

afterEach(clearMocks);
describe('typed Tauri boundary (mocked IPC)', () => {
  it('sends camelCase command arguments and validates the returned snapshot', async () => {
    mockIPC((command, args) => {
      if (command === 'load_config') return fixture;
      expect(command).toBe('save_config');
      expect(args).toEqual({ config: fixture, expectedRevision: 0 });
      return { ...fixture, revision: 1 };
    });
    const native = new NativePersistence();
    expect(await native.load()).toEqual(fixture);
    expect((await native.save(fixture as Configuration, 0)).revision).toBe(1);
  });
  it('rejects malformed payloads and mismatched acknowledgement revisions', async () => {
    mockIPC(() => ({ ...fixture, revision: 7 }));
    await expect(
      new NativePersistence().save(fixture as Configuration, 0),
    ).rejects.toThrow('acknowledgement');
    mockIPC(() => ({ schemaVersion: 4 }));
    await expect(new NativePersistence().load()).rejects.toThrow();
  });
  it('propagates a native error without pretending that the write succeeded', async () => {
    mockIPC(() => {
      throw new Error('Permission denied');
    });
    await expect(
      new NativePersistence().save(fixture as Configuration, 0),
    ).rejects.toThrow('Permission denied');
  });
});
