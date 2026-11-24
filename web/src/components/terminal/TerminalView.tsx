import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { getTerminalHistory, resizeTerminal, subscribeTerminalEvents, writeTerminalInput } from '@/api';
import type { TerminalEventVm } from '@/types';
import { attachTerminalClipboardKeys } from '@/components/terminal/xterm-keyboard-clipboard';
import { attachTerminalDragScroll } from '@/components/terminal/xterm-drag-scroll';
import { TerminalContextMenu } from '@/components/terminal/TerminalContextMenu';

const decode = (base64: string) => {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
};

/** Decodes base64 output bytes to text for a single xterm write. */
const decodeText = (base64: string) => new TextDecoder().decode(decode(base64));

const encode = (input: string) => {
  const bytes = new TextEncoder().encode(input);
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
};

/**
 * Renders a single terminal session via xterm.js. Keyboard input / paste is
 * base64-encoded on the way up; pty output chunks arrive base64-encoded over the
 * terminal event channel and are decoded here. Window resize drives a pty resize
 * after the fit addon recomputes the geometry.
 *
 * T14 cursor handoff: the backend history snapshot reports the ABSOLUTE stream
 * cursors (`base..end`) its bytes span, and every live output chunk carries the
 * same cursor (`endOffset`). Chunks fully covered by what was already written
 * are dropped; `trimmed` snapshots replay their retained suffix with an inline
 * note. The result: history replay and the live stream join with no gap and no
 * duplication, whatever order they resolve in.
 */
