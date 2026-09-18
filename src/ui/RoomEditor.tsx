import { useState } from 'react';
import { BOUNDS, ICON_KINDS, MAX_LIGHTS, type Position } from '../domain/model';
import { useAppState, useStore } from '../state/context';
import { RoomScene } from '../scene/RoomScene';
import { Icon, LightIcon } from './Icons';
import { LightCards } from './LightCards';

const APPEARANCE = {
  bulb: 'Bulb',
  bar: 'Light Bar',
  strip: 'LED Strip',
  lamp: 'Lamp',
};
const AXIS_LABELS = { x: 'Left / right', y: 'Height', z: 'Front / back' };

function CoordinateControl({
  axis,
  value,
  onChange,
  disabled,
}: {
  axis: keyof Position;
  value: number;
  onChange: (value: number) => void;
  disabled: boolean;
}) {
  return (
    <div className="coordinate-control">
      <div className="coordinate-heading">
        <label htmlFor={`axis-${axis}`}>
          {AXIS_LABELS[axis]}{' '}
          <span className="axis-badge mono">{axis.toUpperCase()}</span>
        </label>
        <div className="coordinate-number">
          <input
            aria-label={`${AXIS_LABELS[axis]} coordinate`}
            type="number"
            min={BOUNDS[axis][0]}
            max={BOUNDS[axis][1]}
            step="0.01"
            value={value}
            disabled={disabled}
            onChange={(event) => {
              if (
                event.target.value !== '' &&
                Number.isFinite(event.target.valueAsNumber)
              )
                onChange(event.target.valueAsNumber);
            }}
          />
          <span>m</span>
        </div>
      </div>
      <input
        id={`axis-${axis}`}
        className={`position-range range-${axis}`}
        type="range"
        min={BOUNDS[axis][0]}
        max={BOUNDS[axis][1]}
        step="0.01"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.valueAsNumber)}
      />
      <div className="range-labels mono">
        <span>
          {axis === 'x' ? 'LEFT' : axis === 'y' ? 'FLOOR' : 'MONITOR'}
        </span>
        <span>
          {axis === 'x' ? 'RIGHT' : axis === 'y' ? 'CEILING' : 'INTO ROOM'}
        </span>
      </div>
    </div>
  );
}

