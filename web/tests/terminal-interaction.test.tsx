/** @vitest-environment jsdom */
// Terminal mouse/keyboard interaction fixes (drag-select clipboard + scroll):
//
// ROOT CAUSES (xterm 5.5.0, verified against source):
// 1. Ctrl+C / Ctrl+V were converted to ETX/SYN and sent to the pty by xterm's
//    key handler — no copy, no paste. Copy is only wired to the DOM `copy`
//    event (Cmd+C); Ctrl+V never reaches the DOM `paste` event.
// 2. Wheel during a drag-selection scrolled the viewport without extending
//    the selection endpoints; the next mousemove recomputed the end against
//    the NEW ydisp and the selection collapsed. In mouse-reporting/alt-buffer
//    mode the wheel became arrow keys (`triggerDataEvent`) → `onUserInput` →
//    selection cleared outright.
// 3. Drag past the viewport top/bottom must auto-scroll — xterm's native
//    `_dragScroll` needs continuous mousemove; a wheel-independent fallback
//    with edge detection covers the WKWebView case.
//
// These tests pin the CONTRACT at the TerminalView integration level:
// which handlers get attached, what they return, and what side effects
// (clipboard / scrollLines / selection model writes) they produce.
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act, useState } from 'react';
import { TerminalView } from '@/components/terminal/TerminalView';

Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  // jsdom has no ResizeObserver. The mock records callbacks so tests can
  // fire them (the browser fires them after observe() and on every observed
  // box change — including the collapse to 0 that a display:none flip
  // produces). Entries are not auto-fired: real behavior is under test.
  ResizeObserver: class {
    static callbacks: Array<() => void> = [];
    constructor(cb: () => void) {
      ResizeObserver.callbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});
// jsdom lacks the clipboard API entirely — stub both directions.
const clipboardWrites: string[] = [];
let clipboardReadResult: string | null = 'pasted-text';
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    writeText: vi.fn((text: string) => {
      clipboardWrites.push(text);
      return Promise.resolve();
    }),
    readText: vi.fn(() =>
      clipboardReadResult === null
        ? Promise.reject(new Error('denied'))
        : Promise.resolve(clipboardReadResult),
    ),
  },
});

