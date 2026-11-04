import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { AccessibilityInfo, AppState, useColorScheme } from 'react-native';
import { getLocales } from 'expo-localization';
import PennelMobile from '../modules/pennel-mobile';
import type { MobileCommand } from '../modules/pennel-mobile';
import type { RuntimeClient } from './domain';
import { LatestRequest, RuntimeStore, failure } from './state';
import { resolveLanguage, translate } from './localization';
import { makeTheme } from './theme';
import { localizedFailure } from './errors';

const client: RuntimeClient = {
  bootstrap: () => PennelMobile.bootstrap(),
  getSnapshot: () => PennelMobile.getSnapshot(),
  invoke: <T,>(method: string, payload?: Record<string, unknown>) => PennelMobile.invoke<T>(method, payload),
  subscribe: (listener) => PennelMobile.subscribe(listener),
};

const RuntimeContext = createContext<RuntimeStore | null>(null);
export function RuntimeProvider({ children, store: supplied }: { children: React.ReactNode; store?: RuntimeStore }) {
  const store = useMemo(() => supplied ?? new RuntimeStore(client), [supplied]);
  useEffect(() => {
    void store.connect();
    const listener = AppState.addEventListener('change', (state) => { if (state === 'active') void store.refresh(); });
    return () => { listener.remove(); store.disconnect(); };
  }, [store]);
  return <RuntimeContext.Provider value={store}>{children}</RuntimeContext.Provider>;
}

export function useRuntime() {
  const store = useContext(RuntimeContext);
  if (!store) throw new Error('RuntimeProvider is required.');
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const systemMode = useColorScheme();
  const language = resolveLanguage(state.snapshot?.settings.ui_language ?? 'system', getLocales().map((locale) => locale.languageTag));
  const appearance = state.snapshot?.settings.appearance ?? 'system';
  const theme = useMemo(() => makeTheme(appearance === 'system' ? systemMode === 'dark' ? 'dark' : 'light' : appearance), [appearance, systemMode]);
  const t = useCallback((key: string, values?: Record<string, string | number>) => translate(language, key, values), [language]);
  const run = useCallback(async <T,>(key: string, method: MobileCommand, payload?: Record<string, unknown>): Promise<T | undefined> => {
    try { return await store.run<T>(key, method, payload); } catch { return undefined; }
  }, [store]);
  return { ...state, store, language, theme, t, run };
}

export function useReducedMotion() {
  const [reduced, setReduced] = useState(true);
  useEffect(() => {
    let alive = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((enabled) => { if (alive) setReduced(enabled); });
    const listener = AccessibilityInfo.addEventListener('reduceMotionChanged', setReduced);
    return () => { alive = false; listener.remove(); };
  }, []);
  return reduced;
}

export function useQuery<T>(method: MobileCommand, payload: Record<string, unknown> = {}, enabled = true) {
  const { store, language } = useRuntime();
  const [value, setValue] = useState<T | null>(null);
  const [valueKey, setValueKey] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(new LatestRequest());
  const payloadKey = JSON.stringify(payload);
  const queryKey = `${method}:${payloadKey}`;
  const refresh = useCallback(async () => {
    if (!enabled) return;
    setLoading(true); setError(null);
    try {
      await latest.current.run(() => store.client.invoke<T>(method, JSON.parse(payloadKey) as Record<string, unknown>), (next) => { setValue(next); setValueKey(queryKey); setLoading(false); });
    } catch (error) { setError(localizedFailure(language, failure(error))); setLoading(false); }
  }, [enabled, language, method, payloadKey, queryKey, store]);
  useEffect(() => { setValue(null); void refresh(); return () => latest.current.cancel(); }, [refresh]);
  return { value: valueKey === queryKey ? value : null, loading, error, refresh };
}
