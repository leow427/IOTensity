import { useEffect, useRef } from 'react';
import { colorCss, mixColor, resolveColor } from '../domain/colors';
import type { EditMode, VirtualLight } from '../domain/model';
import { useStore } from '../state/context';
import { Icon, LightIcon } from './Icons';
import { PhysicalStatus } from './PhysicalLights';

export function LightCards({
  lights,
  selectedId,
  mode,
  editable = false,
}: {
  lights: VirtualLight[];
  selectedId?: string | null;
  mode?: EditMode;
  editable?: boolean;
}) {
  const store = useStore();
  const elements = useRef(new Map<string, HTMLElement>());
  const row = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let frame = 0;
    const paint = () => {
      for (const light of lights) {
        const element = elements.current.get(light.id);
        if (!element) continue;
        const rgb = resolveColor(
          light,
          store.output.getColor(light.id),
          editable && light.id === selectedId ? mode : undefined,
        );
        element.style.setProperty('--light-color', colorCss(rgb));
        element.style.setProperty(
          '--light-tint',
          colorCss(mixColor([0.88, 0.88, 0.83], rgb, 0.22)),
        );
      }
      frame = requestAnimationFrame(paint);
    };
    paint();
    return () => cancelAnimationFrame(frame);
  }, [store, lights, selectedId, mode, editable]);
  useEffect(() => {
    if (selectedId)
      elements.current.get(selectedId)?.scrollIntoView?.({
        behavior: 'instant',
        block: 'nearest',
        inline: 'nearest',
      });
  }, [selectedId]);

  return (
    <div
      className={`light-cards ${editable ? '' : 'light-cards-compact'}`}
      ref={row}
      aria-label={
        lights[0]?.output.kind === 'esp32'
          ? 'Physical lights'
          : editable
            ? 'Virtual lights'
            : 'Saved virtual lights'
      }
      role="group"
    >
      {lights.map((light, index) => {
        const content = (
          <>
            <span className="card-top">
              <span className="mono">{String(index + 1).padStart(2, '0')}</span>
              <span className="card-indicator">
                {selectedId === light.id && editable ? (
                  <Icon name="check" size={13} />
                ) : (
                  <span className="light-dot" />
                )}
              </span>
            </span>
            <LightIcon kind={light.iconKind} />
            <span className="card-name">{light.name}</span>
            {light.output.kind === 'esp32' && (
              <PhysicalStatus light={light} compact />
            )}
          </>
        );
        const ref = (element: HTMLElement | null) => {
          if (element) elements.current.set(light.id, element);
          else elements.current.delete(light.id);
        };
        return editable ? (
          <button
            key={light.id}
            ref={ref}
            className="light-card"
            type="button"
            aria-pressed={selectedId === light.id}
            aria-label={`Select ${light.name}`}
            data-light-id={light.id}
            onClick={() => store.selectLight(light.id)}
          >
            {content}
          </button>
        ) : (
          <div
            key={light.id}
            ref={ref}
            className="light-card"
            data-light-id={light.id}
          >
            {content}
          </div>
        );
      })}
    </div>
  );
}
