/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const applyDesktopZoom = vi.fn(async (percent: number) => percent);

vi.mock('@/api/client', () => ({
  getRuntimeApi: () => ({ applyDesktopZoom }),
}));

const installedDrivers: Array<{ adjust: (delta: number) => void; reset: () => void }> = [];
vi.mock('@/lib/desktop-page-zoom', () => ({
  setDesktopPageZoomDriver: (driver: { adjust: (delta: number) => void; reset: () => void } | null) => {
    if (driver) installedDrivers.push(driver);
  },
}));

import { useDesktopZoomShortcuts } from '@/hooks/use-desktop-zoom-shortcuts';
import { defaultPersonalizationPreference } from '@/theme';
import type { PersonalizationPreference, PreferencesVm } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

function preferencesWith(zoom: number): PreferencesVm {
  return {
    appearance: { schemaVersion: 2, themeId: 'builtin.gold-band', colorScheme: 'system', visualQualityByTheme: {} },
    personalization: { ...(defaultPersonalizationPreference as PersonalizationPreference), interfaceZoomPercent: zoom },
    language: 'zh-cn',
    useLocalClaude: false,
    verboseLogging: false,
    browser: { schemaVersion: 1, searchEngine: 'baidu', openLocalLinksInBrowser: true, openWebLinksInBrowser: true },
    avatars: {} as never,
    wallpapers: {} as never,
  };
}

function mountHarness(zoom: number, onPersistZoom: (percent: number) => void) {
  const container = document.createElement('div');
  document.body.append(container);
  const root: Root = createRoot(container);
  const render = (nextZoom: number) => act(() => {
    root.render(<Harness zoom={nextZoom} onPersistZoom={onPersistZoom} />);
  });
  function Harness({ zoom, onPersistZoom: persistZoom }: { zoom: number; onPersistZoom: (percent: number) => void }) {
    useDesktopZoomShortcuts(preferencesWith(zoom), persistZoom);
    return null;
  }
  render(zoom);
  const driver = () => installedDrivers[installedDrivers.length - 1]!;
  return { root, render, driver };
}

beforeEach(() => {
  vi.useFakeTimers();
  installedDrivers.length = 0;
  applyDesktopZoom.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
});

describe('useDesktopZoomShortcuts', () => {
  it('applies each step immediately and debounces persistence to the final value', async () => {
    const onPersistZoom = vi.fn();
    const { root, driver } = mountHarness(100, onPersistZoom);
    try {
      await act(async () => {
        driver().adjust(5);
        driver().adjust(5);
        driver().adjust(5);
      });
      expect(applyDesktopZoom).toHaveBeenCalledTimes(3);
      expect(applyDesktopZoom).toHaveBeenCalledWith(115);
      expect(onPersistZoom).not.toHaveBeenCalled();
      await act(async () => {
        vi.advanceTimersByTime(700);
      });
      // Persistence reports the single zoom percent; the owner persists it
      // through the single-field save so startup defaults can never round-trip
      // a full preference object.
      expect(onPersistZoom).toHaveBeenCalledTimes(1);
      expect(onPersistZoom).toHaveBeenCalledWith(115);
    } finally {
      act(() => root.unmount());
    }
  });

  it('cancels a pending step persist when a newer zoom is committed externally', async () => {
    const onPersistZoom = vi.fn();
    const { root, render, driver } = mountHarness(100, onPersistZoom);
    try {
      await act(async () => {
        driver().adjust(5); // schedules persist(105) in 600ms
      });
      // The settings slider commits 150% and the persisted preference syncs in.
      await act(async () => {
        render(150);
      });
      await act(async () => {
        vi.advanceTimersByTime(800);
      });
      expect(onPersistZoom).not.toHaveBeenCalled();

      // Later steps continue from the externally committed value.
      await act(async () => {
        driver().adjust(5);
        vi.advanceTimersByTime(700);
      });
      expect(onPersistZoom).toHaveBeenCalledTimes(1);
      expect(onPersistZoom).toHaveBeenCalledWith(155);
    } finally {
      act(() => root.unmount());
    }
  });

  it('re-applies an adopted zoom that a pending shortcut step previewed away from', async () => {
    const onPersist = vi.fn();
    // Startup defaults (100) are live before the preferences bootstrap lands.
    const { root, render, driver } = mountHarness(100, onPersist);
    try {
      await act(async () => {
        driver().adjust(5); // webview previews 105; persist(105) scheduled
      });
      // Bootstrap lands with the persisted 180 while that step is pending.
      await act(async () => {
        render(180);
      });
      await act(async () => {
        vi.advanceTimersByTime(800);
      });
      expect(onPersist).not.toHaveBeenCalled();
      // The adopted percent must win visually too, not only in the preference.
      expect(applyDesktopZoom).toHaveBeenCalledWith(180);

      // Later steps continue from the adopted value.
      await act(async () => {
        driver().adjust(5);
      });
      expect(applyDesktopZoom).toHaveBeenLastCalledWith(185);
    } finally {
      act(() => root.unmount());
    }
  });

  it('does not touch the webview when preferences first mount', async () => {
    const onPersist = vi.fn();
    const { root } = mountHarness(100, onPersist);
    try {
      // The backend already applied the persisted zoom before the first paint;
      // adopting startup defaults must not clobber it with a 100% preview.
      expect(applyDesktopZoom).not.toHaveBeenCalled();
    } finally {
      act(() => root.unmount());
    }
  });

  it('resets to the default immediately and persists right away', async () => {
    const onPersistZoom = vi.fn();
    const { root, driver } = mountHarness(180, onPersistZoom);
    try {
      await act(async () => {
        driver().reset();
      });
      expect(applyDesktopZoom).toHaveBeenCalledWith(100);
      expect(onPersistZoom).toHaveBeenCalledTimes(1);
      expect(onPersistZoom).toHaveBeenCalledWith(100);
    } finally {
      act(() => root.unmount());
    }
  });
});
