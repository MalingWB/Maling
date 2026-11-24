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
import { act } from 'react';
import { TerminalView } from '@/components/terminal/TerminalView';

Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  ResizeObserver: class {
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

    constructor() {
      hTerms.push(this);
    }
    loadAddon() {}
    open() {}
    onData() {
      return { dispose() {} };
    }
    onResize() {
      return { dispose() {} };
    }
    onSelectionChange() {
      return { dispose() {} };
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
  return { terms: hTerms, listeners, MockTerminal };
});

const MockTerminal = h.MockTerminal;

vi.mock('@xterm/xterm', () => ({ Terminal: h.MockTerminal }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { loadAddon() {} fit() {} } }));

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

// The real context menu shell stays mounted (only the MenuContent portal is
// stubbed) — the regression under test is the SURFACE's tree-identity
// behavior, so stubbing TerminalContextMenu itself would hide the bug.
// Stub the radix module's Content (portal + focus trap) while keeping Root /
// Trigger / Item real, so ContextMenuItem's menu context stays intact.
vi.mock('radix-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('radix-ui')>();
  const ctxMenu = actual.ContextMenu;
  return {
    ...actual,
    ContextMenu: {
      ...ctxMenu,
      // Content normally portals + focus-traps; jsdom is fine with a plain
      // wrapper that keeps the menu context provider alive.
      Content: ctxMenu.Group ?? ((props: { children?: React.ReactNode }) => <>{props.children}</>),
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

async function mountTerminal(): Promise<{
  root: Root;
  host: HTMLElement;
  termHost: HTMLElement;
  term: MockTerminalInstance;
}> {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => root.render(<TerminalView sessionId="sess-x" />));
  const term = h.terms[0];
  if (!term) throw new Error('terminal not mounted');
  // The drag-scroll module attaches to the component's inner terminal host.
  const termHost = host.querySelector<HTMLElement>('[data-terminal-host]');
  if (!termHost) throw new Error('terminal host not rendered');
  return { root, host, termHost, term };
}

beforeEach(() => {
  h.terms.length = 0;
  h.listeners.clear();
  clipboardWrites.length = 0;
  clipboardReadResult = 'pasted-text';
  document.body.replaceChildren();
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
