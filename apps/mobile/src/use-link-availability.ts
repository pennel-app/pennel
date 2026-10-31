import { useCallback, useMemo, useSyncExternalStore } from 'react';
import { useFocusEffect } from 'expo-router';
import type { CloudLinkAvailability } from '../modules/pennel-mobile/src/types';
import { LinkAvailabilityProbe } from './device-link';
import { useRuntime } from './runtime';

export function useLinkAvailability() {
  const { store, snapshot } = useRuntime();
  const probe = useMemo(() => new LinkAvailabilityProbe(() => store.client.invoke<CloudLinkAvailability>('cloud.linkAvailability')), [store]);
  useFocusEffect(useCallback(() => {
    probe.reset();
    void probe.refresh();
    return probe.cancel;
  }, [probe, snapshot?.cloud.account_epoch]));
  return { ...useSyncExternalStore(probe.subscribe, probe.getSnapshot, probe.getSnapshot), refresh: probe.refresh };
}
