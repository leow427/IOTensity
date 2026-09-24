import { BOUNDS, type EditMode, type VirtualLight } from './model';

export type RGB = readonly [number, number, number];
export const IDLE: RGB = [0.7, 0.73, 0.7];
export const LOCATION_COLORS: readonly [RGB, RGB] = [
  [0.25, 0.91, 0.53],
  [1, 0.35, 0.12],
];
export const HEIGHT_COLORS: readonly [RGB, RGB] = [
  [0.24, 0.78, 1],
  [0.78, 0.43, 1],
];

export function mixColor(a: RGB, b: RGB, t: number): RGB {
  const amount = Math.max(0, Math.min(1, t));
  return [
    a[0] + (b[0] - a[0]) * amount,
    a[1] + (b[1] - a[1]) * amount,
    a[2] + (b[2] - a[2]) * amount,
  ];
}
export function calibrationColor(light: VirtualLight, mode: EditMode): RGB {
  const axis = mode === 'location' ? 'x' : 'y';
  const [min, max] = BOUNDS[axis];
  const endpoints = mode === 'location' ? LOCATION_COLORS : HEIGHT_COLORS;
  return mixColor(
    endpoints[0],
    endpoints[1],
    (light.position[axis] - min) / (max - min),
  );
}
export const scaleColor = (color: RGB, brightness: number): RGB =>
  color.map((c) => (c * brightness) / 100) as unknown as RGB;
export const colorCss = (color: RGB): string =>
  `rgb(${color.map((c) => Math.round(c * 255)).join(' ')})`;

export function resolveColor(
  light: VirtualLight,
  output: RGB | undefined,
  mode?: EditMode,
): RGB {
  // Calibration is editing feedback and stays readable even at zero output brightness.
  return mode ? calibrationColor(light, mode) : (output ?? IDLE);
}
