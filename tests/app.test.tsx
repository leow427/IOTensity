import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import { StoreProvider } from '../src/state/context';
import { AppStore } from '../src/state/store';
import type { Room } from '../src/domain/model';
import { FakeClock, MemoryPersistence } from './helpers';
import { Simulation } from '../src/simulation/engine';

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
async function setup() {
  const persistence = new MemoryPersistence();
  const store = new AppStore(persistence, new Simulation(new FakeClock()));
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
  it('starts empty and each addition immediately creates exactly one orb and one named bulb card', async () => {
    const { user } = await setup();
    expect(screen.getByRole('button', { name: 'Start Sync' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Your Rooms 02' }));
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
    await user.click(screen.getByRole('button', { name: 'Your Rooms 02' }));
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
  it('saves via the keyboard, marks icon edits dirty and restores the saved card on confirmed discard', async () => {
    const { user, persistence } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms 02' }));
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
  it('uses an accessible navigation dialog and displays failed saves with the draft intact', async () => {
    const { user, persistence } = await setup();
    await user.click(screen.getByRole('button', { name: 'Your Rooms 02' }));
    await user.click(screen.getByRole('button', { name: 'Add virtual light' }));
    await user.click(screen.getByRole('button', { name: 'Sync 01' }));
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
});