export function TerminalView({
  sessionId,
  active = true,
  onSelectionQuote,
}: {
  sessionId: string;
  active?: boolean;
  /** 选区引用（连接→对话）：选中文本回调。缺省则不显示引用按钮。 */
  onSelectionQuote?: (text: string) => void;
}) {
  const { t } = useTranslation();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [selectedText, setSelectedText] = useState<string | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      allowTransparency: true,
      scrollback: 5000,
      theme: { background: 'transparent' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;

    // Restore this session's history from the backend (the single source of
    // truth shared by every window), so a terminal's scrollback survives any
    // remount — including detachment to a pop-out window and back.
    //
    // T14: both sides speak the SAME absolute stream cursor. The snapshot
    // reports the window it replays (`base..end`); every live output chunk
    // carries the cursor through its last byte (`endOffset`). Joining is
    // therefore exact regardless of which arrives first: while the snapshot is
    // in flight, live chunks are buffered with their cursors; once it lands,
    // everything through the snapshot's `end` (snapshot or buffered live
    // bytes) is written once and later chunks are deduped against `writtenTo`.
    let joined = false; // snapshot request settled (success or failure)
    let writtenTo = 0; // absolute cursor through everything written so far
    let pendingLive: Array<{ base64: string; end: number }> = [];

    const writeBase64 = (base64: string) => {
      term.write(decodeText(base64));
    };

    const drainPendingLive = () => {
      // Flush buffered live chunks in arrival order, skipping the part the
      // snapshot already replayed.
      for (const pending of pendingLive) {
        if (pending.end <= writtenTo) continue;
        writeBase64(pending.base64);
        writtenTo = Math.max(writtenTo, pending.end);
      }
      pendingLive = [];
    };

    void getTerminalHistory(sessionId)
      .then((snapshot) => {
        if (disposed) return;
        joined = true;
        if (snapshot.trimmed) {
          // Front-trimmed: the retained window starts at `base`, not 0. Note
          // it in the scrollback instead of pretending nothing is missing.
          term.write('\x1b[90m[older output trimmed]\x1b[0m\r\n');
        }
        if (snapshot.base64) {
          writeBase64(snapshot.base64);
        }
        writtenTo = Math.max(writtenTo, snapshot.end);
        drainPendingLive();
      })
      .catch(() => {
        if (disposed) return;
        // No snapshot available: the live stream is the only source. Chunks
        // buffered so far carry absolute cursors starting from the session
        // origin, so replay them as-is.
        joined = true;
        drainPendingLive();
      });

    const dataDisposable = term.onData((data) => {
      // Surface an input write failure inline rather than silently dropping it
      // (fire-and-forget would hide a pty-write error behind a rejected promise).
      writeTerminalInput(sessionId, encode(data)).catch(() => {
        term.write('\r\n\x1b[90m[input failed]\x1b[0m\r\n');
      });
    });

    // Clipboard keys: Ctrl+C copies when a selection exists (no selection
    // keeps the SIGINT convention), Ctrl+V pastes. macOS Cmd+C/V keep using
    // xterm's native copy/paste event path.
    const clipboardKeysDisposable = attachTerminalClipboardKeys(term);
    // Drag-scroll-to-select: wheel during a drag extends the selection, and
    // holding the pointer past the top/bottom edge auto-scrolls (falls back
    // when xterm's native drag-scroll is not engaging). Attached against the
    // component host — stable DOM, independent of xterm's internal open timing.
    const dragScrollDisposable = attachTerminalDragScroll(term, host);
    const resizeDisposable = term.onResize(({ rows, cols }) => {
      if (rows > 0 && cols > 0) void resizeTerminal(sessionId, rows, cols);
    });
    // 选区引用（连接→对话）：追踪选区有无，供父级浮层按钮引用到对话输入框。
    const selectionDisposable = onSelectionQuote
      ? term.onSelectionChange(() => {
          setSelectedText(term.hasSelection() ? term.getSelection() : null);
        })
      : null;

    // Guard against the async-subscription race: in dev React.StrictMode mounts
    // -> unmounts -> remounts, so `subscribeTerminalEvents` can resolve after the
    // first unmount. Without the flags below a stale listener leaks and later
    // writes pty output to the already-disposed terminal (xterm crashes on
    // `_renderer.value.dimensions`). Never deliver output to a dead terminal.
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void subscribeTerminalEvents((event: TerminalEventVm) => {
      if (disposed) return;
      if (event.sessionId !== sessionId) return;
      if (event.event === 'output') {
        const { base64, endOffset } = event;
        if (!joined) {
          // Snapshot still in flight: buffer with its absolute cursor, flush
          // after the join (same-cursor duplicates are dropped there).
          pendingLive.push({ base64, end: endOffset });
          return;
        }
        // `writtenTo` is where this view has actually written up to; a chunk
        // ending at/below it was already rendered by the snapshot or an
        // earlier live chunk, so replaying it would duplicate output.
        if (endOffset <= writtenTo) return;
        writeBase64(base64);
        writtenTo = Math.max(writtenTo, endOffset);
      } else if (event.event === 'exited') {
        term.write('\r\n\x1b[90m[process exited]\x1b[0m\r\n');
      }
    }).then((fn) => {
      // If we already unmounted before the subscription resolved, drop it now
      // instead of leaving a live listener that writes to the disposed terminal.
      if (disposed) fn();
      else unlisten = fn;
    });

    // Refit and sync the pty size whenever the host box resizes.
    const observer = new ResizeObserver(() => {
      if (disposed) return;
      fit.fit();
    });
    observer.observe(host);

    const focusTerm = () => term.focus();
    host.addEventListener('focus', focusTerm);
    host.addEventListener('mousedown', focusTerm);

    return () => {
      disposed = true;
      observer.disconnect();
      dataDisposable.dispose();
      clipboardKeysDisposable();
      dragScrollDisposable();
      resizeDisposable.dispose();
      selectionDisposable?.dispose();
      host.removeEventListener('focus', focusTerm);
      host.removeEventListener('mousedown', focusTerm);
      unlisten?.();
      termRef.current = null;
      // Defer the terminal disposal past the current macrotask. xterm's Viewport
      // constructor queues `setTimeout(() => this.syncScrollArea())`, and that
      // method reads the unguarded `_renderer.value.dimensions`. Disposing here,
      // synchronously, races that timer (React.StrictMode unmounts a freshly-opened
      // terminal and hits it) and crashes with `_renderer.value.dimensions`.
      // Because the Viewport timer was scheduled during `open()` — before this
      // deferred callback — it fires first, while the terminal is still alive.
      const deferred = term;
      window.setTimeout(() => deferred.dispose(), 0);
    };
  }, [sessionId]);

  // Refit + focus only while this terminal is the active tab. Every session stays
  // mounted (switching tabs toggles visibility, not remount), so a terminal that
  // was hidden while it kept buffering output must be refit (its host just went
  // from display:none to a real size) and focused again when shown.
  useEffect(() => {
    if (!active) return;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    const timer = window.setTimeout(() => {
      fit.fit();
      term.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [active, sessionId]);

  return (
    <div className="relative h-full w-full">
      <TerminalContextMenu termRef={termRef} onQuoteToConversation={onSelectionQuote}>
        <div
          data-terminal-host
          ref={hostRef}
          className="h-full w-full overflow-hidden bg-transparent px-3 py-2"
        />
      </TerminalContextMenu>
      {/* 选区引用（连接→对话）：选中文本后浮出，点击进入会话输入框。 */}
      {onSelectionQuote && selectedText ? (
        <button
          type="button"
          data-terminal-selection-quote="true"
          className="absolute right-3 top-2 z-10 rounded-md border border-border/60 bg-popover px-2 py-1 text-xs text-popover-foreground shadow-sm hover:bg-accent"
          onClick={() => {
            const text = termRef.current?.getSelection() ?? selectedText;
            if (text) onSelectionQuote(text);
            termRef.current?.clearSelection();
            setSelectedText(null);
          }}
        >
          {t('terminal.quoteToConversation')}
        </button>
      ) : null}
    </div>
  );
}
