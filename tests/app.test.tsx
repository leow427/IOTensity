import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import { StoreProvider } from '../src/state/context';
import { AppStore } from '../src/state/store';
import type { HardwareClient, DevicesSnapshot } from '../src/hardware/client';
import type { Room } from '../src/domain/model';
import { FakeOutput, MemoryPersistence } from './helpers';

// Component tests exercise accessible UI and the real store; 3D rendering is
// verified separately in the browser and native app, not claimed by this mock.
vi.mock('../src/scene/RoomScene', () => ({
  RoomScene: ({ room, selectedId }: { room: Room; selectedId?: string }) => (
    <div data-testid="scene" data-selected={selectedId}>
      {room.lights.map((light) => (
        <span key={light.id} data-orb={light.id}>
          {light.name}
        </span>
      ))}
    </div>
  ),
}));

const stores: AppStore[] = [];
async function setup(hardware?: HardwareClient) {
  const persistence = new MemoryPersistence();
  const store = new AppStore(persistence, new FakeOutput(), hardware);
  stores.push(store);
  await store.load();
  render(
    <StoreProvider store={store}>
      <App />
    </StoreProvider>,
  );
  return { store, persistence, user: userEvent.setup() };
}
afterEach(() => stores.splice(0).forEach((store) => store.dispose()));

