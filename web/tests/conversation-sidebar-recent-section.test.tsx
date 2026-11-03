/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@/components/ui/scroll-area', () => ({
  ScrollArea: ({ children, ...props }: { children: React.ReactNode } & Record<string, unknown>) => (
    <div {...props}>{children}</div>
  ),
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
  it('sits below the workspace groups with exactly one separator per boundary', () => {
    // Codex-style recency placement: workspaces first, recent conversations
    // last; every adjacent section boundary carries exactly one separator.
    const vm = recentSidebar('cursor-2');
    vm.workspaces = [
      { projectId: 'project-a', workspacePath: '/a', name: 'Workspace A' },
      { projectId: 'project-b', workspacePath: '/b', name: 'Workspace B' },
    ];
    const { container, root } = mount(vm, () => {});
    try {
      const recentHeading = container.querySelector('[data-conversation-sidebar-heading="recent"]')!;
      const groups = container.querySelectorAll('[data-conversation-workspace-group]');
      expect(groups.length).toBeGreaterThan(0);
      expect(
        (groups[groups.length - 1] as Node).compareDocumentPosition(recentHeading as Node)
          & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
      // The recent section lives in its own pinned tail region with internal
      // scrolling — any number of workspaces can never push it out of reach.
      const scrollRegion = container.querySelector('[data-conversation-sidebar-region="scrollable-conversations"]')!;
      const recentRegion = container.querySelector('[data-conversation-sidebar-region="recent"]')!;
      expect(scrollRegion.contains(recentHeading)).toBe(false);
      expect(recentRegion.contains(recentHeading)).toBe(true);
      // The bounded scroller carries both a cap and a floor: long lists cap
      // at ~40vh with internal scrolling, empty or single-item lists keep a
      // visible section (heading plus roughly three rows) instead of
      // collapsing to a single line.
      const scroller = recentRegion.querySelector('.gold-themed-scrollbar')!;
      expect(scroller.className).toContain('max-h-[min(40vh,20rem)]');
      expect(scroller.className).toContain('min-h-[9.5rem]');
      // No pinned section: only the workspaces|recent boundary carries one.
      expect(container.querySelectorAll('[data-conversation-sidebar-separator]').length).toBe(1);
    } finally {
      act(() => root.unmount());
    }

    // A pinned section adds its own single boundary separator — never a
    // second parallel line from the section wrappers themselves.
    const withPins = recentSidebar(null);
    withPins.pinRefs = [{ projectId: 'project-a', taskId: 'task-1', taskUuid: 'uuid-1' } as never];
    const pinned = mount(withPins, () => {});
    try {
      expect(pinned.container.querySelector('[data-conversation-sidebar-heading="pinned"]')).toBeTruthy();
      expect(pinned.container.querySelectorAll('[data-conversation-sidebar-separator]').length).toBe(2);
    } finally {
      act(() => pinned.root.unmount());
    }
  });

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

  it('shows an end hint only when a paginated list is exhausted', () => {
    // Exhausted with a full page of rows -> the end hint confirms "no more".
    const paginated = recentSidebar(null);
    paginated.recentTasks = Array.from({ length: 10 }, (_, index) => ({
      ...paginated.recentTasks[0],
      taskId: `task-${index}`,
      taskUuid: `uuid-${index}`,
    }));
    const first = mount(paginated, () => {});
    try {
      expect(first.container.textContent).toContain('conversation.sidebar.allRecentShown');
    } finally {
      act(() => first.root.unmount());
    }
    // Exhausted with fewer rows than one page -> silent end, no noise.
    const short = recentSidebar(null);
    const second = mount(short, () => {});
    try {
      expect(second.container.textContent).not.toContain('conversation.sidebar.allRecentShown');
      expect(second.container.textContent).not.toContain('conversation.sidebar.loadMoreRecent');
    } finally {
      act(() => second.root.unmount());
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

  it('does not claim an empty recent list while more pages remain', () => {
    // A page whose every slot failed server-side yields zero rows but still a
    // cursor: the section keeps loading instead of declaring "no recent".
    const emptyWithCursor = recentSidebar('cursor-2');
    emptyWithCursor.recentTasks = [];
    const first = mount(emptyWithCursor, () => {});
    try {
      expect(first.container.textContent).not.toContain('conversation.sidebar.noRecent');
      expect(first.container.querySelector('[data-load-more-sentinel="recent"]')).toBeTruthy();
    } finally {
      act(() => first.root.unmount());
    }
    // Genuinely exhausted and empty -> the empty state shows.
    const emptyExhausted = recentSidebar(null);
    emptyExhausted.recentTasks = [];
    const second = mount(emptyExhausted, () => {});
    try {
      expect(second.container.textContent).toContain('conversation.sidebar.noRecent');
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
