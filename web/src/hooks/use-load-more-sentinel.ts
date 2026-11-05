import { useCallback, useEffect, useRef } from 'react';

/**
 * Triggers `onLoadMore` when the user scrolls and the sentinel element is in
 * view. The sentinel sits at the tail of a list section, so after the first
 * real scroll its visibility means the user reached that section's end —
 * independent of which scroll container owns it. IntersectionObserver fires
 * immediately for elements already inside the viewport, so an initial page
 * that fits without scrolling must NOT trigger the next page: the hook only
 * arms after the first scroll event (captured at window level so nested
 * overflow containers count too). After a load appends rows, a still-visible
 * sentinel is re-checked so a short first page keeps filling the viewport; a
 * failed load stops the automatic retry chain (the explicit "show more"
 * button remains).
 */
export function useLoadMoreSentinel(enabled: boolean, onLoadMore: () => unknown) {
  const callbackRef = useRef(onLoadMore);
  callbackRef.current = onLoadMore;
  const elementRef = useRef<HTMLElement | null>(null);
  const observerRef = useRef<IntersectionObserver | null>(null);
  const loadingRef = useRef(false);
  // Auto-load requires a real user scroll; mounting/expand must not fill the
  // viewport by itself.
  const armedByUserScrollRef = useRef(false);
  // A scheduled re-check can fire before the disabled state commits; it must
  // read the latest value, not the one captured when its load started.
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const check = useCallback(() => {
    const element = elementRef.current;
    if (!element || !enabledRef.current || loadingRef.current) return;
    if (!armedByUserScrollRef.current) return;
    // The arm callback is a second entry point besides the observer, so the
    // visibility the intersection used to imply must hold here too: a scroll
    // anywhere arms the hook, but only a sentinel actually in view may load.
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
  // every scroll container (sidebar list, recent section, dialogs).
  useEffect(() => {
    const arm = () => {
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
