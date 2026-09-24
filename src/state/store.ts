import {
  clone,
  createLight,
  MAX_LIGHTS,
  moveLight,
  roomIsDirty,
  validateConfiguration,
  type Configuration,
  type EditMode,
  type IconKind,
  type Position,
  type Preferences,
  type Room,
  validDeviceId,
  shortDeviceId,
} from '../domain/model';
import { errorMessage, type Persistence } from '../persistence/client';
import {
  NativeSyncOutput,
  type SyncOutput,
  type SyncSource,
  type SyncStatus,
} from '../sync/output';

import {
  NativeHardwareClient,
  type HardwareClient,
  type Device,
} from '../hardware/client';

export type Page = 'sync' | 'rooms';
export type Destination = Page | 'close' | 'discard';
export type AppState = {
  phase: 'loading' | 'ready' | 'error';
  loadError: string | null;
  saved: Configuration | null;
  draft: Room | null;
  preferences: Preferences;
  selectedLightId: string | null;
  editMode: EditMode;
  page: Page;
  saveStatus: 'idle' | 'saving' | 'saved' | 'error';
  saveError: string | null;
  preferenceError: string | null;
  preferenceSaving: boolean;
  running: boolean;
  devices: Device[];
  discoveryError: string | null;
  hardwareError: string | null;
  identifying: string | null;
  syncSource: SyncSource;
  syncStatus: SyncStatus;
  syncMessage: string;
  syncBusy: boolean;
  reducedMotion: boolean;
  pending: Destination | null;
  readyToClose: boolean;
};

export class AppStore {
  private state: AppState = {
    phase: 'loading',
    loadError: null,
    saved: null,
    draft: null,
    preferences: { brightness: 75, intensity: 'balanced' },
    selectedLightId: null,
    editMode: 'location',
    page: 'sync',
    saveStatus: 'idle',
    saveError: null,
    preferenceError: null,
    preferenceSaving: false,
    running: false,
    devices: [],
    discoveryError: null,
    hardwareError: null,
    identifying: null,
    syncSource: 'simulation',
    syncStatus: 'stopped',
    syncMessage: 'Ready to start.',
    syncBusy: false,
    reducedMotion: false,
    pending: null,
    readyToClose: false,
  };
  private listeners = new Set<() => void>();
  private writes: Promise<unknown> = Promise.resolve();
  private preferenceTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingPreferenceWrites = 0;
  private disposed = false;

  constructor(
    readonly persistence: Persistence,
    readonly output: SyncOutput = new NativeSyncOutput(),
    readonly hardware: HardwareClient = new NativeHardwareClient(),
  ) {
    output.subscribe(() => {
      const snapshot = output.getSnapshot();
      this.set({
        running: snapshot.status === 'running',
        syncStatus: snapshot.status,
        syncMessage: snapshot.message,
        ...(['starting', 'running'].includes(snapshot.status)
          ? { syncSource: snapshot.source }
          : {}),
      });
    });
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private set(patch: Partial<AppState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener());
  }
  get dirty() {
    return (
      !!this.state.draft &&
      !!this.state.saved &&
      roomIsDirty(this.state.draft, this.state.saved.rooms[0])
    );
  }
  get canEdit() {
    return this.state.phase === 'ready' && this.state.saveStatus !== 'saving';
  }

