import { useEffect, useRef } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useAppState, useStore } from './state/context';
import { BrandMark, Icon } from './ui/Icons';
import { RoomEditor } from './ui/RoomEditor';
import { SyncPage } from './ui/SyncPage';

function UnsavedDialog() {
  const store = useStore();
  const state = useAppState();
  const dialog = useRef<HTMLDialogElement>(null);
  const stay = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.showModal();
    stay.current?.focus();
    return () => previous?.focus();
  }, []);
  const discardOnly = state.pending === 'discard';
  return (
    <dialog
      ref={dialog}
      className="unsaved-dialog"
      onCancel={(event) => {
        event.preventDefault();
        void store.resolveTransition('stay');
      }}
      aria-labelledby="dialog-title"
      aria-describedby="dialog-copy"
    >
      <h2 id="dialog-title">
        {discardOnly ? 'Discard changes?' : 'Save changes?'}
      </h2>
      <p id="dialog-copy">
        {discardOnly
          ? 'Your last saved room will be restored.'
          : 'Your room has unsaved changes.'}
      </p>
      <div className="dialog-actions">
        <button
          ref={stay}
          className="button button-light"
          disabled={state.saveStatus === 'saving'}
          onClick={() => void store.resolveTransition('stay')}
        >
          Stay
        </button>
        <button
          className="button button-outline"
          disabled={state.saveStatus === 'saving'}
          onClick={() => void store.resolveTransition('discard')}
        >
          {discardOnly ? 'Discard Changes' : 'Discard'}
        </button>
        {!discardOnly && (
          <button
            className="button button-orange"
            disabled={state.saveStatus === 'saving'}
            onClick={() => void store.resolveTransition('save')}
          >
            {state.saveStatus === 'saving' ? 'Saving…' : 'Save Room'}
          </button>
        )}
      </div>
    </dialog>
  );
}

export function App() {
  const store = useStore();
  const state = useAppState();
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        event.key.toLowerCase() === 's' &&
        store.getSnapshot().page === 'rooms'
      ) {
        event.preventDefault();
        if (!store.getSnapshot().pending) void store.saveRoom();
      }
      if (event.key === 'Escape' && !store.getSnapshot().pending)
        store.selectLight(null);
    };
    const beforeunload = (event: BeforeUnloadEvent) => {
      if (store.dirty) event.preventDefault();
    };
    const blur = () => store.clearLightPreview();
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const motion = () => store.setReducedMotion(media.matches);
    motion();
    media.addEventListener('change', motion);
    window.addEventListener('keydown', keydown);
    window.addEventListener('beforeunload', beforeunload);
    window.addEventListener('blur', blur);
    let gone = false;
    let unlisten: (() => void) | undefined;
    if (isTauri())
      getCurrentWindow()
        .onCloseRequested((event) => {
          event.preventDefault();
          void store.requestTransition('close');
        })
        .then((cleanup) => {
          if (gone) cleanup();
          else unlisten = cleanup;
        })
        .catch((error: unknown) => store.closeFailed(error));
    return () => {
      gone = true;
      unlisten?.();
      media.removeEventListener('change', motion);
      window.removeEventListener('keydown', keydown);
      window.removeEventListener('beforeunload', beforeunload);
      window.removeEventListener('blur', blur);
    };
  }, [store]);
  useEffect(() => {
    if (state.readyToClose && isTauri())
      getCurrentWindow()
        .destroy()
        .catch((error: unknown) => store.closeFailed(error));
  }, [store, state.readyToClose]);

  return (
    <div className="app-shell">
      <header className="app-titlebar" data-tauri-drag-region>
        <div className="brand">
          <BrandMark />
          <span>IOTensity</span>
        </div>
        {store.persistence.kind !== 'native' && (
          <div className="titlebar-status mono">
            Browser preview · Changes are not saved to disk
          </div>
        )}
      </header>
      <div className="app-body">
        <aside className="sidebar">
          <div>
            <nav aria-label="Main navigation">
              <button
                className={state.page === 'sync' ? 'active' : ''}
                aria-current={state.page === 'sync' ? 'page' : undefined}
                disabled={state.saveStatus === 'saving'}
                onClick={() => void store.requestTransition('sync')}
              >
                <Icon name="sync" />
                <span>Sync</span>
              </button>
              <button
                className={state.page === 'rooms' ? 'active' : ''}
                aria-current={state.page === 'rooms' ? 'page' : undefined}
                disabled={state.saveStatus === 'saving'}
                onClick={() => void store.requestTransition('rooms')}
              >
                <Icon name="room" />
                <span>Your Rooms</span>
              </button>
            </nav>
          </div>
        </aside>
        <main>
          {state.phase === 'loading' && (
            <div className="app-message">
              <BrandMark />
              <h1>Loading room…</h1>
            </div>
          )}
          {state.phase === 'error' && (
            <div className="app-message">
              <Icon name="info" size={32} />
              <h1>Couldn’t load room</h1>
              <p role="alert">{state.loadError}</p>
              <p>
                Restore a valid configuration file in the application data
                folder, then retry.
              </p>
              <button
                className="button button-dark"
                onClick={() => void store.load()}
              >
                Retry loading
              </button>
            </div>
          )}
          {state.phase === 'ready' && (
            <>
              {state.preferenceError && (
                <div className="error-banner preference-error" role="alert">
                  <span>Preferences not saved. {state.preferenceError}</span>
                  <button
                    className="text-button"
                    onClick={() => void store.savePreferences()}
                  >
                    Retry
                  </button>
                </div>
              )}
              {state.page === 'sync' ? <SyncPage /> : <RoomEditor />}
            </>
          )}
        </main>
      </div>
      {state.pending && <UnsavedDialog />}
    </div>
  );
}
