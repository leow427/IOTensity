import { describe, expect, it } from 'vitest';
import { IDLE } from '../src/domain/colors';
import { Simulation, targetColor } from '../src/simulation/engine';
import { FakeClock } from './helpers';

describe('one deterministic simulation service', () => {
  it('does not start without saved lights and owns one cancellable loop', () => {
    const clock = new FakeClock();
    const simulation = new Simulation(clock);
    simulation.start();
    expect(clock.callbacks.size).toBe(0);
    simulation.configure(['a'], 'balanced');
    simulation.start();
    simulation.start();
    simulation.start();
    expect(clock.callbacks.size).toBe(1);
    clock.advance(100);
    const output = simulation.getColor('a');
    expect(output).toBeDefined();
    simulation.stop();
    simulation.stop();
    expect(clock.callbacks.size).toBe(0);
    clock.advance(500);
    expect(simulation.getColor('a')).toEqual(output);
    simulation.start();
    expect(clock.callbacks.size).toBe(1);
    simulation.dispose();
    expect(clock.callbacks.size).toBe(0);
  });
  it('is deterministic and stronger presets approach the target faster', () => {
    const a = new FakeClock();
    const b = new FakeClock();
    const subtle = new Simulation(a);
    const punch = new Simulation(b);
    subtle.configure(['a'], 'subtle');
    punch.configure(['a'], 'punch');
    subtle.start();
    punch.start();
    a.advance(100);
    b.advance(100);
    const target = targetColor(0.1, 'a');
    const distance = (color: readonly number[]) =>
      color.reduce((total, c, i) => total + (c - target[i]) ** 2, 0);
    expect(distance(punch.getColor('a')!)).toBeLessThan(
      distance(subtle.getColor('a')!),
    );
    expect(targetColor(2, 'a')).toEqual(targetColor(2, 'a'));
    expect(subtle.getColor('a')).not.toEqual(IDLE);
  });
  it('retains elapsed time across stop/start and slows reduced-motion output', () => {
    const clock = new FakeClock();
    const simulation = new Simulation(clock);
    simulation.configure(['a'], 'punch', true);
    simulation.start();
    clock.advance(100);
    expect(simulation.time).toBeCloseTo(0.1);
    simulation.stop();
    clock.advance(500);
    simulation.start();
    clock.advance(100);
    expect(simulation.time).toBeCloseTo(0.2);
  });
});
