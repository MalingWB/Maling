import { useEffect, useRef } from 'react';
import { setDesktopPageZoomDriver } from '@/lib/desktop-page-zoom';
import {
  INTERFACE_ZOOM_DEFAULT,
  normalizeInterfaceZoomPercent,
  previewInterfaceZoomPercent,
  withInterfaceZoomPercent,
} from '@/lib/desktop-zoom';
import type { PersonalizationPreference, PreferencesVm } from '@/types';

/** Ctrl+wheel fires many events per second; persist at most this often. */
const ZOOM_PERSIST_DEBOUNCE_MS = 600;

/**
 * Bridges the native page-zoom gestures (intercepted by
 * `installDesktopPageZoomGuard`) to the owned interface zoom preference:
 * each step applies immediately, persistence is debounced through the
 * regular `saveDesktopPreferences` path.
 */
export function useDesktopZoomShortcuts(
  preferences: PreferencesVm,
  onPersistPersonalization: (next: PersonalizationPreference) => void,
) {
  const appliedPercentRef = useRef(preferences.personalization.interfaceZoomPercent);
  const persistTimerRef = useRef<number | null>(null);
  const latestRef = useRef({ preferences, onPersistPersonalization });
  latestRef.current = { preferences, onPersistPersonalization };

  // The applied marker follows persisted changes (settings page commit); in
  // between, it advances locally so consecutive shortcut steps accumulate.
  useEffect(() => {
    appliedPercentRef.current = preferences.personalization.interfaceZoomPercent;
  }, [preferences.personalization.interfaceZoomPercent]);

  useEffect(() => () => {
    if (persistTimerRef.current !== null) window.clearTimeout(persistTimerRef.current);
  }, []);

  useEffect(() => {
    const persist = (target: number) => {
      persistTimerRef.current = null;
      const { preferences: current, onPersistPersonalization: persistPersonalization } = latestRef.current;
      persistPersonalization(withInterfaceZoomPercent(current.personalization, target));
    };
    const schedulePersist = (target: number, immediate: boolean) => {
      if (persistTimerRef.current !== null) {
        window.clearTimeout(persistTimerRef.current);
        persistTimerRef.current = null;
      }
      if (immediate) {
        persist(target);
        return;
      }
      persistTimerRef.current = window.setTimeout(() => persist(target), ZOOM_PERSIST_DEBOUNCE_MS);
    };
    const apply = (target: number, immediate: boolean) => {
      const next = normalizeInterfaceZoomPercent(target);
      if (next === appliedPercentRef.current && !immediate) return;
      appliedPercentRef.current = next;
      void previewInterfaceZoomPercent(next);
      schedulePersist(next, immediate);
    };

    setDesktopPageZoomDriver({
      adjust: (deltaPercent) => apply(appliedPercentRef.current + deltaPercent, false),
      reset: () => apply(INTERFACE_ZOOM_DEFAULT, true),
    });
    return () => setDesktopPageZoomDriver(null);
  }, []);
}
