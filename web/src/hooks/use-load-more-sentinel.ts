import { useCallback, useEffect, useRef } from 'react';

/**
 * Triggers `onLoadMore` when the user scrolls the sentinel's own scroll
 * container and the sentinel element is in view. The sentinel sits at the
 * tail of a list section, so after a real scroll of its container its
 * visibility means the user reached that section's end. IntersectionObserver
 * fires immediately for elements already inside the viewport, so an initial
 * page that fits without scrolling must NOT trigger the next page: the hook
 * only arms on scroll events whose target lies inside the sentinel's nearest
 * scrollable ancestor (window-level scrolls count only for sentinels without
 * one). Scrolls of unrelated panes — e.g. chat streaming auto-scroll — never
 * page a list they do not belong to, and a list that fits without a
 * scrollbar relies on the explicit "show more" button. After a load appends
 * rows, a still-visible sentinel is re-checked so a short first page keeps
 * filling the viewport; a failed load stops the automatic retry chain.
 */
export function useLoadMoreSentinel(enabled: boolean, onLoadMore: () => unknown) {
  const callbackRef = useRef(onLoadMore);
  callbackRef.current = onLoadMore;
  const elementRef = useRef<HTMLElement | null>(null);
  const observerRef = useRef<IntersectionObserver | null>(null);
  const loadingRef = useRef(false);
  // Auto-load requires a real scroll of the owning container; mounting or
  // unrelated panes must not fill the viewport by themselves.
  const armedByUserScrollRef = useRef(false);
  // Owning scroll container for the current sentinel; undefined = not yet
  // resolved, null = no scrollable ancestor (window-level fallback).
  const scrollContainerRef = useRef<HTMLElement | null | undefined>(undefined);
  // A scheduled re-check can fire before the disabled state commits; it must
  // read the latest value, not the one captured when its load started.
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const check = useCallback(() => {
    const element = elementRef.current;
    if (!element || !enabledRef.current || loadingRef.current) return;
    if (!armedByUserScrollRef.current) return;
    // The arm callback is a second entry point besides the observer, so the
    // visibility the intersection used to imply must hold here too.
    if (!isVisible(element)) return;
    loadingRef.current = true;
    let failed = false;
    try {
      const result = callbackRef.current();
      if (result instanceof Promise) {
        result
          .catch(() => {
            failed = true;
          })
          .finally(() => {
            loadingRef.current = false;
            // Re-check once so lists shorter than the viewport keep loading;
            // a rejected load ends the automatic chain.
            if (!failed && element.isConnected && isVisible(element)) {
              window.setTimeout(() => check(), 0);
            }
          });
        return;
      }
    } catch {
      failed = true;
    }
    loadingRef.current = false;
    if (!failed && element.isConnected && isVisible(element)) {
      window.setTimeout(() => check(), 0);
    }
  }, []);

  // Scroll events do not bubble, but they do traverse ancestors in the
  // capture phase, so a window-level capture listener observes scrolls from
  // every scroll container; ownership is decided per event target. Identity
  // checks against the `window`/`document` globals are unreliable across
  // realms (test runners), so page-level targets are recognized structurally:
  // they are not element nodes.
  useEffect(() => {
    const arm = (event: Event) => {
      const element = elementRef.current;
      if (!element) return;
      if (scrollContainerRef.current === undefined) {
        scrollContainerRef.current = findScrollContainer(element);
      }
      const container = scrollContainerRef.current;
      const target = event.target as EventTarget | null;
      const targetIsNode = typeof (target as Node | null)?.nodeType === 'number';
      const ownsScroll = container
        ? target === container || (targetIsNode && container.contains(target as Node))
        : !targetIsNode || (target as Node).nodeType !== 1;
      if (!ownsScroll) return;
      armedByUserScrollRef.current = true;
      check();
    };
    window.addEventListener('scroll', arm, { capture: true, passive: true });
    return () => {
      window.removeEventListener('scroll', arm, { capture: true });
    };
  }, [check]);

  const setSentinelElement = useCallback((element: HTMLElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    elementRef.current = element;
    scrollContainerRef.current = undefined;
    if (!element || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) check();
    });
    observer.observe(element);
    observerRef.current = observer;
  }, [check]);

  useEffect(() => () => {
    observerRef.current?.disconnect();
    observerRef.current = null;
  }, []);

  return setSentinelElement;
}

/** Nearest ancestor that actually scrolls (overflow auto/scroll); null when
 * the sentinel only moves with the page itself. */
function findScrollContainer(element: HTMLElement): HTMLElement | null {
  let node: HTMLElement | null = element.parentElement;
  while (node && node !== document.body && node !== document.documentElement) {
    const { overflowY, overflow } = getComputedStyle(node);
    if (overflowY === 'auto' || overflowY === 'scroll' || overflow === 'auto' || overflow === 'scroll') {
      return node;
    }
    node = node.parentElement;
  }
  return null;
}

/**
 * Window-visibility with ancestor clipping: sections like the recent list
 * scroll inside their own bounded overflow containers, and a sentinel whose
 * raw rect is inside the window but clipped by such a container is not
 * visible — the auto-load chain must stop there instead of fetching pages
 * the user cannot see.
 */
function isVisible(element: HTMLElement) {
  let rect = element.getBoundingClientRect();
  let node: HTMLElement | null = element.parentElement;
  while (node && node !== document.body && node !== document.documentElement) {
    const { overflowY, overflow } = getComputedStyle(node);
    if (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'hidden' || overflow === 'hidden') {
      const clip = node.getBoundingClientRect();
      const top = Math.max(rect.top, clip.top);
      const bottom = Math.min(rect.bottom, clip.bottom);
      const left = Math.max(rect.left, clip.left);
      const right = Math.min(rect.right, clip.right);
      if (bottom <= top || right <= left) return false;
      rect = { top, bottom, left, right } as DOMRect;
    }
    node = node.parentElement;
  }
  return rect.bottom > 0 && rect.top < window.innerHeight;
}
