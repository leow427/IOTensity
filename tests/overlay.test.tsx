import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { MiniRoomScene } from '../src/ui/MiniRoom';
import { LightCards } from '../src/ui/LightCards';
import { StoreProvider } from '../src/state/context';
import { AppStore } from '../src/state/store';
import { clone, type Room } from '../src/domain/model';
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
});
