import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import { createPersistence } from './persistence/client';
import { StoreProvider } from './state/context';
import { AppStore } from './state/store';
import './styles.css';

const store = new AppStore(createPersistence());
void store.load();
const root = ReactDOM.createRoot(document.getElementById('root')!);
root.render(
  <React.StrictMode>
    <StoreProvider store={store}>
      <App />
    </StoreProvider>
  </React.StrictMode>,
);
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    root.unmount();
    store.dispose();
  });
window.addEventListener('unload', () => store.dispose(), { once: true });
