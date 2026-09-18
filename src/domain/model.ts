export const SCHEMA_VERSION = 1 as const;
export const ICON_KINDS = ['bulb', 'bar', 'strip', 'lamp'] as const;
export type IconKind = (typeof ICON_KINDS)[number];
export const INTENSITIES = ['subtle', 'balanced', 'vivid', 'punch'] as const;
export type Intensity = (typeof INTENSITIES)[number];
export type EditMode = 'location' | 'height';
export type Position = { x: number; y: number; z: number };
export type VirtualLight = {
  id: string;
  name: string;
  position: Position;
  iconKind: IconKind;
};
export type Room = { id: string; name: string; lights: VirtualLight[] };
export type Preferences = { brightness: number; intensity: Intensity };
export type Configuration = {
  schemaVersion: typeof SCHEMA_VERSION;
  revision: number;
  rooms: Room[];
  preferences: Preferences;
};

// One scene unit is one metre; origin = floor under the monitor centre.
export const BOUNDS = { x: [-3, 3], y: [0.15, 3], z: [-0.7, 4] } as const;
export const MAX_LIGHTS = 64;
export const clone = <T>(value: T): T => structuredClone(value);

export function defaultConfiguration(): Configuration {
  return {
    schemaVersion: SCHEMA_VERSION,
    revision: 0,
    rooms: [{ id: 'studio', name: 'Studio', lights: [] }],
    preferences: { brightness: 75, intensity: 'balanced' },
  };
}

export function clampAxis(axis: keyof Position, value: number): number {
  const [min, max] = BOUNDS[axis];
  if (!Number.isFinite(value))
    throw new Error('Position must be a finite number.');
  return Math.round(Math.max(min, Math.min(max, value)) * 100) / 100;
}

export function moveLight(
  position: Position,
  mode: EditMode,
  next: Partial<Position>,
): Position {
  const result = { ...position };
  const axes: (keyof Position)[] = mode === 'location' ? ['x', 'z'] : ['y'];
  axes.forEach((axis) => {
    if (next[axis] !== undefined) result[axis] = clampAxis(axis, next[axis]);
  });
  return result;
}

export function createLight(
  room: Room,
  makeId = () => crypto.randomUUID(),
): VirtualLight {
  let number = room.lights.length + 1;
  while (room.lights.some((light) => light.name === `Light ${number}`))
    number++;
  const index = room.lights.length;
  return {
    id: makeId(),
    name: `Light ${number}`,
    iconKind: 'bulb',
    position: {
      x: Math.round((-1.7 + (index % 5) * 0.85) * 100) / 100,
      y: 1.2,
      z: 1 + ((Math.floor(index / 5) * 0.2) % 3),
    },
  };
}

export function roomIsDirty(draft: Room, saved: Room): boolean {
  // JSON object key order is not part of the contract (Rust and JS differ).
  return (
    draft.id !== saved.id ||
    draft.name !== saved.name ||
    draft.lights.length !== saved.lights.length ||
    draft.lights.some((light, index) => {
      const other = saved.lights[index];
      return (
        light.id !== other.id ||
        light.name !== other.name ||
        light.iconKind !== other.iconKind ||
        light.position.x !== other.position.x ||
        light.position.y !== other.position.y ||
        light.position.z !== other.position.z
      );
    })
  );
}

export function validateConfiguration(
  value: unknown,
): asserts value is Configuration {
  const fail = (message: string): never => {
    throw new Error(message);
  };
  const object = (
    v: unknown,
    fields: string[],
    label: string,
  ): Record<string, unknown> => {
    if (!v || typeof v !== 'object' || Array.isArray(v))
      return fail(`Invalid ${label}.`);
    const obj = v as Record<string, unknown>;
    if (
      Object.keys(obj).some((key) => !fields.includes(key)) ||
      fields.some((key) => !(key in obj))
    )
      return fail(`Invalid ${label} fields.`);
    return obj;
  };
  const name = (v: unknown) =>
    typeof v === 'string' &&
    v.trim().length > 0 &&
    new TextEncoder().encode(v).length <= 64 &&
    !Array.from(v).some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127);
  const id = (v: unknown) =>
    typeof v === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(v);
  const config = object(
    value,
    ['schemaVersion', 'revision', 'rooms', 'preferences'],
    'configuration',
  );
  if (config.schemaVersion !== SCHEMA_VERSION)
    fail('Unsupported configuration version. Your file has not been changed.');
  if (!Number.isSafeInteger(config.revision) || (config.revision as number) < 0)
    fail('Invalid revision.');
  if (
    !Array.isArray(config.rooms) ||
    config.rooms.length < 1 ||
    config.rooms.length > 16
  )
    fail('Expected 1–16 rooms.');
  const roomIds = new Set<string>();
  const lightIds = new Set<string>();
  for (const rawRoom of config.rooms as unknown[]) {
    const room = object(rawRoom, ['id', 'name', 'lights'], 'room');
    if (!id(room.id) || roomIds.has(room.id as string))
      fail('Invalid or duplicate room ID.');
    roomIds.add(room.id as string);
    if (!name(room.name)) fail('Room names must contain 1–64 bytes of text.');
    if (!Array.isArray(room.lights) || room.lights.length > MAX_LIGHTS)
      fail('A room supports at most 64 virtual lights.');
    for (const rawLight of room.lights as unknown[]) {
      const light = object(
        rawLight,
        ['id', 'name', 'position', 'iconKind'],
        'light',
      );
      if (!id(light.id) || lightIds.has(light.id as string))
        fail('Invalid or duplicate light ID.');
      lightIds.add(light.id as string);
      if (!name(light.name))
        fail('Light names must contain 1–64 bytes of text.');
      if (!ICON_KINDS.includes(light.iconKind as IconKind))
        fail('Unknown light appearance.');
      const position = object(light.position, ['x', 'y', 'z'], 'position');
      for (const axis of ['x', 'y', 'z'] as const) {
        const n = position[axis];
        if (
          typeof n !== 'number' ||
          !Number.isFinite(n) ||
          n < BOUNDS[axis][0] ||
          n > BOUNDS[axis][1]
        )
          fail(`Position ${axis} is outside the room.`);
      }
    }
  }
  const prefs = object(
    config.preferences,
    ['brightness', 'intensity'],
    'preferences',
  );
  if (
    !Number.isInteger(prefs.brightness) ||
    (prefs.brightness as number) < 0 ||
    (prefs.brightness as number) > 100
  )
    fail('Brightness must be a whole number from 0 to 100.');
  if (!INTENSITIES.includes(prefs.intensity as Intensity))
    fail('Unknown intensity.');
}
