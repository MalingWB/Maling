import { useCallback, useEffect, useRef } from 'react';

/**
 * Triggers `onLoadMore` when the sentinel element scrolls into view. The
 * sentinel sits at the tail of a list section, so its visibility means the
 * user reached that section's end — independent of which scroll container
 * owns it. After a load appends rows, a still-visible sentinel is re-checked
 * so a short first page keeps filling the viewport; a failed load stops the
 * automatic retry chain (the explicit "show more" button remains).
 */
export function useLoadMoreSentinel(enabled: boolean, onLoadMore: () => unknown) {
  const callbackRef = useRef(onLoadMore);
  callbackRef.current = onLoadMore;
  const elementRef = useRef<HTMLElement | null>(null);
  const observerRef = useRef<IntersectionObserver | null>(null);
  const loadingRef = useRef(false);

  const check = useCallback(() => {
    const element = elementRef.current;
    if (!element || !enabled || loadingRef.current) return;
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
  }, [enabled]);

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

function isVisible(element: HTMLElement) {
  const rect = element.getBoundingClientRect();
  return rect.bottom > 0 && rect.top < window.innerHeight;
}
