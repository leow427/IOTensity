import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import { MiniRoom, MiniRoomScene } from '../src/ui/MiniRoom';
import { LightCards } from '../src/ui/LightCards';
import { StoreProvider } from '../src/state/context';
import { AppStore } from '../src/state/store';
import { clone, type Configuration, type Room } from '../src/domain/model';
import fixture from './fixtures/configuration.json';
import { FakeOutput, MemoryPersistence } from './helpers';

it('the overlay and existing cards use the same final color without brightness multiplication', () => {
  const output = new FakeOutput();
  output.getColor = () => [188 / 255, 64 / 255, 0];
  const room = clone(fixture.rooms[0]) as Room;
  const original = clone(room);
  const store = new AppStore(new MemoryPersistence(), output);
  const { container, unmount } = render(
    <StoreProvider store={store}>
      <MiniRoomScene room={room} output={output} />
      <LightCards lights={room.lights} />
    </StoreProvider>,
  );
  expect(screen.getByRole('img')).toHaveAccessibleName(
    /live native light colors/,
  );
  expect(container.querySelector('circle')).toHaveAttribute(
    'fill',
    'rgb(188 64 0)',
  );
  expect(container.querySelector('.light-card')).toHaveStyle(
    '--light-color: rgb(188 64 0)',
  );
  expect(room).toEqual(original);
  expect(screen.queryByRole('button')).not.toBeInTheDocument();
  unmount();
  store.dispose();
});

describe('read-only mini room geometry', () => {
  it('uses saved light identity and moves the orb when a new saved room arrives', () => {
    const output = new FakeOutput();
    const room = clone(fixture.rooms[0]) as Room;
    const { container, rerender, unmount } = render(
      <MiniRoomScene room={room} output={output} />,
    );
    const before = container.querySelector('circle')!.getAttribute('cy');
    const next = clone(room);
    next.lights[0].position.y = 3;
    rerender(<MiniRoomScene room={next} output={output} />);
    expect(container.querySelector('g')).toHaveAttribute(
      'data-light-id',
      room.lights[0].id,
    );
    expect(
      Number(container.querySelector('circle')!.getAttribute('cy')),
    ).toBeLessThan(Number(before));
    expect(room.lights[0].position.y).toBe(1.2);
    unmount();
    output.dispose();
  });
  it('repaints an orb only when its final color changes', () => {
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((paint) => {
      frames.push(paint);
      return frames.length;
    });
    const output = new FakeOutput();
    output.getColor = () => [0, 0, 1];
    const room = clone(fixture.rooms[0]) as Room;
    const { container, unmount } = render(
      <MiniRoomScene room={room} output={output} />,
    );
    const circle = container.querySelector('circle')!;
    expect(circle).toHaveAttribute('fill', 'rgb(0 0 255)');
    const setAttribute = vi.spyOn(circle, 'setAttribute');
    frames.at(-1)!(16);
    expect(setAttribute).not.toHaveBeenCalled();
    output.getColor = () => [1, 0, 0];
    frames.at(-1)!(32);
    expect(circle).toHaveAttribute('fill', 'rgb(255 0 0)');
    unmount();
    output.dispose();
  });
});

describe('mini room saved-configuration listener', () => {
  // Unmount (and unlisten) before the mocked event plugin is cleared.
  afterEach(() => {
    cleanup();
    clearMocks();
  });
  const saved = (revision: number, name: string) => {
    const config = clone(fixture) as Configuration;
    config.revision = revision;
    config.rooms[0].name = name;
    return config;
  };
  const mount = async (load: () => unknown) => {
    mockIPC(
      (command) => {
        if (command === 'load_config') return load();
        throw new Error(`Unexpected command ${command}`);
      },
      { shouldMockEvents: true },
    );
    const output = new FakeOutput();
    const view = render(<MiniRoom output={output} />);
    await act(async () => {});
    return { ...view, output };
  };
  const publish = (config: unknown) =>
    act(() => emit('configuration-saved', config));

  it('clears a load failure when a newer valid configuration is saved', async () => {
    const { unmount, output } = await mount(() => {
      throw new Error('Configuration is unreadable.');
    });
    expect(screen.getByRole('alert')).toHaveTextContent(
      'Configuration is unreadable.',
    );
    await publish(saved(1, 'Recovered'));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('Recovered')).toBeInTheDocument();
    expect(screen.getByRole('img')).toHaveAccessibleName(
      /live native light colors/,
    );
    unmount();
    output.dispose();
  });

  it('ignores an invalid event payload and keeps the last valid room', async () => {
    const { container, unmount, output } = await mount(() => saved(2, 'Kept'));
    expect(screen.getByText('Kept')).toBeInTheDocument();
    await expect(
      publish({ ...saved(3, 'Broken'), schemaVersion: 4 }),
    ).resolves.toBeUndefined();
    await publish({ revision: 4 });
    expect(screen.getByText('Kept')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(container.querySelectorAll('circle')).toHaveLength(
      fixture.rooms[0].lights.length,
    );
    unmount();
    output.dispose();
  });

  it('ignores an older revision after a newer saved room arrives', async () => {
    const { unmount, output } = await mount(() => saved(5, 'Loaded'));
    await publish(saved(6, 'Newest'));
    await publish(saved(4, 'Stale'));
    expect(screen.getByText('Newest')).toBeInTheDocument();
    expect(screen.queryByText('Stale')).not.toBeInTheDocument();
    unmount();
    output.dispose();
  });
});