const h = vi.hoisted(() => {
  // Module-level fit telemetry shared with the FitAddon mock (hoisted state
  // survives the vi.mock factory closure).
  const fitLog: string[] = [];
  class MockTerminal {
    writes: string[] = [];
    disposed = false;
    // Captured handlers — the tests drive them directly.
    customKeyHandler: ((event: KeyboardEvent) => boolean) | null = null;
    customWheelHandler: ((event: WheelEvent) => boolean) | null = null;
    pasteCalls: string[] = [];
    scrollLinesCalls: number[] = [];
    selectionModel: {
      selectionStart: [number, number] | undefined;
      selectionEnd: [number, number] | undefined;
    } = { selectionStart: undefined, selectionEnd: undefined };
    selectionText = '';
    hasSelectionFlag = false;
    selectAllCalled = 0;
    selectionChangeCb: (() => void) | null = null;
    resizeCalls: Array<{ cols: number; rows: number }> = [];

    constructor() {
      hTerms.push(this);
    }
    // Mirrors xterm's real loadAddon contract: the addon's activate hook
    // receives the terminal instance (FitAddon stores it there).
    loadAddon(addon: { loadAddon?: (term: unknown) => void } | undefined) {
      addon?.loadAddon?.(this);
    }
    // Mirrors xterm's real open(): the terminal element is created and
    // attached to the host immediately, so fit() can always read a parent.
    open(host: HTMLElement) {
      this.element = document.createElement('div');
      host.appendChild(this.element);
    }
    onData() {
      return { dispose() {} };
    }
    onResize(cb: (dims: { cols: number; rows: number }) => void) {
      this.resizeCb = cb;
      return { dispose() {} };
    }
    resize(cols: number, rows: number) {
      this.cols = cols;
      this.rows = rows;
      this.resizeCalls.push({ cols, rows });
      this.resizeCb?.({ cols, rows });
    }
    onSelectionChange(cb: () => void) {
      this.selectionChangeCb = cb;
      return { dispose() {} };
    }
    /** Simulates xterm firing the selection-change event (mouseup commit). */
    fireSelectionChange() {
      this.selectionChangeCb?.();
    }
    write(s: string) {
      this.writes.push(s);
    }
    focus() {}
    dispose() {
      this.disposed = true;
    }
    // xterm public API surface used by the interaction modules.
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
      this.customKeyHandler = handler;
    }
    attachCustomWheelEventHandler(handler: (event: WheelEvent) => boolean) {
      this.customWheelHandler = handler;
    }
    hasSelection() {
      return this.hasSelectionFlag;
    }
    getSelection() {
      return this.selectionText;
    }
    clearSelection() {}
    selectAll() {
      this.selectAllCalled += 1;
    }
    paste(text: string) {
      this.pasteCalls.push(text);
    }
    scrollLines(rows: number) {
      this.scrollLinesCalls.push(rows);
    }
    resizeCb: ((dims: { cols: number; rows: number }) => void) | null = null;
    buffer = {
      active: {
        viewportY: 0,
        length: 100,
      },
    };
    cols = 80;
    rows = 24;
    // SelectionService internals the drag-scroll bridge writes through.
    // `refresh` must be a real function (the bridge guards on it) that counts
    // invocations for assertions.
    _selectionService = {
      _model: this.selectionModel,
      refresh: () => {
        this._selectionService.refreshCalls += 1;
      },
      refreshCalls: 0,
      _dragScrollAmount: 0,
    };
    element: HTMLElement | null = null;
  }
  // Hoisted store referenced from the class body before `h` itself exists.
  const hTerms: MockTerminal[] = [];
  const listeners: Set<(event: Record<string, unknown>) => void> = new Set();
  return { terms: hTerms, listeners, MockTerminal, fitLog };
});

const MockTerminal = h.MockTerminal;

vi.mock('@xterm/xterm', () => ({ Terminal: h.MockTerminal }));
// FitAddon mock mirrors the REAL FitAddon contract that matters for the
// tab-switch regression: when the host is invisible (display:none →
// offsetParent null → computed sizes 0), proposeDimensions falls through to
// the MINIMUM geometry (2×1) instead of bailing out, and fit() resizes the
// terminal to it — exactly the corruption source being fixed. When visible,
// it reports the host's size via the element's clientWidth/Height.
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    terminal: { cols: number; rows: number; element: HTMLElement | null } | null = null;
    fitCalls = 0;
    fitCallsWhileHidden = 0;
    loadAddon(term: unknown) {
      this.terminal = term as never;
    }
    fit() {
      this.fitCalls += 1;
      const term = this.terminal;
      if (!term) {
        h.fitLog.push('no-term');
        return;
      }
      const host = term.element?.parentElement as HTMLElement | null;
      h.fitLog.push(
        `host=${!!host} closestHidden=${host ? String(host.closest('.hidden') !== null) : 'n/a'} cw=${host?.clientWidth ?? 'n/a'} ch=${host?.clientHeight ?? 'n/a'}`,
      );
      // Invisible host → real FitAddon reads 0×0 computed sizes → minimums.
      // jsdom has no layout (offsetParent is always null, clientWidth 0), so
      // the test's visibility source of truth is the panel's `hidden` class
      // — the same class the real browser CSS resolves to display:none.
      if (!host || host.closest('.hidden') !== null) {
        this.fitCallsWhileHidden += 1;
        if (term.cols !== 2 || term.rows !== 1) term.resize(2, 1);
        return;
      }
      // Visible: derive geometry from the host box (24 px rows, 8 px cols).
      const cols = Math.max(2, Math.floor(host.clientWidth / 8));
      const rows = Math.max(1, Math.floor(host.clientHeight / 24));
      if (term.cols !== cols || term.rows !== rows) term.resize(cols, rows);
    }
  },
}));