  async load() {
    this.set({ phase: 'loading', loadError: null });
    try {
      const saved = await this.persistence.load();
      validateConfiguration(saved);
      this.set({
        phase: 'ready',
        saved,
        draft: clone(saved.rooms[0]),
        preferences: clone(saved.preferences),
        selectedLightId: null,
      });
      void this.hardware
        .connect(({ devices, discoveryError }) =>
          this.set({ devices, discoveryError }),
        )
        .catch((error) => this.set({ discoveryError: errorMessage(error) }));
      try {
        await this.output.connect();
      } catch (error) {
        this.set({ syncStatus: 'error', syncMessage: errorMessage(error) });
      }
    } catch (error) {
      this.set({ phase: 'error', loadError: errorMessage(error) });
    }
  }
  private edit(draft: Room, selectedLightId = this.state.selectedLightId) {
    if (!this.canEdit) return;
    this.set({ draft, selectedLightId, saveStatus: 'idle', saveError: null });
  }
  addLight() {
    const room = this.state.draft;
    if (!room || !this.canEdit || room.lights.length >= MAX_LIGHTS) return;
    const light = createLight(room);
    this.edit({ ...room, lights: [...room.lights, light] }, light.id);
  }
  private deviceAlreadyBound(deviceId: string, exceptId?: string) {
    const lights = [
      ...(this.state.draft?.lights ?? []),
      ...(this.state.saved?.rooms.slice(1).flatMap((r) => r.lights) ?? []),
    ];
    return lights.some(
      (light) =>
        light.id !== exceptId &&
        light.output.kind === 'esp32' &&
        light.output.deviceId === deviceId,
    );
  }
  bindLight(id: string, deviceId: string | null): boolean {
    const room = this.state.draft;
    if (!room || !this.canEdit || !room.lights.some((light) => light.id === id))
      return false;
    if (
      deviceId !== null &&
      (!validDeviceId(deviceId) || this.deviceAlreadyBound(deviceId, id))
    ) {
      this.set({
        hardwareError:
          'This hardware ID is invalid or already assigned to another light.',
      });
      return false;
    }
    this.edit({
      ...room,
      lights: room.lights.map((light) =>
        light.id === id
          ? {
              ...light,
              output: deviceId
                ? { kind: 'esp32', deviceId }
                : { kind: 'virtual' },
            }
          : light,
      ),
    });
    this.set({ hardwareError: null });
    return true;
  }
  addPhysicalLight(deviceId: string): boolean {
    const room = this.state.draft;
    if (!room || !this.canEdit || room.lights.length >= MAX_LIGHTS)
      return false;
    if (!validDeviceId(deviceId) || this.deviceAlreadyBound(deviceId)) {
      this.set({
        hardwareError:
          'This hardware ID is invalid or already assigned to another light.',
      });
      return false;
    }
    const light = {
      ...createLight(room),
      name: shortDeviceId(deviceId),
      output: { kind: 'esp32' as const, deviceId },
    };
    this.edit({ ...room, lights: [...room.lights, light] }, light.id);
    this.set({ hardwareError: null });
    return true;
  }
  async identifyDevice(deviceId: string) {
    if (this.state.identifying) return;
    this.set({ identifying: deviceId, hardwareError: null });
    try {
      await this.hardware.identify(deviceId);
    } catch (error) {
      this.set({ hardwareError: errorMessage(error) });
    } finally {
      this.set({ identifying: null });
    }
  }
  selectLight(id: string | null) {
    if (
      id !== null &&
      !this.state.draft?.lights.some((light) => light.id === id)
    )
      return;
    this.set({ selectedLightId: id });
  }
  setEditMode(editMode: EditMode) {
    this.set({ editMode });
  }
  updateLight(id: string, patch: { name?: string; iconKind?: IconKind }) {
    const room = this.state.draft;
    if (!room) return;
    this.edit({
      ...room,
      lights: room.lights.map((light) =>
        light.id === id ? { ...light, ...patch } : light,
      ),
    });
  }
  moveLight(id: string, position: Partial<Position>) {
    const room = this.state.draft;
    if (!room) return;
    this.edit({
      ...room,
      lights: room.lights.map((light) =>
        light.id === id
          ? {
              ...light,
              position: moveLight(
                light.position,
                this.state.editMode,
                position,
              ),
            }
          : light,
      ),
    });
  }
  deleteSelected() {
    const room = this.state.draft;
    if (!room || !this.state.selectedLightId) return;
    const index = room.lights.findIndex(
      (light) => light.id === this.state.selectedLightId,
    );
    const lights = room.lights.filter(
      (light) => light.id !== this.state.selectedLightId,
    );
    this.edit(
      { ...room, lights },
      lights[Math.min(index, lights.length - 1)]?.id ?? null,
    );
  }
  private discard() {
    if (!this.state.saved) return;
    const draft = clone(this.state.saved.rooms[0]);
    const selectedLightId = draft.lights.some(
      (light) => light.id === this.state.selectedLightId,
    )
      ? this.state.selectedLightId
      : null;
    this.set({ draft, selectedLightId, saveStatus: 'idle', saveError: null });
  }

