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

function mountSwitchableSentinel(initialEnabled: boolean, onLoadMore: () => unknown) {
  let setEnabled: ((next: boolean) => void) | null = null;
  function Harness() {
    const [enabled, updateEnabled] = React.useState(initialEnabled);
    setEnabled = updateEnabled;
    // The element stays mounted so a stale scheduled re-check can still find
    // it connected — exactly the race the latest `enabled` value must close.
    const setSentinel = useLoadMoreSentinel(enabled, onLoadMore);
    return <div ref={setSentinel} data-sentinel="true" />;
  }
  const container = document.createElement('div');
  document.body.append(container);
  const root: Root = createRoot(container);
  act(() => {
    root.render(<Harness />);
  });
  return {
    container,
    root,
    setEnabled: (next: boolean) => act(() => setEnabled?.(next)),
  };
}

function stubElementVisible(visible: boolean) {
  // jsdom lays out with zero rects; make visibility deterministic for the hook.
  Element.prototype.getBoundingClientRect = function getBoundingClientRect() {
    if (this instanceof HTMLElement && this.dataset.scrollContainer === 'clipping') {
      return {
        top: 0, bottom: 300, left: 0, right: 400, width: 400, height: 300, x: 0, y: 0, toJSON: () => ({}),
      } as DOMRect;
    }
    return {
      top: visible ? 10 : 2000,
      bottom: visible ? 20 : 2010,
      left: 0, right: 10, width: 10, height: 10, x: 0, y: 0, toJSON: () => ({}),
    } as DOMRect;
  };
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

  it('stops the auto-load chain when an overflow ancestor clips the sentinel', async () => {
    // The recent section scrolls in its own bounded overflow container; a
    // sentinel whose raw rect is inside the window but clipped by that
    // container is NOT visible, and the chain must stop there instead of
    // fetching pages the user cannot see.
    Element.prototype.getBoundingClientRect = function getBoundingClientRect() {
      if (this instanceof HTMLElement && this.dataset.scrollContainer === 'clipping') {
        return { top: 0, bottom: 300, left: 0, right: 400, width: 400, height: 300, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
      }
      if (this instanceof HTMLElement && this.dataset.sentinel === 'true') {
        // Inside the window (jsdom innerHeight 768) but below the container's
        // 300px clip — exactly the state raw-rect checks misread as visible.
        return { top: 350, bottom: 360, left: 0, right: 10, width: 10, height: 10, x: 0, y: 350, toJSON: () => ({}) } as DOMRect;
      }
      return { top: 10, bottom: 20, left: 0, right: 10, width: 10, height: 10, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    };
    const onLoadMore = vi.fn(() => Promise.resolve());
    const container = document.createElement('div');
    // Inline style: jsdom's computed style resolves it, matching how the app
    // marks its scroll containers via classes at runtime.
    container.style.overflowY = 'auto';
    container.dataset.scrollContainer = 'clipping';
    document.body.append(container);
    const root: Root = createRoot(container);
    act(() => {
      root.render(
        <div>
          {/* tall spacer pushes the sentinel past the container's 300px clip */}
          <div style={{ height: 1000 }} />
          <Harness />
        </div>,
      );
    });
    function Harness() {
      const setSentinel = useLoadMoreSentinel(true, onLoadMore);
      return <div ref={setSentinel} data-sentinel="true" />;
    }
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
        await settle();
      });
      // The intersection fired once before clipping applies; after that load
      // the clip-aware re-check must not keep the chain going.
      const callsAfterFirst = onLoadMore.mock.calls.length;
      await act(async () => {
        await settle();
        await settle();
      });
      expect(onLoadMore.mock.calls.length).toBe(callsAfterFirst);
    } finally {
      act(() => root.unmount());
      document.body.replaceChildren();
    }
  });

  it('stops a scheduled re-check that lands after the sentinel was disabled', async () => {
    stubElementVisible(true);
    let resolveLoad: (() => void) | null = null;
    const onLoadMore = vi.fn(() => new Promise<void>((resolve) => {
      resolveLoad = resolve;
    }));
    const { root, setEnabled } = mountSwitchableSentinel(true, onLoadMore);
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
      });
      expect(onLoadMore).toHaveBeenCalledTimes(1);
      // The load is still pending when the page flips to error and the section
      // disables auto-load; the element itself is still connected.
      setEnabled(false);
      await act(async () => {
        resolveLoad?.();
        await settle();
      });
      // A stale scheduled re-check must consult the latest enabled value, not
      // the one captured when the load started.
      expect(onLoadMore).toHaveBeenCalledTimes(1);
    } finally {
      act(() => root.unmount());
    }
  });
});
