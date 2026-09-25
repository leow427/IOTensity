import { useEffect, useRef } from 'react';
import { shortDeviceId, type VirtualLight } from '../domain/model';
import { useAppState, useStore } from '../state/context';
import { Icon } from './Icons';

export function PhysicalStatus({ light }: { light: VirtualLight }) {
  const state = useAppState();
  if (light.output.kind === 'virtual')
    return <span className="output-type">Virtual preview</span>;
  const id = light.output.deviceId;
  const device = state.devices.find((device) => device.deviceId === id);
  return (
    <span
      className={`output-type ${device?.online ? 'online' : ''}`}
      title={id}
    >
      {shortDeviceId(id)} ·{' '}
      {device?.online ? (device.streaming ? 'Streaming' : 'Online') : 'Offline'}
    </span>
  );
}

export function PhysicalLightsDialog({
  lightId,
  onClose,
}: {
  lightId: string | null;
  onClose: () => void;
}) {
  const store = useStore();
  const state = useAppState();
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const selected = state.draft?.lights.find((light) => light.id === lightId);
  const allLights = [
    ...(state.draft?.lights ?? []),
    ...(state.saved?.rooms.slice(1).flatMap((room) => room.lights) ?? []),
  ];
  useEffect(() => {
    // Keep the original opener across StrictMode's effect setup/cleanup cycle.
    returnFocus.current ??= document.activeElement as HTMLElement | null;
    const element = dialog.current;
    element?.showModal();
    close.current?.focus();
    return () => {
      element?.close();
      returnFocus.current?.focus();
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="physical-dialog"
      aria-labelledby="physical-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <div className="physical-heading">
        <div>
          <span className="eyebrow mono">LOCAL WI-FI / ESP32</span>
          <h2 id="physical-title">
            {selected ? `Connect ${selected.name}` : 'Add a physical light'}
          </h2>
        </div>
        <button
          ref={close}
          className="icon-button"
          aria-label="Close physical lights"
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <p>
        {selected
          ? 'Choose the device that will receive this room light’s colors.'
          : 'Choose an ESP32 to create a new room light with its own physical output.'}{' '}
        Use Identify to check the LED, then save your room.
      </p>
      {!store.hardware.available ? (
        <p className="hardware-notice">
          Physical lights require the desktop app. Virtual lights remain
          available in this browser preview.
        </p>
      ) : (
        <>
          <p className="hardware-notice" role="status">
            {state.discoveryError ??
              'Discovering automatically on your local network…'}
          </p>
          <button
            className="button button-outline"
            onClick={() => void store.retryDiscovery()}
          >
            Retry discovery
          </button>
          {!state.devices.length && (
            <p>
              No ESP32 lights found yet. Connect the light to the same Wi-Fi
              network and allow IOTensity local network access.
            </p>
          )}
          <ul className="device-list">
            {state.devices.map((device) => {
              const bound = allLights.find(
                (light) =>
                  light.output.kind === 'esp32' &&
                  light.output.deviceId === device.deviceId,
              );
              return (
                <li key={device.deviceId}>
                  <div className="device-description">
                    <strong>{device.shortId}</strong>
                    <span>
                      {device.online ? device.message : 'Offline'}
                      {bound ? ` · ${bound.name}` : ''}
                    </span>
                    <code>{device.deviceId}</code>
                  </div>
                  <div className="device-actions">
                    <button
                      className="button button-outline"
                      disabled={!device.online || !!state.identifying}
                      onClick={() => void store.identifyDevice(device.deviceId)}
                      aria-label={`Identify ${device.shortId}`}
                    >
                      {state.identifying === device.deviceId
                        ? 'Identifying…'
                        : 'Identify'}
                    </button>
                    <button
                      className="button button-dark"
                      disabled={!device.online || !!bound || !store.canEdit}
                      onClick={() => {
                        if (
                          selected
                            ? store.bindLight(selected.id, device.deviceId)
                            : store.addPhysicalLight(device.deviceId)
                        )
                          onClose();
                      }}
                    >
                      {bound
                        ? 'Assigned'
                        : selected
                          ? 'Bind light'
                          : 'Add light'}
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        </>
      )}
      {state.hardwareError && (
        <p className="error-banner" role="alert">
          {state.hardwareError}
        </p>
      )}
    </dialog>
  );
}
