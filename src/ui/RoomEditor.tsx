import { useMemo, useState } from 'react';
import {
  BOUNDS,
  ICON_KINDS,
  MAX_LIGHTS,
  MAX_NAME_BYTES,
  truncateName,
  type Position,
} from '../domain/model';
import { useAppState, useStore } from '../state/context';
import { RoomScene } from '../scene/RoomScene';
import { Icon, LightIcon } from './Icons';
import { LightCards } from './LightCards';
import { PhysicalLightsDialog, PhysicalStatus } from './PhysicalLights';

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
  // The typed text stays local while editing so partial entries such as "0",
  // "-" or "" are not clamped mid-keystroke. In-range numbers update the
  // draft live; blur and Enter commit through the store's shared clamp.
  const [text, setText] = useState<string | null>(null);
  if (disabled && text !== null) setText(null);
  const [min, max] = BOUNDS[axis];
  const commit = () => {
    if (text === null) return;
    const parsed = text.trim() === '' ? Number.NaN : Number(text);
    if (Number.isFinite(parsed)) onChange(parsed);
    setText(null);
  };
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
            min={min}
            max={max}
            step="0.01"
            value={text ?? value}
            disabled={disabled}
            onChange={(event) => {
              const next = event.target.valueAsNumber;
              setText(event.target.value);
              if (
                event.target.value !== '' &&
                Number.isFinite(next) &&
                next >= min &&
                next <= max
              )
                onChange(next);
            }}
            onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === 'Enter') commit();
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
  const [physicalPicker, setPhysicalPicker] = useState<
    string | null | undefined
  >(undefined);
  const [resetKey, setResetKey] = useState(0);
  const room = state.draft!;
  const light = room.lights.find((item) => item.id === state.selectedLightId);
  // Stable per-kind arrays keep the card paint loops running across unrelated renders.
  const groups = useMemo(
    () =>
      (['virtual', 'esp32'] as const).map(
        (kind) =>
          [
            kind,
            room.lights.filter((item) => item.output.kind === kind),
          ] as const,
      ),
    [room.lights],
  );
  const saving = state.saveStatus === 'saving';
  const nameMissing = !!light && !light.name.trim();
  return (
    <section className="page rooms-page" aria-labelledby="rooms-title">
      <header className="page-heading">
        <div>
          <h1 id="rooms-title">Your Rooms</h1>
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
                  <span>Virtual light</span>
                </button>
                <button
                  className="button add-light-button add-physical-button"
                  disabled={saving || room.lights.length >= MAX_LIGHTS}
                  onClick={() => setPhysicalPicker(null)}
                  aria-label="Add physical light"
                >
                  <Icon name="plus" size={19} />
                  <span>Physical light</span>
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
                <p>Add a virtual or physical light.</p>
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
                LIGHTS{' '}
                <span className="count-badge">
                  {String(room.lights.length).padStart(2, '0')}
                </span>
              </span>
            </div>
            {groups.map(([kind, lights]) =>
              lights.length ? (
                <div key={kind} className="output-group">
                  <h3 className="eyebrow mono">
                    {kind === 'virtual' ? 'VIRTUAL PREVIEW' : 'PHYSICAL LIGHTS'}
                  </h3>
                  <LightCards
                    lights={lights}
                    selectedId={state.selectedLightId}
                    mode={state.editMode}
                    editable
                  />
                </div>
              ) : null,
            )}
            {!room.lights.length && (
              <button className="empty-card" onClick={() => store.addLight()}>
                <Icon name="plus" size={24} />
                <span>Add a virtual light</span>
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
              </div>
              <label className="field-label" htmlFor="light-name">
                Name
              </label>
              <input
                className="name-input"
                id="light-name"
                // UTF-16 length never exceeds UTF-8 bytes; truncateName
                // enforces the byte limit for multi-byte text.
                maxLength={MAX_NAME_BYTES}
                value={light.name}
                disabled={saving}
                aria-invalid={nameMissing || undefined}
                aria-describedby={nameMissing ? 'light-name-error' : undefined}
                onChange={(event) =>
                  store.updateLight(light.id, {
                    name: truncateName(event.target.value),
                  })
                }
              />
              {nameMissing && (
                <p className="field-error" id="light-name-error">
                  Enter a name before saving this room.
                </p>
              )}
              <div className="binding-control">
                <PhysicalStatus light={light} />
                {light.output.kind === 'esp32' ? (
                  <div className="binding-actions">
                    <button
                      className="text-button"
                      disabled={
                        saving ||
                        !!state.identifying ||
                        !state.devices.find(
                          (d) =>
                            light.output.kind === 'esp32' &&
                            d.deviceId === light.output.deviceId,
                        )?.online
                      }
                      onClick={() => {
                        if (light.output.kind === 'esp32')
                          void store.identifyDevice(light.output.deviceId);
                      }}
                    >
                      Identify
                    </button>
                    <button
                      className="text-button"
                      disabled={saving}
                      onClick={() => store.bindLight(light.id, null)}
                    >
                      Use virtual output
                    </button>
                  </div>
                ) : (
                  <button
                    className="text-button"
                    disabled={saving}
                    onClick={() => setPhysicalPicker(light.id)}
                  >
                    Bind physical light
                  </button>
                )}
              </div>
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
                    key={`${light.id}-${axis}`}
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
              </div>
            </>
          ) : (
            <div className="inspector-empty">
              <div className="empty-light-symbol">
                <LightIcon kind="bulb" size={52} />
              </div>
              <h2>Select a light</h2>
              <p>Drag its orb or use the position controls.</p>
            </div>
          )}
        </aside>
      </div>
      {physicalPicker !== undefined && (
        <PhysicalLightsDialog
          lightId={physicalPicker}
          onClose={() => setPhysicalPicker(undefined)}
        />
      )}
      {state.hardwareError && physicalPicker === undefined && (
        <p className="error-banner" role="alert">
          {state.hardwareError}
        </p>
      )}
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
