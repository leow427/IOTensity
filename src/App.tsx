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
      <span className="eyebrow mono">YOUR ROOM / UNSAVED CHANGES</span>
      <div className="dialog-symbol">
        <Icon name="room" size={30} />
      </div>
      <h2 id="dialog-title">
        {discardOnly
          ? 'Back to your saved room?'
          : 'Keep your new arrangement?'}
      </h2>
      <p id="dialog-copy">
        {discardOnly
          ? 'This will restore your last saved room, including light names, appearances, and positions.'
          : 'Your room has unsaved changes. Save them before leaving, or discard them to keep your last saved arrangement.'}
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
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const motion = () => store.setReducedMotion(media.matches);
    motion();
    media.addEventListener('change', motion);
    window.addEventListener('keydown', keydown);
    window.addEventListener('beforeunload', beforeunload);
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
          <span className="brand-divider" />
          <span className="brand-model mono">LIGHT / SPACE</span>
        </div>
        <div className="titlebar-status mono">
          <span className="status-dot" />
          {store.persistence.kind === 'native'
            ? 'LOCAL INSTRUMENT'
            : 'BROWSER PREVIEW · NOT SAVED TO DISK'}
          <span className="version">V.01</span>
        </div>
      </header>
      <div className="app-body">
        <aside className="sidebar">
          <div>
            <p className="sidebar-label mono">WORKSPACE</p>
            <nav aria-label="Main navigation">
              <button
                className={state.page === 'sync' ? 'active' : ''}
                aria-current={state.page === 'sync' ? 'page' : undefined}
                disabled={state.saveStatus === 'saving'}
                onClick={() => void store.requestTransition('sync')}
              >
                <Icon name="sync" />
                <span>Sync</span>
                <span className="nav-index mono">01</span>
              </button>
              <button
                className={state.page === 'rooms' ? 'active' : ''}
                aria-current={state.page === 'rooms' ? 'page' : undefined}
                disabled={state.saveStatus === 'saving'}
                onClick={() => void store.requestTransition('rooms')}
              >
                <Icon name="room" />
                <span>Your Rooms</span>
                <span className="nav-index mono">02</span>
              </button>
            </nav>
          </div>
          <div className="sidebar-bottom">
            <div className="sidebar-device">
              <span className="device-line" />
              <span className="device-ring" />
              <span className="device-mini-led" />
            </div>
            <span className="sidebar-edition mono">IO—01</span>
            <span className="sidebar-caption">
              A little light.
              <br />A different atmosphere.
            </span>
            <span className="sidebar-foot mono">DESIGNED FOR YOUR SPACE</span>
          </div>
        </aside>
        <main>
          {state.phase === 'loading' && (
            <div className="app-message">
              <BrandMark />
              <h1>Setting the scene.</h1>
              <p>Loading your saved room…</p>
            </div>
          )}
          {state.phase === 'error' && (
            <div className="app-message">
              <Icon name="info" size={32} />
              <h1>We couldn’t load your room.</h1>
              <p role="alert">{state.loadError}</p>
              <p>
                Your configuration hasn’t been replaced. Restore a valid file in
                the application data folder, then retry.
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