vi.mock('@/api', () => ({
  subscribeTerminalEvents: (listener: (event: unknown) => void) => {
    h.listeners.add(listener as never);
    return Promise.resolve(() => h.listeners.delete(listener as never));
  },
  writeTerminalInput: () => Promise.resolve(),
  resizeTerminal: () => Promise.resolve(),
  getTerminalHistory: () =>
    Promise.resolve({ base64: '', base: 0, end: 0, trimmed: false }),
}));

// The real context menu shell stays mounted (only the Portal's open-gating is
// stubbed) — the regression under test is the SURFACE's tree-identity
// behavior and the quote-item contract, so stubbing TerminalContextMenu
// itself would hide the bug. Radix's real Menu Content is kept because
// ContextMenuItem requires the MenuContent context it provides; only the
// Portal (which gates rendering on the menu's open state) is replaced with a
// passthrough so the items exist in the DOM for direct interaction.
vi.mock('radix-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('radix-ui')>();
  const ctxMenu = actual.ContextMenu;
  return {
    ...actual,
    ContextMenu: {
      ...ctxMenu,
      Portal: ((props: { children?: React.ReactNode }) => <>{props.children}</>) as typeof ctxMenu.Portal,
    },
  };
});

/** KeyboardEvent is not constructible with `key` in all jsdom versions. */
function keyEvent(init: {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  type?: string;
}): KeyboardEvent {
  const event = new KeyboardEvent(init.type ?? 'keydown', {
    key: init.key,
    ctrlKey: init.ctrlKey ?? false,
    metaKey: init.metaKey ?? false,
    shiftKey: init.shiftKey ?? false,
    bubbles: true,
  });
  return event;
}

function wheelEvent(deltaY: number, deltaMode: number = WheelEvent.DOM_DELTA_PIXEL): WheelEvent {
  return new WheelEvent('wheel', { deltaY, deltaMode, cancelable: true });
}

type MockTerminalInstance = InstanceType<typeof h.MockTerminal>;

async function mountTerminal(props: {
  onSelectionQuote?: (text: string) => void;
} = {}): Promise<{
  root: Root;
  host: HTMLElement;
  termHost: HTMLElement;
  term: MockTerminalInstance;
}> {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => root.render(
    <TerminalView sessionId="sess-x" onSelectionQuote={props.onSelectionQuote} />,
  ));
  const term = h.terms[0];
  if (!term) throw new Error('terminal not mounted');
  // The drag-scroll module attaches to the component's inner terminal host.
  const termHost = host.querySelector<HTMLElement>('[data-terminal-host]');
  if (!termHost) throw new Error('terminal host not rendered');
  return { root, host, termHost, term };
}

/** Drives the mock through a committed drag-selection so selectedText state
 * tracks it (the same path onSelectionChange takes in the real terminal). */
async function selectText(
  term: MockTerminalInstance,
  text: string,
): Promise<void> {
  term.hasSelectionFlag = true;
  term.selectionText = text;
  await act(async () => term.fireSelectionChange());
}

function click(el: Element): Promise<void> {
  return act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

/** Opens the terminal context menu: Radix's Portal gates the menu content on
 * the root's `open` state, so the items only exist after the trigger's
 * contextmenu event fires. jsdom + a synthetic click then drive onSelect. */
async function openContextMenu(trigger: Element): Promise<void> {
  await act(async () => {
    trigger.dispatchEvent(
      new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }),
    );
  });
}

beforeEach(() => {
  h.terms.length = 0;
  h.listeners.clear();
  h.fitLog.length = 0;
  clipboardWrites.length = 0;
  clipboardReadResult = 'pasted-text';
  document.body.replaceChildren();
  (globalThis.ResizeObserver as unknown as {
    callbacks: Array<() => void>;
  }).callbacks.length = 0;
});

