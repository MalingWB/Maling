/** @vitest-environment jsdom */
// Verifies TerminalView does not write to a disposed terminal when the async
// event subscription resolves after the component has unmounted. In dev,
// React.StrictMode mounts -> unmounts -> remounts, and if the terminal's own
// `subscribeTerminalEvents(...).then(...)` resolves after the first unmount, the
// stale listener leaks and later delivers pty output to the disposed first
// terminal, crashing xterm with `_renderer.value.dimensions` (undefined).
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';
import { TerminalView } from '@/components/terminal/TerminalView';

// jsdom does not ship ResizeObserver nor opt React into the act() contract.
Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});

const h = vi.hoisted(() => {
  const terms: { writesAfterDispose: number; writes: string[]; disposed: boolean }[] = [];
  const listeners: Set<(event: Record<string, unknown>) => void> = new Set();
  type Deferred = { resolve: (unlisten: () => void) => void; unlisten: () => void };
  const deferreds: Deferred[] = [];
  const history: Record<string, { base64: string; base: number; end: number; trimmed: boolean }> = {};
  const emit = (event: Parameters<Set<unknown>['add']>[0]) => {
    for (const listener of Array.from(listeners)) listener(event as never);
  };
  return { terms, listeners, deferreds, emit, history };
});

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    writesAfterDispose = 0;
    writes: string[] = [];
    disposed = false;
    onDataCb: (() => void) | null = null;
    constructor() { h.terms.push(this); }
    loadAddon() {}
    open() {}
    onData() { return { dispose() {} }; }
    onResize() { return { dispose() {} }; }
    attachCustomKeyEventHandler() {}
    attachCustomWheelEventHandler() {}
    hasSelection() { return false; }
    getSelection() { return ''; }
    clearSelection() {}
    paste() {}
    scrollLines() {}
    buffer = { active: { viewportY: 0, length: 1 } };
    cols = 80;
    rows = 24;
    write(s: string) {
      // content is base64-encoded in the event; here it is plain text.
      if (this.disposed) this.writesAfterDispose += 1;
      else this.writes.push(s);
    }
    focus() {}
    dispose() { this.disposed = true; }
  },
}));

vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { loadAddon() {} fit() {} } }));

vi.mock('@/api', () => ({
  subscribeTerminalEvents: (listener: (event: unknown) => void) =>
    new Promise<void>((resolve) => {
      h.listeners.add(listener as never);
      h.deferreds.push({
        resolve: (unlisten) => resolve(unlisten),
        unlisten: () => h.listeners.delete(listener as never),
      });
    }),
  writeTerminalInput: () => Promise.resolve(),
  resizeTerminal: () => Promise.resolve(),
  getTerminalHistory: (sessionId: string) =>
    Promise.resolve(
      h.history[sessionId] ?? { base64: '', base: 0, end: 0, trimmed: false },
    ),
}));

