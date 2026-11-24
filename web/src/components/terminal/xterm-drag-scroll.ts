import type { Terminal } from '@xterm/xterm';

/**
 * Drag-scroll-to-select for xterm.
 *
 * Two xterm 5.5.0 gaps are bridged here (see xterm source for evidence):
 *
 * 1. Wheel during drag-selection (browser semantics). xterm's wheel handler
 *    scrolls the viewport but never extends the drag-selection endpoints, so
 *    after scrolling the next mousemove recomputes `selectionEnd` against the
 *    NEW ydisp and the selection visually collapses. In alt-buffer / mouse
 *    reporting modes it is worse: the wheel is converted into arrow keys sent
 *    to the pty, and `onUserInput` clears the selection outright
 *    (CoreService.triggerDataEvent → SelectionService clearSelection).
 *
 * 2. Auto-scroll when the pointer is held beyond the top/bottom edge while
 *    dragging. xterm HAS a native mechanism (`SelectionService._dragScroll`
 *    ticked every 50ms from mousemove-computed `_dragScrollAmount`), but it
 *    depends on continuous mousemove events and canvas measurements; in this
 *    WKWebView host it does not engage reliably. We provide a timer-based
 *    fallback that detects the native timer's activity and stands down to
 *    avoid double scrolling.
 *
 * Selection model access: xterm exposes `Terminal['_core']._selectionService`
 * internals without a public API. This module centralizes that access in one
 * typed, defensively-guarded bridge instead of scattering hacks.
 */

/** Fallback auto-scroll cadence, matching xterm's own DRAG_SCROLL_INTERVAL. */
const EDGE_SCROLL_INTERVAL_MS = 50;
/** Rows per tick when the pointer sits exactly on the viewport edge. */
const EDGE_SCROLL_BASE_SPEED = 1;
/** Rows per tick at (and beyond) EDGE_SCROLL_MAX_OVERSHOOT_PX past the edge. */
const EDGE_SCROLL_MAX_SPEED = 8;
/** Distance past the edge (px) at which the scroll speed reaches its max. */
const EDGE_SCROLL_MAX_OVERSHOOT_PX = 120;

interface SelectionModelAccess {
  /** [x, y] buffer coords, or undefined while absent. */
  selectionStart: [number, number] | undefined;
  selectionEnd: [number, number] | undefined;
}

interface SelectionServiceAccess extends SelectionModelAccess {
  /** Re-renders the selection overlay (xterm queues it on an animation frame). */
  refresh(): void;
}

interface CoreAccess {
  _selectionService?: SelectionServiceAccess | undefined;
}

/**
 * Resolves the mutable selection endpoints of a terminal, or null when the
 * internal shape does not match (e.g. xterm upgrade, unexpected test mock).
 * Returning null degrades the feature silently — scrolling still works.
 */
function selectionAccess(term: Terminal): SelectionServiceAccess | null {
  const candidate = (term as unknown as CoreAccess)._selectionService;
  const model = candidate as unknown as {
    _model?: { selectionStart?: [number, number]; selectionEnd?: [number, number] };
    selectionStart?: [number, number];
    selectionEnd?: [number, number];
    refresh?: () => void;
  } | undefined;
  if (!candidate || typeof candidate.refresh !== 'function') return null;
  // xterm's SelectionService exposes selectionStart/End as getters over _model
  // without setters; the model fields are the writable surface.
  const writable = model?._model;
  if (!writable || typeof writable !== 'object') return null;
  return {
    get selectionStart() {
      return writable.selectionStart;
    },
    get selectionEnd() {
      return writable.selectionEnd;
    },
    set selectionEnd(value: [number, number] | undefined) {
      writable.selectionEnd = value;
    },
    refresh: () => candidate.refresh(),
  };
}

/**
 * Resolves the terminal's own drag-scroll state. When xterm's native
 * `_dragScrollAmount` is non-zero its interval timer is already scrolling and
 * extending the selection; our fallback must stand down.
 */
function nativeDragScrollAmount(term: Terminal): number {
  const candidate = (term as unknown as CoreAccess)._selectionService;
  const amount = (candidate as unknown as { _dragScrollAmount?: number } | undefined)
    ?._dragScrollAmount;
  return typeof amount === 'number' ? amount : 0;
}

/** Rows to scroll for a wheel event, honoring each DOM deltaMode. */
function wheelRows(ev: WheelEvent, rows: number): number {
  const delta = ev.deltaY;
  if (delta === 0) return 0;
  if (ev.deltaMode === WheelEvent.DOM_DELTA_LINE) return delta;
  if (ev.deltaMode === WheelEvent.DOM_DELTA_PAGE) return delta * Math.max(rows, 1);
  // DOM_DELTA_PIXEL (default for mice/touchpads in Chromium/WKWebView):
  // roughly one row per ~40px, clamped so a big flick stays manageable.
  const rowsFloat = delta / 40;
  const sign = rowsFloat < 0 ? -1 : 1;
  return sign * Math.max(1, Math.min(Math.abs(Math.round(rowsFloat)), 8));
}

/**
 * Installs wheel-during-drag selection extension and edge auto-scroll.
 *
 * @param term xterm instance (must be `open()`ed — the screen element exists).
 * @param host the container element to detect edge proximity against (the
 *   terminal host div; its bounds match the visible terminal area).
 * @returns a disposer removing every listener and timer installed here.
 */
