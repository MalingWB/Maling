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
      left: 0, right: 10, width: 10, height: 10, x: 0, y: 0,
    } as DOMRect;
  };
}

/** The first real user scroll arms the sentinel. */
function armByUserScroll(target: EventTarget = window) {
  act(() => {
    target.dispatchEvent(new Event('scroll'));
  });
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
  it('does not auto-load before the user scrolls even when the sentinel is already intersecting', async () => {
    // Expanding a section renders the first page with the sentinel inside the
    // viewport without any user scroll; IntersectionObserver fires immediately
    // for such elements. The list must stay at one page until a real scroll.
    const onLoadMore = vi.fn(() => Promise.resolve());
    const { root } = mountSentinel(true, onLoadMore);
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
        await settle();
        await settle();
      });
      expect(onLoadMore).not.toHaveBeenCalled();
    } finally {
      act(() => root.unmount());
    }
  });

  it('arms on the first scroll and then loads an already-visible sentinel', async () => {
    stubElementVisible(true);
    const onLoadMore = vi.fn(() => Promise.resolve());
    const { root } = mountSentinel(true, onLoadMore);
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
        await settle();
      });
      expect(onLoadMore).not.toHaveBeenCalled();
      armByUserScroll();
      await act(async () => {
        await settle();
      });
      expect(onLoadMore.mock.calls.length).toBeGreaterThanOrEqual(1);
    } finally {
      act(() => root.unmount());
    }
  });

  it('ignores scrolls from outside its own scroll container', async () => {
    // The sidebar list must not page because the chat pane scrolled (e.g.
    // streaming auto-scroll): only a scroll whose target lies inside the
    // sentinel's own scroll container arms the hook. A list that fits without
    // a scrollbar therefore relies on the explicit button until its own
    // container actually scrolls.
    stubElementVisible(true);
    const onLoadMore = vi.fn(() => new Promise<void>(() => {}));
    const own = document.createElement('div');
    own.style.overflowY = 'auto';
    document.body.append(own);
    const container = document.createElement('div');
    own.append(container);
    const root: Root = createRoot(container);
    function Harness() {
      const setSentinel = useLoadMoreSentinel(true, onLoadMore);
      return <div ref={setSentinel} data-sentinel="true" />;
    }
    act(() => {
      root.render(<Harness />);
    });
    const unrelated = document.createElement('div');
    unrelated.style.overflowY = 'auto';
    document.body.append(unrelated);
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      armByUserScroll(unrelated);
      armByUserScroll(window);
      await act(async () => {
        await settle();
      });
      expect(onLoadMore).not.toHaveBeenCalled();
      armByUserScroll(own);
      await act(async () => {
        await settle();
      });
      expect(onLoadMore.mock.calls.length).toBeGreaterThanOrEqual(1);
    } finally {
      act(() => root.unmount());
      document.body.replaceChildren();
    }
  });

  it('arms from a scroll inside a nested overflow container', async () => {
    // scroll events do not bubble, but reach window capture listeners; the
    // recent list scrolls in its own container and must still arm the hook.
    stubElementVisible(true);
    const onLoadMore = vi.fn(() => Promise.resolve());
    const scroller = document.createElement('div');
    scroller.style.overflowY = 'auto';
    document.body.append(scroller);
    const container = document.createElement('div');
    scroller.append(container);
    const root: Root = createRoot(container);
    function Harness() {
      const setSentinel = useLoadMoreSentinel(true, onLoadMore);
      return <div ref={setSentinel} data-sentinel="true" />;
    }
    act(() => {
      root.render(<Harness />);
    });
    const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
    try {
      await act(async () => {
        StubIntersectionObserver.callback?.([{ isIntersecting: true }]);
        await settle();
      });
      expect(onLoadMore).not.toHaveBeenCalled();
      armByUserScroll(scroller);
      await act(async () => {
        await settle();
      });
      expect(onLoadMore.mock.calls.length).toBeGreaterThanOrEqual(1);
    } finally {
      act(() => root.unmount());
      document.body.replaceChildren();
    }
  });

  it('loads the next page when the sentinel intersects after the user scrolled', async () => {
    // Visible sentinel: the arm-triggered check itself performs the load; the
    // load stays pending forever so the subsequent intersection cannot
    // double-fire — exactly one request for one armed arrival.
    stubElementVisible(true);
    const onLoadMore = vi.fn(() => new Promise<void>(() => {}));
    const { root } = mountSentinel(true, onLoadMore);
    try {
      expect(StubIntersectionObserver.callback).toBeTruthy();
      armByUserScroll();
      expect(onLoadMore).toHaveBeenCalledTimes(1);
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
      armByUserScroll();
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
      armByUserScroll();
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
      armByUserScroll();
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
      armByUserScroll();
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
      // the one captured when its load started.
      expect(onLoadMore).toHaveBeenCalledTimes(1);
    } finally {
      act(() => root.unmount());
    }
  });
});
