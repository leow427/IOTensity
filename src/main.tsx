import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { MiniRoom } from './ui/MiniRoom';
import { NativeSyncOutput } from './sync/output';
import { createPersistence } from './persistence/client';
import { StoreProvider } from './state/context';
import { AppStore } from './state/store';
import './styles.css';

const overlay =
  new URLSearchParams(window.location.search).get('overlay') === '1';
const output = new NativeSyncOutput();
const store = overlay ? null : new AppStore(createPersistence(), output);
if (store) void store.load();
if (overlay) document.body.classList.add('overlay-body');
const root = ReactDOM.createRoot(document.getElementById('root')!);
root.render(
  <React.StrictMode>
    {store ? (
      <StoreProvider store={store}>
        <App />
      </StoreProvider>
    ) : (
      <MiniRoom output={output} />
    )}
  </React.StrictMode>,
);
const dispose = () => {
  root.unmount();
  if (store) store.dispose();
  else output.dispose();
};
if (import.meta.hot) import.meta.hot.dispose(dispose);
window.addEventListener('unload', dispose, { once: true });