describe('terminal clipboard keyboard contract', () => {
  it('attaches the custom key handler and wheel handler on mount', async () => {
    const { root, term } = await mountTerminal();
    expect(term.customKeyHandler).toBeTypeOf('function');
    expect(term.customWheelHandler).toBeTypeOf('function');
    await act(async () => root.unmount());
  });

  it('copies the selection on Ctrl+C and cancels the key (no SIGINT to pty)', async () => {
    const { root, term } = await mountTerminal();
    term.hasSelectionFlag = true;
    term.selectionText = 'selected text';
    const handled = term.customKeyHandler!(keyEvent({ key: 'c', ctrlKey: true }));
    expect(handled).toBe(false); // cancelled → xterm never sends ETX
    expect(clipboardWrites).toEqual(['selected text']);
    await act(async () => root.unmount());
  });

  it('keeps Ctrl+C as SIGINT (returns true) when nothing is selected', async () => {
    const { root, term } = await mountTerminal();
    term.hasSelectionFlag = false;
    const handled = term.customKeyHandler!(keyEvent({ key: 'c', ctrlKey: true }));
    expect(handled).toBe(true); // xterm sends ETX → pty SIGINT convention kept
    expect(clipboardWrites).toEqual([]);
    await act(async () => root.unmount());
  });

  it('pastes clipboard text through term.paste on Ctrl+V', async () => {
    const { root, term } = await mountTerminal();
    const handled = term.customKeyHandler!(keyEvent({ key: 'v', ctrlKey: true }));
    expect(handled).toBe(false); // cancelled → no SYN to the pty
    await act(async () => {}); // flush the async clipboard read + paste
    expect(term.pasteCalls).toEqual(['pasted-text']);
    await act(async () => root.unmount());
  });

  it('writes an inline hint when the clipboard read is denied on Ctrl+V', async () => {
    const { root, term } = await mountTerminal();
    clipboardReadResult = null; // denied
    term.customKeyHandler!(keyEvent({ key: 'v', ctrlKey: true }));
    await act(async () => {});
    expect(term.pasteCalls).toEqual([]);
    expect(term.writes.some((w: string) => w.includes('clipboard'))).toBe(true);
    await act(async () => root.unmount());
  });

  it('leaves Cmd+C / Cmd+V untouched on macOS (native event path)', async () => {
    const { root, term } = await mountTerminal();
    expect(term.customKeyHandler!(keyEvent({ key: 'c', metaKey: true }))).toBe(true);
    expect(term.customKeyHandler!(keyEvent({ key: 'v', metaKey: true }))).toBe(true);
    expect(clipboardWrites).toEqual([]);
    await act(async () => root.unmount());
  });
});

describe('terminal surface tree identity', () => {
  it('keeps the SAME host DOM node after the context menu attaches (no xterm unload)', async () => {
    // Regression: the surface first renders a Fragment, then flips state after
    // a setTimeout(0) to wrap the host in the Radix ContextMenu. Changing the
    // element type at the same position remounts the ENTIRE subtree — the
    // xterm-opened host div is destroyed and replaced with an empty one, so
    // the terminal renders blank and drag-scroll listeners die with the old
    // node. The host element identity must be stable across the flip.
    const { root, host } = await mountTerminal();
    const hostBefore = host.querySelector<HTMLElement>('[data-terminal-host]');
    expect(hostBefore).not.toBeNull();

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
    });

    const hostAfter = host.querySelector<HTMLElement>('[data-terminal-host]');
    expect(hostAfter).not.toBeNull();
    expect(hostAfter).toBe(hostBefore); // same node → xterm content survives
    await act(async () => root.unmount());
  });
});

