import {
  createContext,
  useContext,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import type { AppStore } from './store';

const Context = createContext<AppStore | null>(null);
export function StoreProvider({
  store,
  children,
}: {
  store: AppStore;
  children: ReactNode;
}) {
  return <Context.Provider value={store}>{children}</Context.Provider>;
}
// Shared hooks and provider deliberately live together.
// eslint-disable-next-line react-refresh/only-export-components
export function useStore() {
  const store = useContext(Context);
  if (!store) throw new Error('Missing application store.');
  return store;
}
// eslint-disable-next-line react-refresh/only-export-components
export function useAppState() {
  const store = useStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}
