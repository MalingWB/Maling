/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@/components/ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

import { ConversationSidebar } from '@/components/conversation/ConversationSidebar';
import { applyConversationRecentTaskPage, createEmptyConversationSidebar } from '@/lib/conversation-sidebar-loading';
import type { ConversationPage, ConversationSidebarVm } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

function recentSidebar(nextCursor: string | null) {
  const sidebar = createEmptyConversationSidebar();
  return applyConversationRecentTaskPage(sidebar, {
    tasks: [
      {
        projectId: 'project-a', taskId: 'task-1', taskUuid: 'uuid-1', title: 'Recent A', autoTitle: false,
        runMode: 'direct', latestRun: { runId: 'run-9', status: 'completed', updatedAt: '2026-10-01T00:00:00Z' } as never,
        runs: [], runHistoryStatus: 'not-loaded', runsNextCursor: null, pinned: false, workspaceName: 'Workspace A',
      },
      {
        projectId: 'project-b', taskId: 'task-2', taskUuid: 'uuid-2', title: 'Recent B (no run)', autoTitle: false,
        runMode: 'workflow', latestRun: null, runs: [], runHistoryStatus: 'not-loaded', runsNextCursor: null,
        pinned: false, workspaceName: 'Workspace B',
      },
    ],
    nextCursor,
    errors: [],
  }, false);
}

function mount(vm: ConversationSidebarVm, onSelect: (page: ConversationPage) => void) {
  const container = document.createElement('div');
  document.body.append(container);
  const root: Root = createRoot(container);
  act(() => {
    root.render(
      <ConversationSidebar
        vm={vm}
        active={{ kind: 'conversation-home' }}
        onSelect={onSelect}
        onNewConversation={() => {}}
        onSearch={() => {}}
        onPinTask={() => {}}
        onUnpinTask={() => {}}
        onRenameTask={() => {}}
        onDeleteTask={() => {}}
        onRetryBootstrap={() => {}}
        onRequestWorkspaceTasks={() => {}}
        onRequestPinnedTasks={() => {}}
        onRequestRecentTasks={() => {}}
        onRequestTaskRuns={() => {}}
      />,
    );
  });
  return { container, root };
}

afterEach(() => {
  document.body.replaceChildren();
});

describe('ConversationSidebar recent section', () => {
  it('renders with a single workspace (always visible)', () => {
    const vm = {
      ...recentSidebar(null),
      workspaces: [{ projectId: 'project-a', workspacePath: '/a', name: 'Workspace A' }],
    };
    const { container, root } = mount(vm, () => {});
    try {
      expect(container.querySelector('[data-conversation-sidebar-heading="recent"]')).toBeTruthy();
      expect(container.querySelector('[data-recent-task-id="task-1"]')).toBeTruthy();
    } finally {
      act(() => root.unmount());
    }
  });

  it('shows the load-more button and sentinel only while more pages remain', () => {
    const more = recentSidebar('cursor-2');
    const exhausted = recentSidebar(null);
    const first = mount(more, () => {});
    try {
      expect(first.container.textContent).toContain('conversation.sidebar.loadMoreRecent');
      expect(first.container.querySelector('[data-load-more-sentinel="recent"]')).toBeTruthy();
    } finally {
      act(() => first.root.unmount());
    }
    const second = mount(exhausted, () => {});
    try {
      expect(second.container.textContent).not.toContain('conversation.sidebar.loadMoreRecent');
      expect(second.container.querySelector('[data-load-more-sentinel="recent"]')).toBeNull();
    } finally {
      act(() => second.root.unmount());
    }
  });

  it('navigates with the full locator and tolerates rows without a run', () => {
    const onSelect = vi.fn();
    const { container, root } = mount(recentSidebar(null), onSelect);
    try {
      const row = container.querySelector('[data-recent-task-id="task-1"]') as HTMLElement;
      act(() => {
        row.click();
      });
      expect(onSelect).toHaveBeenCalledTimes(1);
      expect(onSelect).toHaveBeenCalledWith({
        kind: 'conversation-run',
        projectId: 'project-a',
        taskId: 'task-1',
        taskUuid: 'uuid-1',
        runId: 'run-9',
      });

      // A row whose task has no run yet must not navigate or throw.
      const noRun = container.querySelector('[data-recent-task-id="task-2"]') as HTMLElement;
      act(() => {
        noRun.click();
      });
      expect(onSelect).toHaveBeenCalledTimes(1);
    } finally {
      act(() => root.unmount());
    }
  });

  it('keeps the workspace load-more button hidden once the workspace list is exhausted', async () => {
    const vm = recentSidebar(null);
    vm.workspaces = [{ projectId: 'project-a', workspacePath: '/a', name: 'Workspace A' }];
    vm.lastActiveWorkspaceId = 'project-a';
    vm.tasksByWorkspace = {
      'project-a': [{
        projectId: 'project-a', taskId: 'task-1', taskUuid: 'uuid-1', title: 'Only', autoTitle: false,
        runMode: 'direct', runs: [], runHistoryStatus: 'ready', runsNextCursor: null, pinned: false,
      }],
    };
    vm.workspaceTaskPages = { 'project-a': { status: 'ready', nextCursor: null } };
    const container = document.createElement('div');
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => {
      root.render(
        <ConversationSidebar
          vm={vm}
          active={{ kind: 'conversation-home' }}
          onSelect={() => {}}
          onNewConversation={() => {}}
          onSearch={() => {}}
          onPinTask={() => {}}
          onUnpinTask={() => {}}
          onRenameTask={() => {}}
          onDeleteTask={() => {}}
          onRetryBootstrap={() => {}}
          onRequestWorkspaceTasks={() => {}}
          onRequestPinnedTasks={() => {}}
          onRequestRecentTasks={() => {}}
          onRequestTaskRuns={() => {}}
        />,
      );
    });
    try {
      // The auto-expanded workspace renders its rows; with the page exhausted
      // neither the load-more button nor the auto-load sentinel may appear.
      expect(container.querySelector('[data-conversation-workspace-group="project-a"] [data-task-id="task-1"]')).toBeTruthy();
      expect(container.textContent).not.toContain('conversation.sidebar.loadMoreConversations');
      expect(container.querySelector('[data-load-more-sentinel="workspace:project-a"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
    }
  });
});