  // Every transaction is built from the last acknowledged configuration at execution time.
  // The queue serializes preferences and room snapshots; the native revision also rejects stale writers.
  private commit(change: (config: Configuration) => void): Promise<void> {
    const task = this.writes.then(async () => {
      if (!this.state.saved)
        throw new Error('Load your configuration before saving.');
      const config = clone(this.state.saved);
      change(config);
      validateConfiguration(config);
      const result = await this.persistence.save(
        config,
        this.state.saved.revision,
      );
      validateConfiguration(result);
      this.set({ saved: result });
    });
    this.writes = task.catch(() => undefined);
    return task;
  }
  async saveRoom(): Promise<boolean> {
    if (!this.state.draft || !this.canEdit) return false;
    if (!this.dirty) return true;
    const snapshot = clone(this.state.draft);
    this.set({ saveStatus: 'saving', saveError: null });
    try {
      await this.commit((config) => {
        config.rooms[0] = snapshot;
      });
      this.set({ saveStatus: 'saved' });
      return true;
    } catch (error) {
      this.set({ saveStatus: 'error', saveError: errorMessage(error) });
      return false;
    }
  }
  setPreferences(patch: Partial<Preferences>) {
    if (this.state.phase !== 'ready') return;
    const preferences = { ...this.state.preferences, ...patch };
    if (patch.brightness !== undefined)
      preferences.brightness = Math.max(
        0,
        Math.min(100, Math.round(patch.brightness)),
      );
    this.set({ preferences, preferenceError: null });

    if (this.preferenceTimer) clearTimeout(this.preferenceTimer);
    this.preferenceTimer = setTimeout(() => {
      this.preferenceTimer = null;
      void this.savePreferences();
    }, 400);
  }
  async savePreferences(): Promise<boolean> {
    if (this.preferenceTimer) clearTimeout(this.preferenceTimer);
    this.preferenceTimer = null;
    if (!this.state.saved) return false;
    const snapshot = clone(this.state.preferences);
    if (
      !this.pendingPreferenceWrites &&
      JSON.stringify(snapshot) === JSON.stringify(this.state.saved.preferences)
    )
      return true;
    this.pendingPreferenceWrites++;
    this.set({ preferenceSaving: true, preferenceError: null });
    try {
      await this.commit((config) => {
        config.preferences = snapshot;
      });
      return true;
    } catch (error) {
      this.set({ preferenceError: errorMessage(error) });
      return false;
    } finally {
      this.pendingPreferenceWrites--;
      this.set({ preferenceSaving: this.pendingPreferenceWrites > 0 });
    }
  }
  setReducedMotion(reducedMotion: boolean) {
    this.set({ reducedMotion });
  }
  setSyncSource(syncSource: SyncSource) {
    if (
      this.state.syncBusy ||
      ['starting', 'running', 'stopping'].includes(this.state.syncStatus)
    )
      return;
    this.set({ syncSource });
  }
  async start() {
    if (this.state.syncBusy || !this.state.saved?.rooms[0].lights.length)
      return;
    this.set({ syncBusy: true });
    try {
      await this.output.start(this.state.syncSource, this.state.reducedMotion);
    } catch (error) {
      this.set({
        syncStatus: 'error',
        syncMessage: typeof error === 'string' ? error : errorMessage(error),
      });
    } finally {
      this.set({ syncBusy: false });
    }
  }
  async stop() {
    if (this.state.syncBusy) return;
    this.set({ syncBusy: true });
    try {
      await this.output.stop();
    } catch (error) {
      this.set({
        syncStatus: 'error',
        syncMessage: typeof error === 'string' ? error : errorMessage(error),
      });
    } finally {
      this.set({ syncBusy: false });
    }
  }

  async requestTransition(destination: Destination) {
    if (destination === this.state.page) return;
    if (this.dirty || this.state.saveStatus === 'saving') {
      this.set({ pending: destination });
      return;
    }
    await this.finishTransition(destination);
  }
  async resolveTransition(choice: 'save' | 'discard' | 'stay') {
    const destination = this.state.pending;
    if (!destination || this.state.saveStatus === 'saving') return;
    if (choice === 'stay') {
      this.set({ pending: null });
      return;
    }
    if (choice === 'save' && !(await this.saveRoom())) {
      this.set({ pending: null, page: 'rooms' });
      return;
    }
    if (choice === 'discard') this.discard();
    this.set({ pending: null });
    await this.finishTransition(destination);
  }
  private async finishTransition(destination: Destination) {
    if (destination === 'close') {
      if (this.state.phase === 'error' || (await this.savePreferences()))
        this.set({ readyToClose: true });
    } else if (destination !== 'discard') this.set({ page: destination });
  }
  closeFailed(error: unknown) {
    this.set({ readyToClose: false, preferenceError: errorMessage(error) });
  }
  dispose() {
    this.disposed = true;
    if (this.preferenceTimer) clearTimeout(this.preferenceTimer);
    this.output.dispose();
    this.hardware.dispose();
    this.listeners.clear();
  }
}