describe('terminal quote-to-conversation flow', () => {
  it('floating button appears on selection and emits the captured text', async () => {
    const quoteSpy = vi.fn();
    const { root, term } = await mountTerminal({ onSelectionQuote: quoteSpy });
    // No button before a selection exists.
    expect(document.querySelector('[data-terminal-selection-quote]')).toBeNull();
    await selectText(term, 'hello world');
    const button = document.querySelector('[data-terminal-selection-quote]');
    expect(button).not.toBeNull();
    await click(button!);
    expect(quoteSpy).toHaveBeenCalledWith('hello world');
    await act(async () => root.unmount());
  });

  it('floating button falls back to the captured selection when the live selection collapsed', async () => {
    // ROOT CAUSE of the silent no-op: the click handler read
    // `getSelection() ?? selectedText` — `??` keeps an EMPTY STRING from the
    // live selection instead of falling back, so the quote never fired when
    // the click itself had collapsed the selection. `||` must win here.
    const quoteSpy = vi.fn();
    const { root, term } = await mountTerminal({ onSelectionQuote: quoteSpy });
    await selectText(term, 'captured text');
    // The click collapsed the selection before the handler ran (no
    // selection-change event fired — exactly the race being pinned).
    term.hasSelectionFlag = false;
    term.selectionText = '';
    const button = document.querySelector('[data-terminal-selection-quote]');
    expect(button).not.toBeNull();
    await click(button!);
    expect(quoteSpy).toHaveBeenCalledWith('captured text');
    await act(async () => root.unmount());
  });

  it('context menu quote item emits the captured text even if the live selection cleared', async () => {
    const quoteSpy = vi.fn();
    const { root, host, term } = await mountTerminal({ onSelectionQuote: quoteSpy });
    await selectText(term, 'menu quote');
    // Right-click opens the menu (Radix Portal mounts the content on open).
    await openContextMenu(host.querySelector('[data-terminal-host]')!);
    // Menu interaction cleared the live selection; the item stays enabled on
    // the captured state and still quotes the original text.
    term.hasSelectionFlag = false;
    term.selectionText = '';
    const item = document.querySelector('[data-terminal-menu-quote]');
    expect(item).not.toBeNull();
    await click(item!);
    expect(quoteSpy).toHaveBeenCalledWith('menu quote');
    await act(async () => root.unmount());
  });

  it('context menu quote item is disabled without a selection', async () => {
    const quoteSpy = vi.fn();
    const { root, host, term } = await mountTerminal({ onSelectionQuote: quoteSpy });
    await openContextMenu(host.querySelector('[data-terminal-host]')!);
    const item = document.querySelector('[data-terminal-menu-quote]');
    expect(item).not.toBeNull();
    expect(item!.getAttribute('aria-disabled')).toBe('true');
    await click(item!);
    expect(quoteSpy).not.toHaveBeenCalled();
    await act(async () => root.unmount());
  });

  it('quote button and menu item are absent when quoting is not wired (detached terminal)', async () => {
    const { root, term } = await mountTerminal();
    await selectText(term, 'no receiver');
    expect(document.querySelector('[data-terminal-selection-quote]')).toBeNull();
    expect(document.querySelector('[data-terminal-menu-quote]')).toBeNull();
    await act(async () => root.unmount());
  });
});

