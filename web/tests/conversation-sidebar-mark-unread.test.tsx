/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return { ...actual, saveConversationPreference: vi.fn().mockResolvedValue(undefined) };
});

vi.mock('@/components/ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

import { ConversationSidebar } from '@/components/conversation/ConversationSidebar';
import type { ConversationSidebarVm, ConversationTaskRowVm, ConversationTerminalResultVm } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const callbacks = {
  onSelect: () => {},
  onNewConversation: () => {},
  onSearch: () => {},
  onPinTask: () => {},
  onUnpinTask: () => {},
  onRenameTask: () => {},
  onDeleteTask: () => {},
  onRetryBootstrap: () => {},
  onRequestWorkspaceTasks: () => {},
  onRequestPinnedTasks: () => {},
  onRequestRecentTasks: () => {},
  onRequestTaskRuns: () => {},
};

const directTask: ConversationTaskRowVm = {
  projectId: 'workspace-a',
  taskId: 'direct-task',
  taskUuid: 'uuid-direct',
  title: 'Direct conversation',
  autoTitle: false,
  runMode: 'direct',
  agentIdentity: { agentType: 'claude-acp', displayName: 'Claude', iconKey: 'claude' },
  unreadTerminalResult: null,
  canMarkUnread: true,
  latestRun: null,
  runs: [],
  runHistoryStatus: 'ready',
  runsNextCursor: null,
  pinned: true,
  resident: false,
};

const unreadResult: ConversationTerminalResultVm = {
  eventId: 'event-1',
  runId: 'run-1',
  kind: 'completed',
  occurredAt: '2026-08-18T10:00:00Z',
};

function sidebarVm(task: ConversationTaskRowVm): ConversationSidebarVm {
  return {
    loadStatus: 'ready',
    workspaces: [{ projectId: 'workspace-a', workspacePath: 'D:\\workspace-a', name: 'Workspace A' }],
    pinRefs: [],
    pinnedTasks: [task],
    pinnedTaskPage: { status: 'ready', nextCursor: null },
    recentTasks: [],
    recentTaskPage: { status: 'not-loaded', nextCursor: null },
    tasksByWorkspace: { 'workspace-a': [] },
    workspaceTaskPages: { 'workspace-a': { status: 'ready', nextCursor: null } },
    lastActiveWorkspaceId: 'workspace-a',
  };
}

async function openMenu(container: HTMLElement) {
  const trigger = container.querySelector('[data-slot="context-menu-trigger"]');
  expect(trigger).not.toBeNull();
  await act(async () => {
    trigger!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, button: 2, clientX: 20, clientY: 20 }));
  });
  return Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]'));
}

function menuLabels(items: HTMLElement[]) {
  return items.map((item) => item.textContent ?? '');
}

afterEach(() => {
  document.body.replaceChildren();
});

describe('Direct mark-unread context menu', () => {
  it('sits next to the residency toggle and marks the task unread on click', async () => {
    const container = document.createElement('div'); document.body.append(container);
    const root = createRoot(container);
    const onSetResident = vi.fn().mockResolvedValue(undefined);
    const onMarkUnread = vi.fn().mockResolvedValue(undefined);
    try {
      await act(async () => root.render(
        <ConversationSidebar {...callbacks} onSetResident={onSetResident} onMarkUnread={onMarkUnread} vm={sidebarVm(directTask)} active={{ kind: 'conversation-home' }} />,
      ));
      const items = await openMenu(container);
      expect(menuLabels(items)).toEqual([
        'conversation.sidebar.keepResident',
        'conversation.sidebar.markUnread',
      ]);
      await act(async () => items[1]!.click());
      expect(onMarkUnread).toHaveBeenCalledWith('workspace-a', 'direct-task');
      expect(onSetResident).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
    }
  });

  it('hides the mark item once the row already carries an unread result', async () => {
    const container = document.createElement('div'); document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ConversationSidebar {...callbacks} onSetResident={vi.fn()} onMarkUnread={vi.fn()} vm={sidebarVm({ ...directTask, unreadTerminalResult: unreadResult })} active={{ kind: 'conversation-home' }} />,
      ));
      const items = await openMenu(container);
      expect(menuLabels(items)).toEqual(['conversation.sidebar.keepResident']);
    } finally {
      await act(async () => root.unmount());
    }
  });

  it('hides the mark item when the backend does not project a restorable terminal result', async () => {
    const container = document.createElement('div'); document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ConversationSidebar {...callbacks} onSetResident={vi.fn()} onMarkUnread={vi.fn()} vm={sidebarVm({ ...directTask, canMarkUnread: false })} active={{ kind: 'conversation-home' }} />,
      ));
      const items = await openMenu(container);
      expect(menuLabels(items)).toEqual(['conversation.sidebar.keepResident']);
    } finally {
      await act(async () => root.unmount());
    }
  });

  it('keeps the residency-only menu when no mark handler is wired', async () => {
    const container = document.createElement('div'); document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ConversationSidebar {...callbacks} onSetResident={vi.fn()} vm={sidebarVm(directTask)} active={{ kind: 'conversation-home' }} />,
      ));
      const items = await openMenu(container);
      expect(menuLabels(items)).toEqual(['conversation.sidebar.keepResident']);
    } finally {
      await act(async () => root.unmount());
    }
  });
});