describe('TerminalView event subscription lifecycle', () => {
  beforeEach(() => {
    h.terms.length = 0;
    h.listeners.clear();
    h.deferreds.length = 0;
    Object.keys(h.history).forEach((key) => delete h.history[key]);
    document.body.replaceChildren();
  });

  it('does not write to a disposed terminal after the subscription resolves post-unmount', async () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    let root: Root = createRoot(host);

    await act(async () => root.render(<TerminalView sessionId="sess-1" />));
    const term = h.terms[0];
    expect(term).toBeDefined();

    // Unmount before the async subscription promise resolves (StrictMode does
    // exactly this during its double-mount).
    await act(async () => root.unmount());
    // Terminal disposal is intentionally deferred one macrotask (it must outlive
    // xterm's own pending Viewport syncScrollArea timer); flush it.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(term.disposed).toBe(true);

    // The subscription .then resolves AFTER unmount.
    await act(async () => {
      h.deferreds[0].resolve(h.deferreds[0].unlisten);
    });

    // Late pty output targeted at the (now disposed) session.
    await act(async () => {
      h.emit({ event: 'output', sessionId: 'sess-1', base64: 'aGVsbG8=' });
    });

    // A stale listener must never deliver output to a disposed terminal.
    expect(term.writesAfterDispose).toBe(0);
  });

  it('does not recreate the xterm when only `active` toggles (keeps history on tab switch)', async () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    let root: Root = createRoot(host);

    await act(async () => root.render(<TerminalView sessionId="sess-1" active />));
    const term = h.terms[0];
    expect(h.terms).toHaveLength(1);
    // Writing output then it should be visible; mark it so we can confirm the
    // same instance survives the toggle.
    expect(term).toBeDefined();

    // Toggle active false -> true (what the panel does when switching tabs). The
    // component stays mounted (same key/sessionId), so no new xterm is created —
    // that is exactly what preserves the terminal's rendered history/scrollback.
    await act(async () => root.render(<TerminalView sessionId="sess-1" active={false} />));
    await act(async () => root.render(<TerminalView sessionId="sess-1" active />));

    expect(h.terms).toHaveLength(1);
    expect(h.terms[0]).toBe(term);
  });

  it('replays a session\'s backend history when the terminal mounts (detach reattach)', async () => {
    // The backend (shared by every window) holds a session's output; on mount we
    // fetch and replay it so a detach-to-pop-out-window + return still shows the
    // full scrollback, not a fresh terminal. The snapshot carries the absolute
    // stream cursors (T14) the view joins the live stream against.
    h.history['sess-replay'] = { base64: btoa('HISTORY_TEXT\r\n$ '), base: 0, end: 16, trimmed: false };
    const host = document.createElement('div');
    document.body.appendChild(host);
    let root: Root = createRoot(host);
    await act(async () => { root.render(<TerminalView sessionId="sess-replay" />); });

    const term = h.terms[0];
    expect(term).toBeDefined();
    // flush the async getTerminalHistory replay
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(term.writes.join('')).toContain('HISTORY_TEXT');

    delete h.history['sess-replay'];
  });

  it('drops live chunks the snapshot already replayed and writes only the tail (T14 cursor join)', async () => {
    // The snapshot resolves while live chunks are still buffered: bytes the
    // snapshot covers must be written exactly once, later chunks as-is.
    h.history['sess-join'] = { base64: btoa('SNAPSHOT\r\n'), base: 0, end: 10, trimmed: false };
    const host = document.createElement('div');
    document.body.appendChild(host);
    let root: Root = createRoot(host);
    await act(async () => { root.render(<TerminalView sessionId="sess-join" />); });
    const term = h.terms[0];

    // Live chunks arriving BEFORE the snapshot promise resolves get buffered:
    // one fully covered by the snapshot (skipped), one straddling/past it.
    const resolved = h.history['sess-join'];
    await act(async () => {
      h.emit({ event: 'output', sessionId: 'sess-join', base64: btoa('SNAP'), endOffset: 4 });
      h.emit({ event: 'output', sessionId: 'sess-join', base64: btoa('NEW STUFF'), endOffset: 19 });
    });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });

    const joined = term.writes.join('');
    // The snapshot replay is present exactly once...
    expect(joined.split('SNAPSHOT').length - 1).toBe(1);
    // ...the buffered chunk fully inside the snapshot was dropped...
    expect(joined).not.toContain('SNAPNEW');
    // ...and the chunk past the snapshot end was written after it.
    expect(joined.indexOf('SNAPSHOT')).toBeLessThan(joined.indexOf('NEW STUFF'));
    expect(resolved).toBeDefined();

    // A later live chunk ending at/below `writtenTo` (19) is deduped.
    await act(async () => {
      h.emit({ event: 'output', sessionId: 'sess-join', base64: btoa('OLD'), endOffset: 19 });
    });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(term.writes.join('')).not.toContain('OLD');

    // A genuinely new chunk is written.
    await act(async () => {
      h.emit({ event: 'output', sessionId: 'sess-join', base64: btoa('FRESH'), endOffset: 24 });
    });
    expect(term.writes.join('')).toContain('FRESH');

    delete h.history['sess-join'];
  });
});
