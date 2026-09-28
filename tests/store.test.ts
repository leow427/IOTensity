import { afterEach, describe, expect, it, vi } from 'vitest';
import fixture from './fixtures/configuration.json';
import { AppStore } from '../src/state/store';
import type { Configuration } from '../src/domain/model';
import { deferred, FakeOutput, MemoryPersistence } from './helpers';

const stores: AppStore[] = [];
async function setup(saved?: Configuration) {
  const persistence = new MemoryPersistence(saved);
  const store = new AppStore(persistence, new FakeOutput());
  stores.push(store);
  await store.load();
  return { store, persistence };
}
afterEach(() => stores.splice(0).forEach((store) => store.dispose()));

describe('draft / saved / runtime ownership', () => {
  it('creates and selects lights by stable ID and selects a neighbour after deletion', async () => {
    const { store } = await setup();
    store.addLight();
    const first = store.getSnapshot().selectedLightId!;
    store.addLight();
    const second = store.getSnapshot().selectedLightId!;
    expect(first).not.toBe(second);
    expect(store.getSnapshot().draft!.lights).toHaveLength(2);
    store.deleteSelected();
    expect(store.getSnapshot().selectedLightId).toBe(first);
    expect(store.getSnapshot().saved!.rooms[0].lights).toHaveLength(0);
    store.deleteSelected();
    expect(store.getSnapshot().selectedLightId).toBeNull();
    expect(store.dirty).toBe(false);
  });
  it('selection, tabs and native output never dirty the room; reverting edits clears dirty', async () => {
    const { store } = await setup(fixture as Configuration);
    const id = 'light-bulb';
    store.selectLight(id);
    store.setEditMode('height');
    await store.start();
    await store.stop();
    expect(store.dirty).toBe(false);
    store.updateLight(id, { iconKind: 'strip' });
    expect(store.dirty).toBe(true);
    store.updateLight(id, { iconKind: 'bulb' });
    expect(store.dirty).toBe(false);
    store.moveLight(id, { y: 2 });
    expect(store.getSnapshot().saved!.rooms[0].lights[0].position.y).toBe(1.2);
  });
  it('failed save preserves the draft and Sync snapshot, then succeeds on retry', async () => {
    const { store, persistence } = await setup();
    store.addLight();
    persistence.error = new Error('Disk is read-only');
    expect(await store.saveRoom()).toBe(false);
    expect(store.getSnapshot().saveError).toContain('read-only');
    expect(store.dirty).toBe(true);
    expect(store.getSnapshot().saved!.rooms[0].lights).toHaveLength(0);
    persistence.error = null;
    expect(await store.saveRoom()).toBe(true);
    expect(store.dirty).toBe(false);
    expect(persistence.config.rooms[0].lights).toHaveLength(1);
  });
  it('freezes all editing and rejects duplicate submission while a snapshot is in flight', async () => {
    const { store, persistence } = await setup();
    store.addLight();
    const gate = deferred();
    persistence.gate = gate.promise;
    const id = store.getSnapshot().selectedLightId!;
    const save = store.saveRoom();
    store.updateLight(id, { name: 'Newer edit' });
    store.addLight();
    store.deleteSelected();
    store.moveLight(id, { x: 3 });
    expect(await store.saveRoom()).toBe(false);
    expect(store.getSnapshot().draft!.lights).toHaveLength(1);
    expect(store.getSnapshot().draft!.lights[0].name).toBe('Light 1');
    gate.resolve();
    await save;
    expect(store.dirty).toBe(false);
    expect(persistence.calls).toHaveLength(1);
  });
  it('serializes preference writes from committed rooms and never leaks an unsaved draft', async () => {
    vi.useFakeTimers();
    const { store, persistence } = await setup();
    store.addLight();
    store.setPreferences({ brightness: 31 });
    await store.savePreferences();
    expect(persistence.config.rooms[0].lights).toHaveLength(0);
    expect(persistence.config.preferences.brightness).toBe(31);
    await store.saveRoom();
    store.updateLight(store.getSnapshot().selectedLightId!, {
      name: 'Uncommitted',
    });
    store.setPreferences({ intensity: 'punch' });
    await store.savePreferences();
    expect(persistence.config.rooms[0].lights[0].name).toBe('Light 1');
    expect(store.getSnapshot().draft!.lights[0].name).toBe('Uncommitted');
  });
  it('an older queued preference save cannot overwrite a newer committed room or newer preference', async () => {
    vi.useFakeTimers();
    const { store, persistence } = await setup();
    const gate = deferred();
    persistence.gate = gate.promise;
    store.setPreferences({ brightness: 20 });
    const old = store.savePreferences();
    store.addLight();
    const room = store.saveRoom();
    store.setPreferences({ brightness: 80 });
    const newer = store.savePreferences();
    gate.resolve();
    await Promise.all([old, room, newer]);
    expect(persistence.config.preferences.brightness).toBe(80);
    expect(persistence.config.rooms[0].lights).toHaveLength(1);
    expect(persistence.config.revision).toBe(3);
    expect(store.getSnapshot().preferences.brightness).toBe(80);
  });
  it('restores names, appearances and positions on discard and reload, with native output stopped', async () => {
    const { store, persistence } = await setup(fixture as Configuration);
    store.updateLight('light-bar', { name: 'Saved name', iconKind: 'lamp' });
    await store.saveRoom();
    store.updateLight('light-bar', { name: 'Discard me', iconKind: 'strip' });
    await store.requestTransition('discard');
    expect(store.getSnapshot().pending).toBe('discard');
    await store.resolveTransition('discard');
    expect(store.getSnapshot().draft!.lights[1].iconKind).toBe('lamp');
    const reload = new AppStore(persistence, new FakeOutput());
    stores.push(reload);
    await reload.load();
    expect(reload.getSnapshot().draft!.lights[1].name).toBe('Saved name');
    expect(reload.getSnapshot().running).toBe(false);
    expect(reload.getSnapshot().selectedLightId).toBeNull();
  });
  it('offers Save / Discard / Stay on navigation and keeps the editor open after save failure', async () => {
    const { store, persistence } = await setup();
    await store.requestTransition('rooms');
    store.addLight();
    await store.requestTransition('sync');
    expect(store.getSnapshot().pending).toBe('sync');
    await store.resolveTransition('stay');
    expect(store.getSnapshot().page).toBe('rooms');
    await store.requestTransition('sync');
    persistence.error = new Error('Save failed');
    await store.resolveTransition('save');
    expect(store.getSnapshot().page).toBe('rooms');
    expect(store.dirty).toBe(true);
    persistence.error = null;
    await store.requestTransition('sync');
    await store.resolveTransition('save');
    expect(store.getSnapshot().page).toBe('sync');
    expect(store.getSnapshot().saved!.rooms[0].lights).toHaveLength(1);
  });
  it('guards normal close, flushes preferences and leaves an unchanged native output running across navigation', async () => {
    const { store } = await setup(fixture as Configuration);
    await store.start();
    await store.requestTransition('rooms');
    expect(store.getSnapshot().running).toBe(true);
    store.addLight();
    await store.requestTransition('close');
    expect(store.getSnapshot().readyToClose).toBe(false);
    await store.resolveTransition('stay');
    store.setPreferences({ brightness: 25 });
    await store.requestTransition('close');
    await store.resolveTransition('discard');
    expect(store.getSnapshot().readyToClose).toBe(true);
    expect(store.getSnapshot().saved!.preferences.brightness).toBe(25);
  });
  it('surfaces load errors without allowing defaults to overwrite data', async () => {
    const persistence = new MemoryPersistence();
    persistence.error = new Error('Unsupported version');
    const store = new AppStore(persistence);
    stores.push(store);
    await store.load();
    expect(store.getSnapshot().phase).toBe('error');
    expect(store.getSnapshot().saved).toBeNull();
    store.addLight();
    expect(await store.saveRoom()).toBe(false);
    expect(persistence.calls).toHaveLength(0);
  });
  it.each(['save', 'discard', 'stay'] as const)(
    'guards room edits made during a close flush and preserves the %s choice',
    async (choice) => {
      const { store, persistence } = await setup();
      const gate = deferred();
      persistence.gate = gate.promise;
      store.setPreferences({ brightness: 25 });
      const closing = store.requestTransition('close');
      await Promise.resolve();
      store.addLight();
      gate.resolve();
      await closing;
      expect(store.getSnapshot().readyToClose).toBe(false);
      expect(store.getSnapshot().pending).toBe('close');
      expect(store.dirty).toBe(true);
      await store.resolveTransition(choice);
      expect(store.getSnapshot().readyToClose).toBe(choice !== 'stay');
      expect(persistence.config.rooms[0].lights).toHaveLength(
        choice === 'save' ? 1 : 0,
      );
      expect(store.dirty).toBe(choice === 'stay');
    },
  );
  it('flushes preferences edited during close and blocks edits once close is authorized', async () => {
    const { store, persistence } = await setup();
    const gate = deferred();
    persistence.gate = gate.promise;
    store.setPreferences({ brightness: 25 });
    const closing = store.requestTransition('close');
    await Promise.resolve();
    store.setPreferences({ brightness: 80 });
    gate.resolve();
    await closing;
    expect(
      persistence.calls.map((call) => call.preferences.brightness),
    ).toEqual([25, 80]);
    expect(persistence.config.preferences.brightness).toBe(80);
    expect(store.getSnapshot().readyToClose).toBe(true);
    store.addLight();
    store.setPreferences({ brightness: 10 });
    expect(store.dirty).toBe(false);
    expect(store.getSnapshot().preferences.brightness).toBe(80);
  });
  it.each([false, true])(
    'Stay cancels an earlier close flush even when the intervening edit is reverted: %s',
    async (revert) => {
      const { store, persistence } = await setup();
      const gate = deferred();
      persistence.gate = gate.promise;
      store.setPreferences({ brightness: 25 });
      const closing = store.requestTransition('close');
      await Promise.resolve();
      store.addLight();
      await store.requestTransition('close');
      await store.resolveTransition('stay');
      if (revert) store.deleteSelected();
      gate.resolve();
      await closing;
      expect(store.getSnapshot().readyToClose).toBe(false);
      expect(store.getSnapshot().pending).toBeNull();
      expect(store.dirty).toBe(!revert);
    },
  );
  it('honors a fresh close requested after Stay while the canceled flush is still pending', async () => {
    const { store, persistence } = await setup();
    const gate = deferred();
    persistence.gate = gate.promise;
    store.setPreferences({ brightness: 25 });
    const canceled = store.requestTransition('close');
    await Promise.resolve();
    store.addLight();
    await store.requestTransition('close');
    await store.resolveTransition('stay');
    await store.requestTransition('close');
    const confirmed = store.resolveTransition('discard');
    gate.resolve();
    await Promise.all([canceled, confirmed]);
    expect(store.getSnapshot().readyToClose).toBe(true);
    expect(store.getSnapshot().pending).toBeNull();
    expect(store.dirty).toBe(false);
  });
  it('keeps close blocked when its preference flush fails', async () => {
    const { store, persistence } = await setup();
    store.setPreferences({ brightness: 25 });
    persistence.error = new Error('Disk full');
    await store.requestTransition('close');
    expect(store.getSnapshot().readyToClose).toBe(false);
    expect(store.getSnapshot().preferenceError).toContain('Disk full');
    persistence.error = null;
    await store.requestTransition('close');
    expect(store.getSnapshot().readyToClose).toBe(true);
    expect(store.getSnapshot().preferenceError).toBeNull();
  });
  it('clears an older failed preference write when the newer queued write succeeds', async () => {
    const { store, persistence } = await setup();
    const gate = deferred();
    vi.spyOn(persistence, 'save').mockImplementationOnce(async () => {
      await gate.promise;
      throw new Error('Temporary failure');
    });
    store.setPreferences({ brightness: 25 });
    const old = store.savePreferences();
    store.setPreferences({ brightness: 80 });
    const newer = store.savePreferences();
    gate.resolve();
    expect(await old).toBe(false);
    expect(await newer).toBe(true);
    expect(persistence.config.preferences.brightness).toBe(80);
    expect(store.getSnapshot().preferenceError).toBeNull();
  });
  it('forwards live Reduce Motion changes to the running output', async () => {
    const { store } = await setup(fixture as Configuration);
    const update = vi.spyOn(store.output, 'setReducedMotion');
    await store.start();
    store.setReducedMotion(true);
    store.setReducedMotion(false);
    expect(update.mock.calls).toEqual([[true], [false]]);
    expect(store.getSnapshot().running).toBe(true);
  });
});
