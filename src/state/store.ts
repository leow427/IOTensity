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
} from '../domain/model';
import { errorMessage, type Persistence } from '../persistence/client';
import { Simulation } from '../simulation/engine';

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
    readonly simulation = new Simulation(),
  ) {}
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
      this.configureSimulation();
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
      this.configureSimulation();
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
    this.configureSimulation();
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
  private configureSimulation() {
    const ids =
      this.state.saved?.rooms.flatMap((room) =>
        room.lights.map((light) => light.id),
      ) ?? [];
    this.simulation.configure(
      ids,
      this.state.preferences.intensity,
      this.state.reducedMotion,
    );
    if (!ids.length) this.stop();
  }
  setReducedMotion(reducedMotion: boolean) {
    this.set({ reducedMotion });
    this.configureSimulation();
  }
  start() {
    if (!this.state.saved?.rooms.some((room) => room.lights.length)) return;
    this.simulation.start();
    this.set({ running: this.simulation.isRunning });
  }
  stop() {
    this.simulation.stop();
    this.set({ running: false });
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
    this.simulation.dispose();
    this.listeners.clear();
  }
}
