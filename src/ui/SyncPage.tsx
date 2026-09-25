import { useState } from 'react';
import { INTENSITIES } from '../domain/model';
import { useAppState, useStore } from '../state/context';
import { RoomScene } from '../scene/RoomScene';
import { Icon, IntensityIcon } from './Icons';
import { LightCards } from './LightCards';
import { OverlayControl } from './OverlayControl';

const INTENSITY_COPY = {
  subtle: '1.8 s transitions',
  balanced: '800 ms transitions',
  vivid: '300 ms transitions',
  punch: '80 ms transitions',
};

function TransportDisplay() {
  const state = useAppState();
  return (
    <div className="transport-display">
      <div>
        <span className={`status-dot ${state.running ? 'active' : ''}`} />
        <span className="mono">{state.syncStatus.toUpperCase()}</span>
      </div>
    </div>
  );
}

export function SyncPage() {
  const store = useStore();
  const state = useAppState();
  const [resetKey, setResetKey] = useState(0);
  const room = state.saved!.rooms[0];
  return (
    <section className="page sync-page" aria-labelledby="sync-title">
      <header className="page-heading">
        <div>
          <h1 id="sync-title">Sync</h1>
        </div>
      </header>
      <div className="sync-workspace">
        <div className="sync-preview-column">
          <div className="scene-panel sync-scene">
            <div className="scene-topline">
              <div className="scene-title">
                <span className="status-dot" />
                <span>{room.name}</span>
              </div>
              <button
                className="scene-icon-button"
                aria-label="Reset preview view"
                onClick={() => setResetKey((key) => key + 1)}
              >
                <Icon name="reset" size={17} />
              </button>
            </div>
            <div className="scene-canvas">
              <RoomScene room={room} resetKey={resetKey} />
            </div>
            <div className="scene-bottomline">
              <span className="mono">
                {state.running ? 'SYNC RUNNING' : 'SAVED ROOM'}
                <i />
                {String(room.lights.length).padStart(2, '0')} LIGHTS
              </span>
              <span className="preview-corner-mark">⌖</span>
            </div>
            {!room.lights.length && (
              <div className="sync-empty">
                <h2>No lights</h2>
                <p>Add and save lights to start sync.</p>
                <button
                  className="button button-light"
                  onClick={() => void store.requestTransition('rooms')}
                >
                  Set up your room
                  <Icon name="arrow" size={17} />
                </button>
              </div>
            )}
          </div>
          <div className="sync-light-tray">
            <div className="tray-heading">
              <span className="eyebrow mono">LIGHT OUTPUT</span>
              <button
                className="text-button"
                onClick={() => void store.requestTransition('rooms')}
              >
                Edit room
                <Icon name="arrow" size={14} />
              </button>
            </div>
            {room.lights.length ? (
              <>
                {(['virtual', 'esp32'] as const).map((kind) => {
                  const lights = room.lights.filter(
                    (light) => light.output.kind === kind,
                  );
                  return lights.length ? (
                    <div className="output-group" key={kind}>
                      <h3 className="eyebrow mono">
                        {kind === 'virtual'
                          ? 'VIRTUAL PREVIEW'
                          : 'PHYSICAL LIGHTS'}
                      </h3>
                      <LightCards lights={lights} />
                    </div>
                  ) : null;
                })}
              </>
            ) : (
              <div className="empty-output">
                <span className="output-dashes">
                  <i />
                  <i />
                  <i />
                  <i />
                </span>
                <span>No saved lights</span>
              </div>
            )}
          </div>
        </div>
        <aside className="sync-controls" aria-label="Sync controls">
          <label className="field-label" htmlFor="sync-source">
            Source
          </label>
          <select
            id="sync-source"
            className="source-select"
            value={state.syncSource}
            disabled={
              state.syncBusy ||
              ['starting', 'running', 'stopping'].includes(state.syncStatus)
            }
            onChange={(event) =>
              store.setSyncSource(
                event.target.value as 'simulation' | 'test' | 'display',
              )
            }
          >
            <option value="simulation">Animated colors</option>
            <option value="test">Test image</option>
            <option value="display">Main display</option>
          </select>
          <TransportDisplay />
          <p
            className="sync-status"
            role={state.syncStatus === 'error' ? 'alert' : undefined}
            aria-live="polite"
          >
            {!store.output.available
              ? 'Sync requires the desktop app.'
              : ['starting', 'stopping', 'error'].includes(state.syncStatus)
                ? state.syncMessage
                : ''}
          </p>
          <button
            className={`start-button ${state.running ? 'is-running' : ''}`}
            disabled={
              !room.lights.length ||
              !store.output.available ||
              state.syncBusy ||
              state.syncStatus === 'stopping'
            }
            onClick={() =>
              void (state.running || state.syncStatus === 'starting'
                ? store.stop()
                : store.start())
            }
          >
            <span className="start-icon">
              <Icon name={state.running ? 'stop' : 'play'} size={21} />
            </span>
            <span>
              {state.running || state.syncStatus === 'starting'
                ? 'Stop Sync'
                : 'Start Sync'}
            </span>
            <span className="button-led" />
          </button>
          <div className="control-divider" />
          <div className="brightness-heading">
            <label htmlFor="brightness">
              <Icon name="sun" size={20} />
              Brightness
            </label>
            <span className="brightness-value">
              {state.preferences.brightness}
              <small>%</small>
            </span>
          </div>
          <input
            id="brightness"
            className="brightness-range"
            type="range"
            min="0"
            max="100"
            step="1"
            value={state.preferences.brightness}
            onChange={(event) =>
              store.setPreferences({ brightness: event.target.valueAsNumber })
            }
            style={
              {
                '--range-value': `${state.preferences.brightness}%`,
              } as React.CSSProperties
            }
          />
          <div className="range-labels mono">
            <span>LOW</span>
            <span>FULL</span>
          </div>
          <div className="control-divider" />
          {state.syncSource === 'test' && (
            <p className="sync-note">Red / green above, blue / white below.</p>
          )}
          <fieldset
            className="intensity-control"
            disabled={state.syncSource !== 'simulation'}
          >
            <legend>Intensity</legend>
            <div className="intensity-options">
              {INTENSITIES.map((intensity) => (
                <button
                  key={intensity}
                  title={`${intensity[0].toUpperCase()}${intensity.slice(1)}: ${INTENSITY_COPY[intensity]}`}
                  aria-label={`${intensity[0].toUpperCase()}${intensity.slice(1)} intensity`}
                  aria-pressed={state.preferences.intensity === intensity}
                  onClick={() => store.setPreferences({ intensity })}
                >
                  <IntensityIcon level={intensity} />
                  <span>{intensity[0].toUpperCase() + intensity.slice(1)}</span>
                </button>
              ))}
            </div>
            <p>
              {state.syncSource === 'simulation'
                ? INTENSITY_COPY[state.preferences.intensity]
                : 'Applies to animated colors only.'}
            </p>
          </fieldset>

          <OverlayControl />
        </aside>
      </div>
      <footer className="page-footer">
        <span>
          <Icon name="info" size={14} />
          Save positions to update sync. Depth does not affect screen colors.
        </span>
        <span className="mono">
          {state.preferenceSaving
            ? 'SAVING PREFERENCES…'
            : state.reducedMotion
              ? 'REDUCED MOTION'
              : ''}
        </span>
      </footer>
    </section>
  );
}
