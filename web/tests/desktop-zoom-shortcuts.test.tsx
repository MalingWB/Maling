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

function mountHarness(zoom: number, onPersist: (next: PersonalizationPreference) => void) {
  const container = document.createElement('div');
  document.body.append(container);
  const root: Root = createRoot(container);
  const render = (nextZoom: number) => act(() => {
    root.render(<Harness zoom={nextZoom} onPersist={onPersist} />);
  });
  function Harness({ zoom, onPersist: persist }: { zoom: number; onPersist: (next: PersonalizationPreference) => void }) {
    useDesktopZoomShortcuts(preferencesWith(zoom), persist);
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
    const onPersist = vi.fn();
    const { root, driver } = mountHarness(100, onPersist);
    try {
      await act(async () => {
        driver().adjust(5);
        driver().adjust(5);
        driver().adjust(5);
      });
      expect(applyDesktopZoom).toHaveBeenCalledTimes(3);
      expect(applyDesktopZoom).toHaveBeenCalledWith(115);
      expect(onPersist).not.toHaveBeenCalled();
      await act(async () => {
        vi.advanceTimersByTime(700);
      });
      expect(onPersist).toHaveBeenCalledTimes(1);
      expect(onPersist.mock.calls[0][0].interfaceZoomPercent).toBe(115);
    } finally {
      act(() => root.unmount());
    }
  });

  it('cancels a pending step persist when a newer zoom is committed externally', async () => {
    const onPersist = vi.fn();
    const { root, render, driver } = mountHarness(100, onPersist);
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
      expect(onPersist).not.toHaveBeenCalled();

      // Later steps continue from the externally committed value.
      await act(async () => {
        driver().adjust(5);
        vi.advanceTimersByTime(700);
      });
      expect(onPersist).toHaveBeenCalledTimes(1);
      expect(onPersist.mock.calls[0][0].interfaceZoomPercent).toBe(155);
    } finally {
      act(() => root.unmount());
    }
  });

  it('resets to the default immediately and persists right away', async () => {
    const onPersist = vi.fn();
    const { root, driver } = mountHarness(180, onPersist);
    try {
      await act(async () => {
        driver().reset();
      });
      expect(applyDesktopZoom).toHaveBeenCalledWith(100);
      expect(onPersist).toHaveBeenCalledTimes(1);
      expect(onPersist.mock.calls[0][0].interfaceZoomPercent).toBe(100);
    } finally {
      act(() => root.unmount());
    }
  });
});
