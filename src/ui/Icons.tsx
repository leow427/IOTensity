import type { IconKind, Intensity } from '../domain/model';

export type IconName =
  | 'sync'
  | 'room'
  | 'plus'
  | 'play'
  | 'stop'
  | 'sun'
  | 'arrow'
  | 'reset'
  | 'move'
  | 'height'
  | 'check'
  | 'close'
  | 'trash'
  | 'save'
  | 'orbit'
  | 'info';

export function Icon({
  name,
  size = 20,
  className = '',
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  const paths: Record<IconName, React.ReactNode> = {
    sync: (
      <>
        <path d="M4 9a8 8 0 0 1 13-3l3 3M20 3v6h-6M20 15A8 8 0 0 1 7 18l-3-3M4 21v-6h6" />
      </>
    ),
    room: (
      <>
        <path d="m12 3 9 5v9l-9 5-9-5V8l9-5Z M3 8l9 5 9-5M12 13v9M12 3v10" />
      </>
    ),
    plus: <path d="M12 5v14M5 12h14" />,
    play: (
      <path d="m8 5 11 7-11 7V5Z" fill="currentColor" strokeLinejoin="round" />
    ),
    stop: (
      <rect x="6" y="6" width="12" height="12" rx="1" fill="currentColor" />
    ),
    sun: (
      <>
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5" />
      </>
    ),
    arrow: <path d="M4 12h16M14 6l6 6-6 6" />,
    reset: (
      <>
        <path d="M4 10a8 8 0 1 1 0 5M4 4v6h6" />
      </>
    ),
    move: (
      <path d="M3 12h18M12 3v18M3 12l3-3M3 12l3 3M21 12l-3-3M21 12l-3 3M12 3l-3 3M12 3l3 3M12 21l-3-3M12 21l3-3" />
    ),
    height: <path d="M12 3v18M7 8l5-5 5 5M7 16l5 5 5-5M3 3h2M3 21h2" />,
    check: <path d="m5 12 4 4L19 6" />,
    close: <path d="m6 6 12 12M6 18 18 6" />,
    trash: (
      <>
        <path d="M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7" />
      </>
    ),
    save: (
      <>
        <path d="M5 3h12l4 4v14H3V3h2ZM7 3v6h9V3M7 21v-7h10v7" />
      </>
    ),
    orbit: (
      <>
        <ellipse cx="12" cy="12" rx="10" ry="5" transform="rotate(-30 12 12)" />
        <circle cx="12" cy="12" r="2" />
      </>
    ),
    info: (
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v6M12 7h.01" />
      </>
    ),
  };
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}

export function LightIcon({
  kind,
  size = 38,
}: {
  kind: IconKind;
  size?: number;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinejoin="round"
      strokeLinecap="round"
      aria-hidden="true"
      data-icon={kind}
    >
      {kind === 'bulb' && (
        <>
          <path
            d="M16 29c0-5-6-6-6-15a14 14 0 0 1 28 0c0 9-6 10-6 15v5H16v-5Z"
            transform="translate(0 3) scale(1 .85)"
          />
          <path d="M17 35h14M18 40h12M21 44h6M24 26V15m-5 2 5 5 5-5" />
        </>
      )}
      {kind === 'bar' && (
        <>
          <path
            d="m5 22 28-12c3-1 9 2 9 6v10c0 2-1 4-3 5L12 42c-3 1-7-3-7-6V22Z"
            fill="currentColor"
            fillOpacity=".12"
          />
          <path d="m5 22 7 6 30-12M12 28v14M12 33l24-10" />
        </>
      )}
      {kind === 'strip' && (
        <>
          <path
            d="M6 9h26c12 0 12 14 0 14H17c-12 0-12 15 0 15h25"
            strokeWidth="8"
          />
          <path
            d="M8 9h.1M16 9h.1M24 9h.1M33 9h.1M38 15h.1M30 23h.1M22 23h.1M13 24h.1M10 31h.1M17 38h.1M26 38h.1M35 38h.1"
            stroke="var(--icon-cutout, #dddcd3)"
            strokeWidth="2.5"
          />
        </>
      )}
      {kind === 'lamp' && (
        <>
          <path
            d="M17 6h14l9 22H8l9-22Z"
            fill="currentColor"
            fillOpacity=".18"
          />
          <path d="M24 28v14M14 43h20M9 28h30M31 29v6" />
        </>
      )}
    </svg>
  );
}

export function IntensityIcon({ level }: { level: Intensity }) {
  const count = { subtle: 1, balanced: 2, vivid: 3, punch: 4 }[level];
  return (
    <svg
      width="26"
      height="28"
      viewBox="0 0 26 28"
      fill="none"
      aria-hidden="true"
    >
      {[0, 1, 2, 3].map((i) => (
        <rect
          key={i}
          x={2 + i * 6}
          y={22 - i * 4}
          width="3"
          height={4 + i * 4}
          rx="1"
          fill="currentColor"
          opacity={i < count ? 1 : 0.18}
        />
      ))}
    </svg>
  );
}

export function BrandMark() {
  return (
    <svg
      width="27"
      height="27"
      viewBox="0 0 27 27"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M4 5v17"
        stroke="currentColor"
        strokeWidth="4"
        strokeLinecap="round"
      />
      <circle cx="18" cy="13.5" r="7" stroke="currentColor" strokeWidth="4" />
    </svg>
  );
}
