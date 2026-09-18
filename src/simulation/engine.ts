import { IDLE, mixColor, type RGB } from '../domain/colors';
import type { Intensity } from '../domain/model';

export interface AnimationClock {
  now(): number;
  request(callback: (time: number) => void): number;
  cancel(id: number): void;
}
export const browserClock: AnimationClock = {
  now: () => performance.now(),
  request: (callback) => requestAnimationFrame(callback),
  cancel: (id) => cancelAnimationFrame(id),
};
// Exponential response time constants in seconds; lower values react faster.
export const RESPONSE: Record<Intensity, number> = {
  subtle: 1.8,
  balanced: 0.8,
  vivid: 0.3,
  punch: 0.08,
};

export function targetColor(seconds: number, id: string): RGB {
  let hash = 0;
  for (const c of id) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
  const phase = ((hash % 1024) / 1024) * Math.PI * 2;
  return [
    0.5 + 0.48 * Math.sin(seconds * 0.65 + phase),
    0.5 + 0.48 * Math.sin(seconds * 0.65 + phase + 2.1),
    0.5 + 0.48 * Math.sin(seconds * 0.65 + phase + 4.2),
  ];
}

export class Simulation {
  private frame: number | null = null;
  private last = 0;
  private elapsed = 0;
  private ids: string[] = [];
  private colors = new Map<string, RGB>();
  private intensity: Intensity = 'balanced';
  private reducedMotion = false;
  private running = false;

  constructor(private clock: AnimationClock = browserClock) {}
  configure(ids: string[], intensity: Intensity, reducedMotion = false) {
    this.ids = ids;
    this.intensity = intensity;
    this.reducedMotion = reducedMotion;
    for (const id of this.colors.keys())
      if (!ids.includes(id)) this.colors.delete(id);
  }
  start() {
    if (this.running || !this.ids.length) return;
    this.running = true;
    this.last = this.clock.now();
    this.frame = this.clock.request(this.tick);
  }
  stop() {
    this.running = false;
    if (this.frame !== null) this.clock.cancel(this.frame);
    this.frame = null;
  }
  getColor(id: string): RGB | undefined {
    return this.colors.get(id);
  }
  get time() {
    return this.elapsed;
  }
  get isRunning() {
    return this.running;
  }
  dispose() {
    this.stop();
    this.colors.clear();
  }

  private tick = (now: number) => {
    if (!this.running) return;
    const dt = Math.max(0, Math.min((now - this.last) / 1000, 0.1));
    this.last = now;
    this.elapsed += dt;
    const speed = this.reducedMotion ? 0.15 : 1;
    const alpha =
      1 -
      Math.exp(
        -dt / Math.max(RESPONSE[this.intensity], this.reducedMotion ? 2.5 : 0),
      );
    for (const id of this.ids)
      this.colors.set(
        id,
        mixColor(
          this.colors.get(id) ?? IDLE,
          targetColor(this.elapsed * speed, id),
          alpha,
        ),
      );
    this.frame = this.clock.request(this.tick);
  };
}
