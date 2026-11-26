import type { Terminal } from '@xterm/xterm';

/**
 * Terminal clipboard keyboard integration.
 *
 * Root cause being fixed (see xterm 5.5.0 source):
 * - xterm only binds copy to the DOM `copy` event (fired by the platform
 *   shortcut, e.g. Cmd+C on macOS) and paste to the DOM `paste` event.
 * - On Ctrl+C/Ctrl+V xterm's key handler converts the key into ETX/SYN bytes
 *   and sends them to the pty (`Terminal._keyDown` → `triggerDataEvent`), so
 *   a selection is never copied and the paste event never fires.
 *
 * Fix strategy — intercept via xterm's official `attachCustomKeyEventHandler`
 * (the handler runs BEFORE xterm's own key processing; returning `false`
 * cancels xterm's handling entirely):
 * - Ctrl+C with a selection  → copy the selection, cancel (no SIGINT).
 * - Ctrl+C without selection → let xterm send ETX (terminal SIGINT convention).
 * - Ctrl+Shift+C             → copy even without… with a selection (standard
 *                              Linux terminal shortcut), cancel.
 * - Ctrl+V / Ctrl+Shift+V    → paste: read the webview clipboard and feed it
 *                              through `term.paste()` so xterm applies its own
 *                              newline normalization + bracketed paste mode.
 *                              Falls back to an inline hint when the webview
 *                              denies clipboard reads.
 * - Cmd+C / Cmd+V (macOS)    → untouched; the native copy/paste event path
 *                              already works in this webview.
 * - Everything else          → untouched (return true).
 */

const READ_CLIPBOARD_FAILED_HINT = '\r\n\x1b[90m[clipboard read denied - use Cmd+V]\x1b[0m\r\n';

/** Ctrl (or Shift) modifier state of a keyboard event, platform-neutral. */
function isCtrlLike(event: KeyboardEvent): boolean {
  // On macOS Ctrl is still Ctrl (Cmd is meta), so this works uniformly.
  return event.ctrlKey && !event.metaKey;
}

function isKeyC(event: KeyboardEvent): boolean {
  return event.key === 'c' || event.key === 'C';
}

function isKeyV(event: KeyboardEvent): boolean {
  return event.key === 'v' || event.key === 'V';
}

/**
 * Installs the clipboard keyboard handling on a terminal.
 *
 * @param term the xterm instance (must already be created; `open()` not required).
 * @param opts.onClipboardDenied optional callback invoked when a paste fails
 *   because the webview denied clipboard access — allows the host to surface
 *   an inline hint. When omitted, the hint is written into the terminal itself.
 * @returns a disposer that detaches the handler.
 */
export function attachTerminalClipboardKeys(
  term: Terminal,
  opts: { onClipboardDenied?: () => void } = {},
): () => void {
  const handler = (event: KeyboardEvent): boolean => {
    // Copy paths ------------------------------------------------------------

    // Cmd+C (macOS native copy): xterm's copy-event binding already handles it.
    if (event.metaKey && !event.ctrlKey && isKeyC(event) && event.type === 'keydown') {
      return true;
    }

    if (isCtrlLike(event) && isKeyC(event) && event.type === 'keydown') {
      if (event.shiftKey) {
        // Ctrl+Shift+C: copy (conventionally works even with no selection —
        // a no-op selection copy is harmless).
        void copySelection(term);
        return false;
      }
      if (term.hasSelection()) {
        // Ctrl+C with a selection copies instead of interrupting the process.
        void copySelection(term);
        return false;
      }
      // No selection: keep the terminal convention — ETX/SIGINT goes to pty.
      return true;
    }

    // Paste paths -----------------------------------------------------------

    if (event.metaKey && !event.ctrlKey && isKeyV(event) && event.type === 'keydown') {
      // Cmd+V: the webview fires a native `paste` event that xterm already
      // listens to. Do not intercept.
      return true;
    }

    if (isCtrlLike(event) && isKeyV(event) && event.type === 'keydown') {
      // Ctrl+V / Ctrl+Shift+V: paste from the webview clipboard. Cancel the
      // key so xterm does not send SYN to the pty.
      void pasteFromClipboard(term, opts);
      return false;
    }

    return true;
  };

  term.attachCustomKeyEventHandler(handler);
  return () => {
    // xterm exposes no detach API for the custom key handler; resetting is the
    // documented way to clear it. The terminal is owned by this component, so
    // clearing on teardown cannot affect other consumers.
    term.attachCustomKeyEventHandler(() => true);
  };
}

async function copySelection(term: Terminal): Promise<void> {
  const text = term.getSelection();
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Clipboard write denial is rare in a focused webview; keep silent to
    // avoid polluting the terminal stream on an opportunistic shortcut.
  }
}

async function pasteFromClipboard(
  term: Terminal,
  opts: { onClipboardDenied?: () => void },
): Promise<void> {
  let text: string | null = null;
  try {
    text = await navigator.clipboard.readText();
  } catch {
    text = null;
  }
  if (text === null) {
    // Webview denied clipboard access — surface a hint instead of failing
    // silently. Prefers the host-provided callback; falls back to writing
    // an inline note into the terminal (same convention as `[input failed]`).
    if (opts.onClipboardDenied) {
      opts.onClipboardDenied();
    } else {
      term.write(READ_CLIPBOARD_FAILED_HINT);
    }
    return;
  }
  // `term.paste` normalizes newlines and applies bracketed paste mode.
  term.paste(text);
}
