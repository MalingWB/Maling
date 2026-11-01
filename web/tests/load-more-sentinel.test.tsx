/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useLoadMoreSentinel } from '@/hooks/use-load-more-sentinel';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

type IntersectionCallback = (entries: Array<{ isIntersecting: boolean }>) => void;

class StubIntersectionObserver {
  static instances: StubIntersectionObserver[] = [];
  static callback: IntersectionCallback | null = null;
  observed: HTMLElement | null = null;
  constructor(callback: IntersectionCallback) {
    StubIntersectionObserver.callback = callback;
    StubIntersectionObserver.instances.push(this);
  }
  observe(element: HTMLElement) {
    this.observed = element;
  }
  disconnect() {
    this.observed = null;
  }
  unobserve() {
    this.observed = null;
  }
}

function mountSentinel(enabled: boolean, onLoadMore: () => unknown) {
  function Harness() {
    const setSentinel = useLoadMoreSentinel(enabled, onLoadMore);
    return enabled ? <div ref={setSentinel} data-sentinel="true" /> : null;
  }
  const container = document.createElement('div');
  document.body.append(container);
  const root: Root = createRoot(container);
  act(() => {
    root.render(<Harness />);
  });
  return { container, root };
}

function stubElementVisible(visible: boolean) {
  // jsdom lays out with zero rects; make visibility deterministic for the hook.
  Element.prototype.getBoundingClientRect = () => ({
    top: visible ? 10 : 2000,
    bottom: visible ? 20 : 2010,
    left: 0, right: 10, width: 10, height: 10, x: 0, y: 0, toJSON: () => ({}),
  } as DOMRect);
}

beforeEach(() => {
  vi.stubGlobal('IntersectionObserver', StubIntersectionObserver);
  StubIntersectionObserver.instances = [];
  StubIntersectionObserver.callback = null;
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe('useLoadMoreSentinel', () => {
  it('loads the next page when the sentinel intersects the viewport', async () => {
    const onLoadMore = vi.fn();
    const { root } = mountSentinel(true, onLoadMore);
    try {
      expect(StubIntersectionObserver.callback).toBeTruthy();
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
      });
      expect(onLoadMore).toHaveBeenCalledTimes(1);
    } finally {
      act(() => root.unmount());
    }
  });

  it('does not fire while disabled', async () => {
    const onLoadMore = vi.fn();
    const { root } = mountSentinel(false, onLoadMore);
    try {
      // Disabled renders no sentinel element at all.
      expect(document.querySelector('[data-sentinel="true"]')).toBeNull();
      expect(onLoadMore).not.toHaveBeenCalled();
    } finally {
      act(() => root.unmount());
    }
  });

  it('keeps loading while the sentinel stays visible and stops on failure', async () => {
    stubElementVisible(true);
    let calls = 0;
    const onLoadMore = () => {
      calls += 1;
      if (calls >= 2) return Promise.reject(new Error('load failed'));
      return Promise.resolve();
    };
    const { root } = mountSentinel(true, onLoadMore);
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
        await settle();
      });
      // First success re-checks (still visible) and loads again; the second,
      // failing load ends the automatic chain.
      await act(async () => {
        await settle();
      });
      expect(calls).toBe(2);
    } finally {
      act(() => root.unmount());
    }
  });
});