describe('terminal tab-switch geometry contract', () => {
  // The panel keeps every TerminalView mounted and toggles a wrapper's
  // `hidden` class. REAL FitAddon behavior when the host goes display:none:
  // ResizeObserver fires (content box → 0), fit() reads 0×0 computed sizes,
  // proposeDimensions floors to the MINIMUM geometry (cols 2 / rows 1), and
  // the terminal — plus the pty behind it, via onResize — is resized to 2×1.
  // Buffer reflow at 2 columns and a SIGWINCH-driven repaint by the shell
  // corrupt the layout; switching back refits to the real size but the
  // reflow round-trip is lossy (wrapped lines mis-align). The fix contract:
  // fit() must be a NO-OP while the terminal's host is not visible.

  /** Panel-accurate tab-switch harness. TerminalPanel toggles the wrapper's
   * `hidden` class AND the `active` prop in the SAME render commit
   * (TerminalPanel.tsx: `className={active ? 'h-full' : 'hidden'}`), so the
   * harness re-renders TerminalView with the matching prop — the fix's
   * activeRef guard reads exactly that prop. The wrapper class is derived
   * from the same state, keeping the two coupled like the panel. */
  function TabSwitchHarness(props: { ctrl: { flip: (hidden: boolean) => void } }) {
    const [active, setActive] = useState(true);
    props.ctrl.flip = (hidden: boolean) => setActive(!hidden);
    return (
      <div className={active ? 'h-full' : 'hidden'}>
        <TerminalView sessionId="sess-x" active={active} />
      </div>
    );
  }

  /** Flips the harness the way TerminalPanel's commit does (hidden class +
   * active prop together), then fires every registered ResizeObserver
   * callback (the browser fires them when the observed box changes —
   * including collapsing to 0 on display:none). */
  async function flipAndFire(
    ctrl: { flip: (hidden: boolean) => void },
    hidden: boolean,
  ): Promise<void> {
    await act(async () => {
      ctrl.flip(hidden);
    });
    const callbacks = (globalThis.ResizeObserver as unknown as {
      callbacks: Array<() => void>;
    }).callbacks;
    await act(async () => {
      for (const fire of callbacks) fire();
    });
  }

  it('does NOT resize the terminal (nor the pty) while its host is hidden', async () => {
    // Mount through the panel-accurate harness: hidden class + active prop
    // toggle together, exactly as TerminalPanel commits them.
    const wrapper = document.createElement('div');
    document.body.appendChild(wrapper);
    const root = createRoot(wrapper);
    const ctrl: { flip: (hidden: boolean) => void } = { flip: () => {} };
    await act(async () => root.render(<TabSwitchHarness ctrl={ctrl} />));
    const term = h.terms[0];
    const termHost = wrapper.querySelector<HTMLElement>('[data-terminal-host]')!;
    // jsdom has no layout; pin the host box the fit mock reads when visible.
    // (The mock xterm open() already attached term.element under the host.)
    Object.defineProperty(termHost, 'clientWidth', { configurable: true, value: 800 });
    Object.defineProperty(termHost, 'clientHeight', { configurable: true, value: 600 });

    // Initial visible fit: ResizeObserver fires after observe() with the
    // current box — geometry converges to the real 100×25 size. (jsdom may
    // interleave the mount-time effect fit before the sizes are pinned; that
    // artifact is pre-baselined, the assertions below watch only deltas.)
    await flipAndFire(ctrl, false);
    expect(term.cols).toBe(100);
    expect(term.rows).toBe(25);
    const baselineLength = term.resizeCalls.length;

    // Tab switch AWAY: the wrapper goes display:none and active flips to
    // false in the same commit; the browser collapses the box and fires the
    // observer. BEFORE the fix this resized the terminal (and the pty) to
    // the 2×1 minimum; the contract is a no-op.
    await flipAndFire(ctrl, true);
    expect(term.cols).toBe(100);
    expect(term.rows).toBe(25);
    expect(term.resizeCalls.length).toBe(baselineLength);

    // Tab switch BACK: active flips true (the active-effect refits via its
    // own setTimeout — pre-baselined, idempotent at the same size), the box
    // is restored and the observer fires again — geometry must resolve to
    // the same real size (no 2×1 round-trip residue).
    await flipAndFire(ctrl, false);
    expect(term.cols).toBe(100);
    expect(term.rows).toBe(25);
    // Exactly ONE real resize happened in the whole cycle: the initial fit.
    // (An idempotent fit must not re-issue resize for an unchanged size.)
    expect(term.resizeCalls.length).toBe(baselineLength);

    await act(async () => root.unmount());
    wrapper.remove();
  });

  it('still refits when the host becomes visible again after a REAL size change', async () => {
    // Hiding must not permanently freeze the geometry: if the panel itself
    // was resized while the tab was hidden (e.g. the dock was dragged), the
    // show path must fit to the NEW box, not skip forever.
    const wrapper = document.createElement('div');
    document.body.appendChild(wrapper);
    const root = createRoot(wrapper);
    const ctrl: { flip: (hidden: boolean) => void } = { flip: () => {} };
    await act(async () => root.render(<TabSwitchHarness ctrl={ctrl} />));
    const term = h.terms[0];
    const termHost = wrapper.querySelector<HTMLElement>('[data-terminal-host]')!;
    Object.defineProperty(termHost, 'clientWidth', { configurable: true, value: 800 });
    Object.defineProperty(termHost, 'clientHeight', { configurable: true, value: 600 });

    await flipAndFire(ctrl, false);
    expect(term.cols).toBe(100);
    expect(term.rows).toBe(25);
    const baselineLength = term.resizeCalls.length;

    await flipAndFire(ctrl, true);
    // The dock was resized while hidden (browser: box change on the parent;
    // jsdom: adjust the pinned size directly).
    Object.defineProperty(termHost, 'clientWidth', { configurable: true, value: 640 });
    Object.defineProperty(termHost, 'clientHeight', { configurable: true, value: 480 });

    await flipAndFire(ctrl, false);
    // The show path refits to the new box: 640/8 = 80 cols, 480/24 = 20 rows.
    expect(term.cols).toBe(80);
    expect(term.rows).toBe(20);
    // One real resize during the whole cycle: the show-path refit.
    expect(term.resizeCalls.length).toBe(baselineLength + 1);

    await act(async () => root.unmount());
    wrapper.remove();
  });
});