describe('room editing UI', () => {
  it('keeps physical output failures visible when discovery succeeds and clears them on recovery', async () => {
    let receive!: (snapshot: DevicesSnapshot) => void;
    const outputError = 'Cannot open the output socket. Retrying.';
    await setup({
      available: true,
      async preview() {},
      async identify() {},
      async retryDiscovery() {},
      dispose() {},
      async connect(listener) {
        receive = listener;
        receive({ devices: [], discoveryError: null, outputError });
      },
    });
    expect(screen.getByRole('alert')).toHaveTextContent(outputError);
    act(() => receive({ devices: [], discoveryError: null, outputError }));
    expect(screen.getByRole('alert')).toHaveTextContent(outputError);
    act(() =>
      receive({ devices: [], discoveryError: null, outputError: null }),
    );
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
  it('starts empty and each addition immediately creates exactly one orb and one named bulb card', async () => {
    const { user } = await setup();
    expect(screen.getByRole('button', { name: 'Start Sync' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const card = screen.getByRole('button', { name: 'Select Light 1' });
    expect(card).toHaveAttribute('aria-pressed', 'true');
    expect(card.querySelector('[data-icon="bulb"]')).toBeInTheDocument();
    expect(
      screen.getByTestId('scene').querySelectorAll('[data-orb]'),
    ).toHaveLength(1);
    expect(
      within(
        screen.getByRole('group', { name: 'Virtual lights' }),
      ).getAllByRole('button'),
    ).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    expect(
      screen.getByTestId('scene').querySelectorAll('[data-orb]'),
    ).toHaveLength(2);
    expect(
      screen.getByRole('button', { name: 'Select Light 2' }),
    ).toHaveAttribute('aria-pressed', 'true');
  });
  it('keeps card and orb selection synchronized; name/icon/delete changes affect the correct ID', async () => {
    const { user, store } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const first = store.getSnapshot().selectedLightId!;
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const second = store.getSnapshot().selectedLightId!;
    const firstCard = screen.getByRole('button', { name: 'Select Light 1' });
    firstCard.focus();
    await user.keyboard('{Enter}');
    expect(screen.getByTestId('scene')).toHaveAttribute('data-selected', first);
    await user.clear(screen.getByLabelText('Name'));
    await user.type(screen.getByLabelText('Name'), 'Desk left');
    await user.click(screen.getByRole('button', { name: 'Light Bar' }));
    const renamed = screen.getByRole('button', { name: 'Select Desk left' });
    expect(renamed).toHaveAttribute('data-light-id', first);
    expect(renamed.querySelector('[data-icon="bar"]')).toBeInTheDocument();
    expect(
      screen
        .getByRole('button', { name: 'Select Light 2' })
        .querySelector('[data-icon="bulb"]'),
    ).toBeInTheDocument();
    act(() => store.selectLight(second));
    expect(
      screen.getByRole('button', { name: 'Select Light 2' }),
    ).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByRole('button', { name: 'Delete light' }));
    expect(
      screen.queryByRole('button', { name: 'Select Light 2' }),
    ).not.toBeInTheDocument();
    expect(renamed).toHaveAttribute('aria-pressed', 'true');
  });
  it('limits light names to 64 UTF-8 bytes and explains a missing name inline', async () => {
    const { user, store } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const input = screen.getByLabelText('Name');
    const id = store.getSnapshot().selectedLightId!;
    const name = () =>
      store.getSnapshot().draft!.lights.find((light) => light.id === id)!.name;
    expect(input).not.toHaveAttribute('aria-invalid');
    await user.clear(input);
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription(
      'Enter a name before saving this room.',
    );
    await user.type(input, '   ');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    await user.clear(input);
    await user.paste('灯'.repeat(30));
    expect(name()).toBe('灯'.repeat(21));
    expect(input).toHaveValue('灯'.repeat(21));
    expect(input).not.toHaveAttribute('aria-invalid');
    expect(input).not.toHaveAccessibleDescription();
  });
  it('saves via the keyboard, marks icon edits dirty and restores the saved card on confirmed discard', async () => {
    const { user, persistence } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    await user.click(screen.getByRole('button', { name: 'Lamp' }));
    fireEvent.keyDown(window, { key: 's', metaKey: true });
    await screen.findByText('✓ SAVED TO DISK');
    expect(persistence.config.rooms[0].lights[0].iconKind).toBe('lamp');
    await user.click(screen.getByRole('button', { name: 'LED Strip' }));
    expect(screen.getByText('● UNSAVED CHANGES')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Discard Changes' }));
    await user.click(
      within(screen.getByRole('dialog')).getByRole('button', {
        name: 'Discard Changes',
      }),
    );
    expect(
      screen
        .getByRole('button', { name: 'Select Light 1' })
        .querySelector('[data-icon="lamp"]'),
    ).toBeInTheDocument();
    expect(screen.queryByText('● UNSAVED CHANGES')).not.toBeInTheDocument();
  });
  it('clears the selection with Escape on the page but not while editing a field', async () => {
    const { user, store } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const selected = store.getSnapshot().selectedLightId;
    const name = screen.getByLabelText('Name');
    await user.click(name);
    await user.keyboard('{Escape}');
    expect(store.getSnapshot().selectedLightId).toBe(selected);
    expect(name).toHaveFocus();
    await user.click(screen.getByLabelText('Left / right coordinate'));
    await user.keyboard('{Escape}');
    expect(store.getSnapshot().selectedLightId).toBe(selected);
    screen.getByRole('button', { name: 'Select Light 1' }).focus();
    await user.keyboard('{Escape}');
    expect(store.getSnapshot().selectedLightId).toBeNull();
  });
  it('lets coordinates be typed through partial out-of-range values and clamps on commit', async () => {
    const { user, store } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const id = store.getSnapshot().selectedLightId!;
    const position = () =>
      store.getSnapshot().draft!.lights.find((light) => light.id === id)!
        .position;
    const before = position();
    await user.click(screen.getByRole('tab', { name: 'Height' }));
    const height = screen.getByLabelText('Height coordinate');
    await user.clear(height);
    await user.type(height, '0');
    // "0" is below the floor: kept as typed, not clamped to 0.15 mid-entry.
    expect(height).toHaveValue(0);
    expect(position().y).toBe(before.y);
    await user.type(height, '.5');
    expect(height).toHaveValue(0.5);
    expect(position()).toEqual({ ...before, y: 0.5 });
    await user.tab();
    expect(height).toHaveValue(0.5);

    await user.clear(height);
    await user.type(height, '9');
    expect(position().y).toBe(0.5);
    await user.tab();
    expect(position()).toEqual({ ...before, y: 3 });
    expect(height).toHaveValue(3);

    await user.clear(height);
    await user.type(height, '0.01{Enter}');
    expect(position().y).toBe(0.15);
    expect(height).toHaveValue(0.15);
    expect(height).toHaveFocus();
    await user.clear(height);
    await user.tab();
    expect(height).toHaveValue(0.15);

    await user.click(screen.getByRole('tab', { name: 'Location' }));
    const x = screen.getByLabelText('Left / right coordinate');
    await user.clear(x);
    await user.type(x, '-1.25');
    expect(x).toHaveValue(-1.25);
    expect(position()).toEqual({ ...before, x: -1.25, y: 0.15 });
  });
  it('uses an accessible navigation dialog and displays failed saves with the draft intact', async () => {
    const { user, persistence } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    await user.click(screen.getByRole('button', { name: 'Sync' }));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByRole('button', { name: 'Stay' })).toHaveFocus();
    persistence.error = new Error('Disk is full');
    await user.click(within(dialog).getByRole('button', { name: 'Save Room' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Disk is full');
    expect(
      screen.getByRole('heading', { name: /Your Rooms/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Select Light 1' }),
    ).toBeInTheDocument();
  });
  it('offers a keyboard-accessible close without saving after a failed close-time preference flush', async () => {
    const { store, persistence, user } = await setup();
    act(() => store.setPreferences({ brightness: 25 }));
    persistence.error = new Error('Revision conflict');
    await act(() => store.requestTransition('close'));
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('Preferences not saved. Revision conflict');
    const escape = within(alert).getByRole('button', {
      name: 'Close Without Saving',
    });
    act(() => within(alert).getByRole('button', { name: 'Retry' }).focus());
    await user.tab();
    expect(escape).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(store.getSnapshot().readyToClose).toBe(true);
    expect(persistence.config.preferences.brightness).toBe(75);
  });
});

describe('physical light UI', () => {
  it('adds a physical light with its plus button, identifies it, and keeps the virtual row separate when offline', async () => {
    let receive!: (snapshot: DevicesSnapshot) => void;
    const device = {
      deviceId: 'esp32-020000a1b2c3',
      shortId: 'IOT-A1B2C3',
      model: 'esp32-rgb',
      online: true,
      streaming: false,
      message: 'Online',
      boundLightId: null,
    };
    const identify = vi.fn(async () => {});
    const preview = vi.fn(async () => {});
    const retryDiscovery = vi.fn(async () => {});
    const { user, store } = await setup({
      available: true,
      preview,
      identify,
      retryDiscovery,
      dispose() {},
      async connect(listener) {
        receive = listener;
        receive({ devices: [device], outputError: null, discoveryError: null });
      },
    });
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    await user.click(
      screen.getByRole('button', { name: 'Add physical light' }),
    );
    const dialog = screen.getByRole('dialog');
    const draftBeforeRetry = structuredClone(store.getSnapshot().draft);
    await user.click(
      within(dialog).getByRole('button', { name: 'Retry discovery' }),
    );
    expect(retryDiscovery).toHaveBeenCalledOnce();
    expect(store.getSnapshot().draft).toEqual(draftBeforeRetry);
    await user.click(
      within(dialog).getByRole('button', { name: 'Identify IOT-A1B2C3' }),
    );
    expect(identify).toHaveBeenCalledWith(device.deviceId);
    await user.click(within(dialog).getByRole('button', { name: 'Add light' }));
    expect(
      within(
        screen.getByRole('group', { name: 'Virtual lights' }),
      ).getAllByRole('button'),
    ).toHaveLength(1);
    const row = screen.getByRole('group', { name: 'Physical lights' });
    expect(within(row).getAllByRole('button')).toHaveLength(1);
    expect(store.getSnapshot().draft!.lights[1].id).not.toBe(device.deviceId);
    expect(preview).toHaveBeenLastCalledWith({
      deviceId: device.deviceId,
      position: store.getSnapshot().draft!.lights[1].position,
      mode: 'location',
    });
    fireEvent.blur(window);
    expect(preview).toHaveBeenLastCalledWith(null);
    await user.click(screen.getByRole('button', { name: /Save Room/ }));
    act(() =>
      receive({
        devices: [{ ...device, online: false }],
        outputError: null,
        discoveryError: null,
      }),
    );
    expect(row).toHaveTextContent('Offline');
    expect(store.dirty).toBe(false);
    await user.click(
      screen.getByRole('button', { name: 'Add physical light' }),
    );
    expect(
      within(screen.getByRole('dialog')).getByRole('button', {
        name: 'Assigned',
      }),
    ).toBeDisabled();
  });
  it('binds an existing room light while retaining its logical ID', async () => {
    const device = {
      deviceId: 'esp32-020000a1b2c3',
      shortId: 'IOT-A1B2C3',
      model: 'esp32-rgb',
      online: true,
      streaming: false,
      message: 'Online',
      boundLightId: null,
    };
    const { user, store } = await setup({
      available: true,
      async preview() {},
      async identify() {},
      async retryDiscovery() {},
      dispose() {},
      async connect(receive) {
        receive({ devices: [device], outputError: null, discoveryError: null });
      },
    });
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const logicalId = store.getSnapshot().selectedLightId;
    await user.click(
      screen.getByRole('button', { name: 'Bind physical light' }),
    );
    await user.click(
      within(screen.getByRole('dialog')).getByRole('button', {
        name: 'Bind light',
      }),
    );
    expect(store.getSnapshot().draft!.lights[0]).toMatchObject({
      id: logicalId,
      output: { kind: 'esp32', deviceId: device.deviceId },
    });
    expect(
      screen.queryByRole('group', { name: 'Virtual lights' }),
    ).not.toBeInTheDocument();
  });
  it('lets Escape close the bind dialog without clearing the selection or losing focus', async () => {
    const { user, store } = await setup({
      available: true,
      async preview() {},
      async identify() {},
      async retryDiscovery() {},
      dispose() {},
      async connect(receive) {
        receive({ devices: [], outputError: null, discoveryError: null });
      },
    });
    await user.click(screen.getByRole('button', { name: 'Your Rooms' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    const selected = store.getSnapshot().selectedLightId;
    const bind = screen.getByRole('button', { name: 'Bind physical light' });
    await user.click(bind);
    const dialog = screen.getByRole('dialog');
    expect(
      within(dialog).getByRole('button', { name: 'Close physical lights' }),
    ).toHaveFocus();
    // jsdom has no close requests: send the keydown, then the dialog's cancel.
    await user.keyboard('{Escape}');
    expect(store.getSnapshot().selectedLightId).toBe(selected);
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(store.getSnapshot().selectedLightId).toBe(selected);
    expect(
      screen.getByRole('button', { name: 'Bind physical light' }),
    ).toHaveFocus();
  });
});