export function attachTerminalDragScroll(
  term: Terminal,
  host: HTMLElement,
): () => void {
  const disposers: Array<() => void> = [];
  let dragging = false;
  let lastClientY = 0;
  let edgeScrollTimer: ReturnType<typeof setInterval> | null = null;

  const on = (
    target: EventTarget,
    type: string,
    listener: EventListener,
    options?: AddEventListenerOptions,
  ) => {
    target.addEventListener(type, listener, options);
    disposers.push(() => target.removeEventListener(type, listener, options));
  };

  const stopEdgeScroll = () => {
    if (edgeScrollTimer !== null) {
      clearInterval(edgeScrollTimer);
      edgeScrollTimer = null;
    }
  };

  const startEdgeScroll = () => {
    if (edgeScrollTimer !== null) return;
    edgeScrollTimer = setInterval(edgeScrollTick, EDGE_SCROLL_INTERVAL_MS);
  };

  /**
   * Extends the current selection's end toward the last pointer row. Positive
   * `rows` scrolls down (content moves up), negative scrolls up.
   */
  const extendSelectionAfterScroll = (rows: number): void => {
    const sel = selectionAccess(term);
    if (!sel || !sel.selectionStart) return;

    // Clamp the end row into buffer bounds after the viewport moved.
    const ydisp = term.buffer.active.viewportY;
    const lines = term.buffer.active.length;
    const row = Math.min(Math.max(ydisp, 0), Math.max(lines - 1, 0));

    // Keep the pointer's column at the viewport edge rows (browser semantics:
    // extending past the edge selects through the full edge line).
    const previousEnd = sel.selectionEnd;
    const col = previousEnd ? previousEnd[0] : 0;
    sel.selectionEnd = [rows > 0 ? cols(term) : rows < 0 ? 0 : col, row];
    sel.refresh();
  };

  const cols = (term: Terminal): number => term.cols;

  const edgeScrollTick = (): void => {
    if (!dragging) {
      stopEdgeScroll();
      return;
    }
    // Defer to xterm's native drag-scroll when it is active (its mousemove
    // handler already computed an out-of-viewport amount).
    if (nativeDragScrollAmount(term) !== 0) return;

    const screenEl = host.querySelector<HTMLElement>('.xterm-screen');
    if (!screenEl) return;
    const rect = screenEl.getBoundingClientRect();
    let amount = 0;
    if (lastClientY < rect.top) {
      const depth = Math.min(rect.top - lastClientY, EDGE_SCROLL_MAX_OVERSHOOT_PX);
      amount = -(EDGE_SCROLL_BASE_SPEED + Math.round((depth / EDGE_SCROLL_MAX_OVERSHOOT_PX) * (EDGE_SCROLL_MAX_SPEED - EDGE_SCROLL_BASE_SPEED)));
    } else if (lastClientY > rect.bottom) {
      const depth = Math.min(lastClientY - rect.bottom, EDGE_SCROLL_MAX_OVERSHOOT_PX);
      amount = EDGE_SCROLL_BASE_SPEED + Math.round((depth / EDGE_SCROLL_MAX_OVERSHOOT_PX) * (EDGE_SCROLL_MAX_SPEED - EDGE_SCROLL_BASE_SPEED));
    }
    if (amount === 0) return;
    term.scrollLines(amount);
    extendSelectionAfterScroll(amount);
  };

  // Track drag state. mousedown on the xterm screen (primary button) starts a
  // potential selection; any document-level mouseup ends it. This mirrors how
  // xterm's own SelectionService listens on the document for mouseup.
  on(host, 'mousedown', (event) => {
    const mouseEvent = event as MouseEvent;
    if (mouseEvent.button !== 0) return;
    // Ignore clicks that land outside the screen area (scrollbar, padding).
    const screenEl = host.querySelector<HTMLElement>('.xterm-screen');
    if (!screenEl) return;
    const rect = screenEl.getBoundingClientRect();
    if (
      mouseEvent.clientX < rect.left ||
      mouseEvent.clientX > rect.right ||
      mouseEvent.clientY < rect.top ||
      mouseEvent.clientY > rect.bottom
    ) {
      return;
    }
    dragging = true;
    lastClientY = mouseEvent.clientY;
    startEdgeScroll();
  });
  on(document, 'mouseup', () => {
    dragging = false;
    stopEdgeScroll();
  });
  on(document, 'mousemove', (event) => {
    if (!dragging) return;
    lastClientY = (event as MouseEvent).clientY;
  });

  // Wheel during drag: scroll AND extend the selection (browser semantics).
  // Returns false so xterm does not also scroll (double scroll) and does not
  // convert the wheel into arrow-key input for alt-buffer apps (which would
  // clear the selection via onUserInput).
  term.attachCustomWheelEventHandler((ev: WheelEvent) => {
    if (!dragging) return true;
    if (ev.deltaY === 0) return true;
    ev.preventDefault();
    const rows = wheelRows(ev, term.rows);
    if (rows !== 0) {
      term.scrollLines(rows);
      extendSelectionAfterScroll(rows);
    }
    return false;
  });

  return () => {
    stopEdgeScroll();
    for (const dispose of disposers.splice(0)) dispose();
    // Clear the custom wheel handler (no detach API; resetting is documented).
    term.attachCustomWheelEventHandler(() => true);
  };
}
