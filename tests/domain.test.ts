import { describe, expect, it } from 'vitest';
import fixture from './fixtures/configuration.json';
import {
  BOUNDS,
  clone,
  createLight,
  defaultConfiguration,
  moveLight,
  roomIsDirty,
  validateConfiguration,
  type Configuration,
} from '../src/domain/model';
import {
  calibrationColor,
  HEIGHT_COLORS,
  LOCATION_COLORS,
  resolveColor,
} from '../src/domain/colors';

describe('shared configuration and coordinates', () => {
  it('compares room values independently of native JSON property order', () => {
    const draft = defaultConfiguration().rooms[0];
    draft.lights.push(createLight(draft));
    const saved = {
      lights: draft.lights.map(({ id, name, position, iconKind, output }) => ({
        position: { z: position.z, y: position.y, x: position.x },
        iconKind,
        output,
        name,
        id,
      })),
      name: draft.name,
      id: draft.id,
    };
    expect(JSON.stringify(saved)).not.toBe(JSON.stringify(draft));
    expect(roomIsDirty(draft, saved)).toBe(false);
    saved.lights[0].iconKind = 'lamp';
    expect(roomIsDirty(draft, saved)).toBe(true);
  });
  it('accepts the same fixture used in Rust, including every appearance', () => {
    validateConfiguration(fixture);
    expect(JSON.parse(JSON.stringify(fixture))).toEqual(fixture);
  });
  it('clamps only the axes belonging to the active mode', () => {
    expect(
      moveLight({ x: 1, y: 2, z: 3 }, 'location', { x: 90, y: 0, z: -9 }),
    ).toEqual({ x: 3, y: 2, z: -0.7 });
    expect(
      moveLight({ x: 1, y: 2, z: 3 }, 'height', { x: 0, y: -5, z: 0 }),
    ).toEqual({ x: 1, y: 0.15, z: 3 });
    expect(() => moveLight({ x: 1, y: 2, z: 3 }, 'height', { y: NaN })).toThrow(
      'finite',
    );
  });
  it('creates unique stable IDs and a generic bulb with bounded coordinates', () => {
    const room = defaultConfiguration().rooms[0];
    for (let i = 0; i < 64; i++) room.lights.push(createLight(room));
    expect(new Set(room.lights.map((light) => light.id)).size).toBe(64);
    expect(room.lights.every((light) => light.iconKind === 'bulb')).toBe(true);
    validateConfiguration({ ...defaultConfiguration(), rooms: [room] });
  });
  it.each(['id', 'name', 'position', 'iconKind'] as const)(
    'rejects invalid %s at the boundary',
    (field) => {
      const config = clone(fixture) as Configuration;
      const invalid = {
        id: '../oops',
        name: '',
        position: { x: Infinity, y: 0, z: 20 },
        iconKind: 'zig',
      };
      Object.assign(config.rooms[0].lights[0], { [field]: invalid[field] });
      expect(() => validateConfiguration(config)).toThrow();
    },
  );
  it('rejects unsupported versions, unknown runtime fields, fractional preferences and duplicate IDs', () => {
    expect(() =>
      validateConfiguration({ ...fixture, schemaVersion: 99 }),
    ).toThrow('version');
    expect(() => validateConfiguration({ ...fixture, running: true })).toThrow(
      'fields',
    );
    expect(() =>
      validateConfiguration({
        ...fixture,
        preferences: { brightness: 1.5, intensity: 'balanced' },
      }),
    ).toThrow('Brightness');
    const duplicate = clone(fixture);
    duplicate.rooms[0].lights[1].id = duplicate.rooms[0].lights[0].id;
    expect(() => validateConfiguration(duplicate)).toThrow('duplicate');
  });
});

describe('calibration', () => {
  it('interpolates continuously in X and is independent of Z', () => {
    const light = createLight(defaultConfiguration().rooms[0]);
    light.position.x = BOUNDS.x[0];
    expect(calibrationColor(light, 'location')).toEqual(LOCATION_COLORS[0]);
    light.position.x = BOUNDS.x[1];
    expect(calibrationColor(light, 'location')).toEqual(LOCATION_COLORS[1]);
    light.position.x = 0;
    const middle = calibrationColor(light, 'location');
    expect(middle[0]).toBeCloseTo(0.625);
    light.position.z = 4;
    expect(calibrationColor(light, 'location')).toEqual(middle);
  });
  it('uses cyan / violet height endpoints and overrides zero brightness only for calibration', () => {
    const light = createLight(defaultConfiguration().rooms[0]);
    light.position.y = BOUNDS.y[0];
    expect(calibrationColor(light, 'height')).toEqual(HEIGHT_COLORS[0]);
    light.position.y = BOUNDS.y[1];
    expect(calibrationColor(light, 'height')).toEqual(HEIGHT_COLORS[1]);
    expect(resolveColor(light, [0, 0, 0])).toEqual([0, 0, 0]);
    expect(resolveColor(light, [188 / 255, 0, 0])).toEqual([188 / 255, 0, 0]);
    expect(resolveColor(light, [1, 0, 0], 'height')).toEqual(HEIGHT_COLORS[1]);
  });
});
