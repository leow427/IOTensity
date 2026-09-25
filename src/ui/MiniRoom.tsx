import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { colorCss, resolveColor } from '../domain/colors';
import {
  validateConfiguration,
  type Configuration,
  type Position,
  type Room,
} from '../domain/model';
import { errorMessage, NativePersistence } from '../persistence/client';
import type { SyncOutput } from '../sync/output';
import { Icon } from './Icons';

function projectPosition({ x, y, z }: Position): [number, number] {
  return [198 + x * 35 - z * 16, 154 - y * 38 + z * 15];
}

export function MiniRoomScene({
  room,
  output,
}: {
  room: Room;
  output: SyncOutput;
}) {
  const elements = useRef(new Map<string, SVGCircleElement>());
  useEffect(() => {
    let frame = 0;
    const paint = () => {
      for (const light of room.lights) {
        const element = elements.current.get(light.id);
        if (element)
          element.setAttribute(
            'fill',
            colorCss(resolveColor(light, output.getColor(light.id))),
          );
      }
      frame = requestAnimationFrame(paint);
    };
    paint();
    return () => cancelAnimationFrame(frame);
  }, [room, output]);
  return (
    <svg
      viewBox="0 0 360 238"
      role="img"
      aria-label="Saved virtual room with live native light colors"
    >
      <path d="M104 30 L314 30 L314 144 L104 144 Z" fill="#374038" />
      <path d="M104 30 L28 90 L28 220 L104 144 Z" fill="#2c352e" />
      <path
        d="M104 144 L314 144 L238 220 L28 220 Z"
        fill="#424b40"
        stroke="#697362"
      />
      <path
        d="M104 144 L238 220 M174 144 L98 220 M244 144 L168 220 M78 169 L289 169 M53 195 L263 195"
        stroke="#58634f"
        strokeWidth="0.5"
      />
      <rect
        x="159"
        y="90"
        width="78"
        height="44"
        rx="3"
        fill="#181f1b"
        stroke="#78856d"
      />
      <path d="M198 134 v12 M184 146 h28" stroke="#99a78e" strokeWidth="3" />
      <text x="198" y="116" textAnchor="middle" fill="#a5b49d" fontSize="8">
        SCREEN
      </text>
      {room.lights.map((light) => {
        const [x, y] = projectPosition(light.position);
        const [, floor] = projectPosition({ ...light.position, y: 0 });
        return (
          <g key={light.id} data-light-id={light.id}>
            <title>{light.name}</title>
            <line
              x1={x}
              x2={x}
              y1={y}
              y2={floor}
              stroke="#a2b096"
              strokeWidth="1"
            />
            <ellipse
              cx={x}
              cy={floor}
              rx="4"
              ry="2"
              fill="none"
              stroke="#a2b096"
            />
            <circle
              ref={(element) => {
                if (element) elements.current.set(light.id, element);
                else elements.current.delete(light.id);
              }}
              cx={x}
              cy={y}
              r="7"
              stroke="#d2dbcd"
              strokeWidth="0.6"
            />
          </g>
        );
      })}
    </svg>
  );
}

export function MiniRoom({ output }: { output: SyncOutput }) {
  const [config, setConfig] = useState<Configuration | null>(null);
  const [error, setError] = useState<string | null>(null);
  const status = useSyncExternalStore(output.subscribe, output.getSnapshot);
  useEffect(() => {
    let gone = false;
    let unlisten: (() => void) | undefined;
    const receive = (next: Configuration) => {
      validateConfiguration(next);
      if (!gone)
        setConfig((current) =>
          current && current.revision > next.revision ? current : next,
        );
    };
    void (async () => {
      const cleanup = await listen<Configuration>(
        'configuration-saved',
        ({ payload }) => receive(payload),
      );
      if (gone) {
        cleanup();
        return;
      }
      unlisten = cleanup;
      receive(await new NativePersistence().load());
      await output.connect();
    })().catch((reason: unknown) => {
      if (!gone) setError(errorMessage(reason));
    });
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [output]);
  return (
    <main className="mini-room">
      <header data-tauri-drag-region>
        <span data-tauri-drag-region>
          IOTensity{' '}
          <small data-tauri-drag-region>
            {config?.rooms[0].name ?? 'Mini room'}
          </small>
        </span>
        <button
          aria-label="Close mini room"
          onClick={() =>
            void invoke('set_overlay', { open: false }).catch(
              (reason: unknown) => setError(errorMessage(reason)),
            )
          }
        >
          <Icon name="close" size={14} />
        </button>
      </header>
      {error ? (
        <p role="alert">{error}</p>
      ) : config ? (
        <MiniRoomScene room={config.rooms[0]} output={output} />
      ) : (
        <p>Loading saved room…</p>
      )}
      <footer>
        <span
          className={`status-dot ${status.status === 'running' ? 'active' : ''}`}
        />
        {status.status === 'running'
          ? status.source === 'display'
            ? 'LIVE · MAIN DISPLAY'
            : status.source === 'simulation'
              ? 'LIVE · SIMULATION'
              : 'LIVE · TEST IMAGE'
          : status.status.toUpperCase()}
      </footer>
    </main>
  );
}
