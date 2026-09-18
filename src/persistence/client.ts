import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  clone,
  defaultConfiguration,
  validateConfiguration,
  type Configuration,
} from '../domain/model';

export interface Persistence {
  readonly kind: 'native' | 'preview';
  load(): Promise<Configuration>;
  save(config: Configuration, expectedRevision: number): Promise<Configuration>;
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof error.message === 'string'
  )
    return error.message;
  return 'The configuration could not be saved. Check your application data folder permissions and try again.';
}

export class NativePersistence implements Persistence {
  readonly kind = 'native' as const;
  async load() {
    const result: unknown = await invoke('load_config');
    validateConfiguration(result);
    return result;
  }
  async save(config: Configuration, expectedRevision: number) {
    validateConfiguration(config);
    const result: unknown = await invoke('save_config', {
      config,
      expectedRevision,
    });
    validateConfiguration(result);
    if (result.revision !== expectedRevision + 1)
      throw new Error(
        'Unexpected save acknowledgement. Reload the app before trying again.',
      );
    return result;
  }
}

// Explicitly labelled, volatile browser preview. Disk persistence only exists in Tauri.
export class PreviewPersistence implements Persistence {
  readonly kind = 'preview' as const;
  private config = defaultConfiguration();
  async load() {
    return clone(this.config);
  }
  async save(config: Configuration, expectedRevision: number) {
    validateConfiguration(config);
    if (this.config.revision !== expectedRevision)
      throw new Error('Configuration changed. Reload before saving.');
    this.config = { ...clone(config), revision: expectedRevision + 1 };
    return clone(this.config);
  }
}
export const createPersistence = (): Persistence =>
  isTauri() ? new NativePersistence() : new PreviewPersistence();
