import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { errorMessage } from '../persistence/client';

export function OverlayControl() {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);
  useEffect(() => {
    if (!isTauri()) return;
    let gone = false;
    let changed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const cleanup = await listen<boolean>(
        'overlay-changed',
        ({ payload }) => {
          changed = true;
          if (!gone) setOpen(payload);
        },
      );
      if (gone) {
        cleanup();
        return;
      }
      unlisten = cleanup;
      const current = await invoke<boolean>('overlay_is_open');
      if (!gone && !changed) setOpen(current);
    })().catch((reason: unknown) => {
      if (!gone) setError(errorMessage(reason));
    });
    return () => {
      gone = true;
      unlisten?.();
    };
  }, []);
  const toggle = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await invoke('set_overlay', { open: !open });
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  return (
    <div className="overlay-control">
      <button
        className="button button-outline"
        disabled={!isTauri() || busy}
        onClick={() => void toggle()}
      >
        {open ? 'Close mini room' : 'Open mini room'}
      </button>
      <p>Keep your virtual lights above other windows.</p>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
