import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ClipboardPaste, Quote, Scan, Copy } from 'lucide-react';
import type { Terminal } from '@xterm/xterm';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu';

/**
 * Right-click menu for the terminal surface.
 *
 * Root cause being fixed: xterm's right-click handling
 * (`Clipboard.rightClickHandler`) only prepares its hidden textarea for the
 * WEBVIEW's native context menu — it never shows a menu itself. This desktop
 * app suppresses native context menus (see Shell/Workspace shells), so right
 * click did nothing. This component renders an in-app menu instead:
 * copy / paste / select-all / quote-to-conversation.
 *
 * TREE IDENTITY CONTRACT (regression guard): the terminal host div must NEVER
 * be remounted after xterm has `open()`ed into it — a remount destroys the
 * xterm DOM (screen/viewport live inside it) and the terminal renders blank.
 * Therefore:
 * - The menu tree is stable from the FIRST render: the host div is the
 *   ContextMenuTrigger (asChild + Slot) from the start, so no later state
 *   flip can change the element type at that position.
 * - Radix merges its handlers onto the existing node without adding DOM.
 * - Menu actions read the terminal through the ref lazily at click time;
 *   before the terminal effect has run the items are simply disabled.
 *
 * Paste uses `navigator.clipboard.readText()`; when the webview denies the
 * read the item degrades to a disabled hint instead of failing silently.
 */
export function TerminalContextMenu({
  termRef,
  children,
  onQuoteToConversation,
}: {
  termRef: React.RefObject<Terminal | null>;
  children: React.ReactNode;
  /** Present only when selection quoting is wired (connection → conversation). */
  onQuoteToConversation?: (text: string) => void;
}) {
  const { t } = useTranslation();
  const [canReadClipboard, setCanReadClipboard] = useState<boolean | null>(null);
  const [mounted, setMounted] = useState(false);

  // The trigger (and thus the host div) mounts with the component; menu items
  // enable once the terminal effect has run (termRef.current is set).
  useEffect(() => {
    setMounted(true);
  }, []);

  // Probe clipboard read permission once. `null` = unknown (still probing) —
  // the paste item stays enabled and degrades at click time in that case.
  // Guarded access: jsdom and older webviews expose no clipboard at all.
  useEffect(() => {
    let cancelled = false;
    const clipboard = globalThis.navigator?.clipboard;
    if (!clipboard) {
      setCanReadClipboard(false);
      return;
    }
    clipboard
      .readText()
      .then(() => {
        if (!cancelled) setCanReadClipboard(true);
      })
      .catch(() => {
        if (!cancelled) setCanReadClipboard(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const pasteDisabled = canReadClipboard === false;

  const copySelection = () => {
    const term = termRef.current;
    const text = term?.getSelection();
    if (text) void globalThis.navigator?.clipboard?.writeText(text);
  };

  const paste = () => {
    globalThis.navigator?.clipboard
      ?.readText()
      .then((text: string) => termRef.current?.paste(text))
      .catch(() => setCanReadClipboard(false));
  };

  return (
    <ContextMenu>
      {/* The host div itself is the trigger — asChild merges Radix's context
          menu handlers onto it without adding DOM or remounting it. */}
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-44">
        <ContextMenuItem
          data-terminal-menu-copy="true"
          disabled={!mounted || !termRef.current?.hasSelection()}
          onSelect={copySelection}
        >
          <Copy className="size-3.5" />
          <span>{t('terminal.contextMenu.copy')}</span>
        </ContextMenuItem>
        <ContextMenuItem
          data-terminal-menu-paste="true"
          disabled={!mounted || pasteDisabled}
          onSelect={paste}
        >
          <ClipboardPaste className="size-3.5" />
          <span>{pasteDisabled ? t('terminal.contextMenu.pasteDenied') : t('terminal.contextMenu.paste')}</span>
        </ContextMenuItem>
        <ContextMenuItem
          data-terminal-menu-select-all="true"
          disabled={!mounted}
          onSelect={() => termRef.current?.selectAll()}
        >
          <Scan className="size-3.5" />
          <span>{t('terminal.contextMenu.selectAll')}</span>
        </ContextMenuItem>
        {onQuoteToConversation ? (
          <ContextMenuItem
            data-terminal-menu-quote="true"
            disabled={!mounted || !termRef.current?.hasSelection()}
            onSelect={() => {
              const text = termRef.current?.getSelection();
              if (text) onQuoteToConversation(text);
            }}
          >
            <Quote className="size-3.5" />
            <span>{t('terminal.quoteToConversation')}</span>
          </ContextMenuItem>
        ) : null}
      </ContextMenuContent>
    </ContextMenu>
  );
}