export function RoomEditor() {
  const store = useStore();
  const state = useAppState();
  const [resetKey, setResetKey] = useState(0);
  const room = state.draft!;
  const light = room.lights.find((item) => item.id === state.selectedLightId);
  const saving = state.saveStatus === 'saving';
  return (
    <section className="page rooms-page" aria-labelledby="rooms-title">
      <header className="page-heading">
        <div>
          <p className="eyebrow mono">SPACE / 02</p>
          <h1 id="rooms-title">
            Your Rooms<span className="title-dot">.</span>
          </h1>
          <p className="page-description">Give every light a place.</p>
        </div>
        <div className="save-actions">
          <span
            className={`save-state mono ${store.dirty ? 'is-dirty' : ''}`}
            role="status"
          >
            {saving
              ? 'SAVING…'
              : store.dirty
                ? '● UNSAVED CHANGES'
                : state.saveStatus === 'saved'
                  ? store.persistence.kind === 'native'
                    ? '✓ SAVED TO DISK'
                    : '✓ PREVIEW UPDATED'
                  : 'ALL CHANGES SAVED'}
          </span>
          <button
            className="button button-dark save-button"
            disabled={!store.dirty || saving}
            onClick={() => void store.saveRoom()}
          >
            <Icon name="save" size={16} />
            {saving ? 'Saving…' : 'Save Room'}
            <kbd>⌘ S</kbd>
          </button>
        </div>
      </header>
      {state.saveError && (
        <div className="error-banner" role="alert">
          <Icon name="info" />
          <span>{state.saveError}</span>
          <button className="text-button" onClick={() => void store.saveRoom()}>
            Retry save
          </button>
        </div>
      )}
      <div className="room-workspace">
        <div className="room-stage-column">
          <div className="scene-panel room-scene">
            <div className="scene-topline">
              <div className="scene-title">
                <span className="status-dot" />
                <span>{room.name}</span>
                <span className="scene-caption mono">ROOM 01</span>
              </div>
              <div className="scene-actions">
                <button
                  className="scene-icon-button"
                  aria-label="Reset room view"
                  title="Reset view"
                  onClick={() => setResetKey((key) => key + 1)}
                >
                  <Icon name="reset" size={17} />
                </button>
                <button
                  className="button add-light-button"
                  disabled={saving || room.lights.length >= MAX_LIGHTS}
                  onClick={() => store.addLight()}
                  aria-label="Add virtual light"
                >
                  <Icon name="plus" size={19} />
                  <span>Add light</span>
                </button>
              </div>
            </div>
            <div className="scene-canvas">
              <RoomScene
                room={room}
                selectedId={state.selectedLightId}
                mode={state.editMode}
                editable
                resetKey={resetKey}
              />
            </div>
            {!room.lights.length && (
              <div className="stage-empty-note">
                <span className="small-cross">+</span>
                <p>
                  Your space is ready.
                  <br />
                  <strong>Add your first virtual light.</strong>
                </p>
              </div>
            )}
            <div className="scene-bottomline">
              <span>
                <Icon name="orbit" size={15} />
                Drag to orbit <i />
                Scroll to zoom
              </span>
              <span className="mono">
                {String(room.lights.length).padStart(2, '0')} LIGHTS
              </span>
            </div>
          </div>
          <div className="light-tray">
            <div className="tray-heading">
              <span className="eyebrow mono">
                YOUR LIGHTS{' '}
                <span className="count-badge">
                  {String(room.lights.length).padStart(2, '0')}
                </span>
              </span>
              <span className="tray-hint">Select a light to position it</span>
            </div>
            {room.lights.length ? (
              <LightCards
                lights={room.lights}
                selectedId={state.selectedLightId}
                mode={state.editMode}
                editable
              />
            ) : (
              <button className="empty-card" onClick={() => store.addLight()}>
                <Icon name="plus" size={24} />
                <span>Add a virtual light</span>
                <small>Start with a little glow.</small>
              </button>
            )}
          </div>
        </div>
        <aside className="light-inspector" aria-label="Selected light editor">
          {light ? (
            <>
              <div className="inspector-title">
                <span className="eyebrow mono">LIGHT SETTINGS</span>
                <button
                  className="icon-button"
                  aria-label="Clear light selection"
                  onClick={() => store.selectLight(null)}
                >
                  <Icon name="close" size={16} />
                </button>
              </div>
              <div className="inspector-light-icon">
                <LightIcon kind={light.iconKind} size={43} />
                <span className="mono">
                  VIRTUAL /{' '}
                  {String(
                    room.lights.findIndex((item) => item.id === light.id) + 1,
                  ).padStart(2, '0')}
                </span>
              </div>
              <label className="field-label" htmlFor="light-name">
                Name
              </label>
              <input
                className="name-input"
                id="light-name"
                maxLength={64}
                value={light.name}
                disabled={saving}
                onChange={(event) =>
                  store.updateLight(light.id, { name: event.target.value })
                }
              />
              <fieldset className="appearance-picker" disabled={saving}>
                <legend className="field-label">Appearance</legend>
                <div>
                  {ICON_KINDS.map((kind) => (
                    <button
                      key={kind}
                      title={APPEARANCE[kind]}
                      aria-label={APPEARANCE[kind]}
                      aria-pressed={light.iconKind === kind}
                      onClick={() =>
                        store.updateLight(light.id, { iconKind: kind })
                      }
                    >
                      <LightIcon kind={kind} size={23} />
                      <span>{APPEARANCE[kind]}</span>
                    </button>
                  ))}
                </div>
              </fieldset>
              <div className="inspector-divider" />
              <div
                className="edit-tabs"
                role="tablist"
                aria-label="Position mode"
              >
                <button
                  role="tab"
                  id="location-tab"
                  aria-selected={state.editMode === 'location'}
                  aria-controls="position-panel"
                  tabIndex={state.editMode === 'location' ? 0 : -1}
                  onKeyDown={(event) => {
                    if (
                      event.key === 'ArrowRight' ||
                      event.key === 'ArrowLeft'
                    ) {
                      store.setEditMode('height');
                      document.getElementById('height-tab')?.focus();
                    }
                  }}
                  onClick={() => store.setEditMode('location')}
                >
                  <Icon name="move" size={15} />
                  Location
                </button>
                <button
                  role="tab"
                  id="height-tab"
                  aria-selected={state.editMode === 'height'}
                  aria-controls="position-panel"
                  tabIndex={state.editMode === 'height' ? 0 : -1}
                  onKeyDown={(event) => {
                    if (
                      event.key === 'ArrowRight' ||
                      event.key === 'ArrowLeft'
                    ) {
                      store.setEditMode('location');
                      document.getElementById('location-tab')?.focus();
                    }
                  }}
                  onClick={() => store.setEditMode('height')}
                >
                  <Icon name="height" size={15} />
                  Height
                </button>
              </div>
              <div
                id="position-panel"
                role="tabpanel"
                aria-labelledby={`${state.editMode}-tab`}
                className="position-panel"
              >
                {(state.editMode === 'location'
                  ? (['x', 'z'] as const)
                  : (['y'] as const)
                ).map((axis) => (
                  <CoordinateControl
                    key={axis}
                    axis={axis}
                    value={light.position[axis]}
                    disabled={saving}
                    onChange={(value) =>
                      store.moveLight(light.id, { [axis]: value })
                    }
                  />
                ))}
                <p className="calibration-note">
                  <span className={`calibration-dot ${state.editMode}`} />
                  <span>
                    {state.editMode === 'location'
                      ? 'Green to orange shows left to right.'
                      : 'Cyan to violet shows floor to ceiling.'}
                    <br />
                    This color is a placement guide.
                  </span>
                </p>
              </div>
              <div className="inspector-footer">
                <button
                  className="text-button delete-button"
                  disabled={saving}
                  onClick={() => store.deleteSelected()}
                >
                  <Icon name="trash" size={15} />
                  Delete light
                </button>
                <span className="mono">
                  {light.position.x.toFixed(2)} / {light.position.y.toFixed(2)}{' '}
                  / {light.position.z.toFixed(2)}
                </span>
              </div>
            </>
          ) : (
            <div className="inspector-empty">
              <span className="eyebrow mono">LIGHT SETTINGS</span>
              <div className="empty-light-symbol">
                <LightIcon kind="bulb" size={52} />
              </div>
              <h2>
                A place for
                <br />
                every light.
              </h2>
              <p>
                Add a virtual light, then select its orb or card to fine-tune
                the position.
              </p>
              <div className="empty-instruction">
                <Icon name="move" />
                <span>Move around the room</span>
              </div>
              <div className="empty-instruction">
                <Icon name="height" />
                <span>Set the right height</span>
              </div>
              <div className="inspector-empty-foot mono">
                YOUR ROOM. YOUR ARRANGEMENT.
              </div>
            </div>
          )}
        </aside>
      </div>
      <footer className="page-footer">
        <span>
          <Icon name="info" size={14} />
          Sync uses your last saved room.
        </span>
        <button
          className="text-button"
          disabled={!store.dirty || saving}
          onClick={() => void store.requestTransition('discard')}
        >
          Discard Changes
        </button>
      </footer>
    </section>
  );
}