describe('terminal drag-scroll contract', () => {
  it('ignores the wheel when not dragging (returns true → default scroll)', async () => {
    const { root, term } = await mountTerminal();
    const handled = term.customWheelHandler!(wheelEvent(-120));
    expect(handled).toBe(true);
    expect(term.scrollLinesCalls).toEqual([]);
    await act(async () => root.unmount());
  });

  it('scrolls AND extends the selection during a drag (wheel)', async () => {
    const { root, termHost, term } = await mountTerminal();
    // Start a drag: mousedown inside the xterm screen area.
    const screen = document.createElement('div');
    screen.className = 'xterm-screen';
    termHost.appendChild(screen);
    vi.spyOn(screen, 'getBoundingClientRect').mockReturnValue(
      new DOMRect(0, 0, 800, 600),
    );
    await act(async () => {
      termHost.dispatchEvent(new MouseEvent('mousedown', { button: 0, clientY: 100, bubbles: true }));
    });
    // Real interaction order: the drag produced a selection first, then the
    // user rolls the wheel to keep selecting.
    term.selectionModel.selectionStart = [5, 20];
    // Wheel up (content moves down the buffer): scrollLines negative rows.
    const event = wheelEvent(-120);
    const handled = term.customWheelHandler!(event);
    expect(handled).toBe(false); // suppressed → xterm must not double-scroll
    expect(term.scrollLinesCalls.length).toBeGreaterThan(0);
    expect(term.scrollLinesCalls[0]).toBeLessThan(0); // upward
    // Selection end extended to the top edge of the new viewport (col 0
    // because the scroll is upward in our mocked buffer).
    expect(term.selectionModel.selectionEnd).toEqual([0, 0]);
    expect(term._selectionService.refreshCalls).toBeGreaterThan(0);
    await act(async () => root.unmount());
  });

  it('auto-scrolls toward the pointer when dragging past the bottom edge', async () => {
    vi.useFakeTimers();
    try {
      const { root, termHost, term } = await mountTerminal();
      const screen = document.createElement('div');
      screen.className = 'xterm-screen';
      termHost.appendChild(screen);
      vi.spyOn(screen, 'getBoundingClientRect').mockReturnValue(
        new DOMRect(0, 0, 800, 600),
      );
      await act(async () => {
        termHost.dispatchEvent(new MouseEvent('mousedown', { button: 0, clientY: 100, bubbles: true }));
      });
      // Pointer moves 80px BELOW the bottom edge (y=680 > 600).
      await act(async () => {
        document.dispatchEvent(new MouseEvent('mousemove', { clientY: 680, bubbles: true }));
      });
      await act(async () => {
        vi.advanceTimersByTime(120); // > 2 ticks of the 50ms interval
      });
      expect(term.scrollLinesCalls.length).toBeGreaterThan(0);
      expect(term.scrollLinesCalls[0]).toBeGreaterThan(0); // downward
      // mouseup ends the drag and stops the timer.
      await act(async () => {
        document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
      });
      const countAtStop = term.scrollLinesCalls.length;
      await act(async () => {
        vi.advanceTimersByTime(200);
      });
      expect(term.scrollLinesCalls.length).toBe(countAtStop);
      await act(async () => root.unmount());
    } finally {
      vi.useRealTimers();
    }
  });
});
