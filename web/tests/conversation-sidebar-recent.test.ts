import { describe, expect, it } from 'vitest';

import {
  CONVERSATION_SIDEBAR_RECENT_WINDOW,
  applyConversationRecentTaskPage,
  applyConversationSidebarBootstrap,
  beginConversationRecentTaskLoad,
  createEmptyConversationSidebar,
  failConversationRecentTaskLoad,
} from '@/lib/conversation-sidebar-loading';
import { applyConversationTaskSnapshot } from '@/lib/conversation-task-state';
import { applyConversationSidebarRunStateUpdate, applyConversationSidebarTaskActivity } from '@/lib/conversation-sidebar-activity';

function recentRow(overrides: Partial<Parameters<typeof applyConversationRecentTaskPage>[1]['tasks'][number]> = {}) {
  return {
    projectId: 'a', taskId: 'task-1', taskUuid: 'uuid-1', title: 'Task A', autoTitle: false,
    runMode: 'direct' as const, runs: [], runHistoryStatus: 'not-loaded' as const, pinned: false,
    workspaceName: 'Workspace A', ...overrides,
  };
}

function sidebarWithRecent(recentTasks: ReturnType<typeof recentRow>[]) {
  return {
    ...createEmptyConversationSidebar(),
    workspaces: [
      { projectId: 'a', name: 'Workspace A', workspacePath: '/a' },
      { projectId: 'b', name: 'Workspace B', workspacePath: '/b' },
    ],
    recentTasks,
    recentTaskPage: { status: 'ready' as const, nextCursor: null },
  };
}

describe('conversation sidebar recent tasks', () => {
  it('pages the merged list with cursor pagination and a bounded window', () => {
    let sidebar = createEmptyConversationSidebar();
    sidebar = beginConversationRecentTaskLoad(sidebar);
    expect(sidebar.recentTaskPage.status).toBe('loading');

    const first = Array.from({ length: 10 }, (_, index) => recentRow({ taskId: `task-${index}`, taskUuid: `uuid-${index}` }));
    sidebar = applyConversationRecentTaskPage(sidebar, { tasks: first, nextCursor: 'cursor-1', errors: [] }, false);
    expect(sidebar.recentTasks).toHaveLength(10);
    expect(sidebar.recentTaskPage).toEqual({ status: 'ready', nextCursor: 'cursor-1' });

    // Append dedupes by stable task identity and keeps the window bound.
    const appended = [first[0], ...Array.from({ length: 10 }, (_, index) => recentRow({ projectId: 'b', taskId: `task-b-${index}`, taskUuid: `uuid-b-${index}`, workspaceName: 'Workspace B' }))];
    const full = Array.from({ length: CONVERSATION_SIDEBAR_RECENT_WINDOW - 19 }, (_, index) => recentRow({ taskId: `task-fill-${index}`, taskUuid: `uuid-fill-${index}` }));
    sidebar = applyConversationRecentTaskPage(sidebar, { tasks: [...appended, ...full], nextCursor: 'more', errors: [] }, true);
    expect(sidebar.recentTasks).toHaveLength(CONVERSATION_SIDEBAR_RECENT_WINDOW);
    expect(new Set(sidebar.recentTasks.map((task) => task.taskUuid)).size).toBe(CONVERSATION_SIDEBAR_RECENT_WINDOW);
    expect(sidebar.recentTaskPage.nextCursor).toBeNull();

    sidebar = failConversationRecentTaskLoad(sidebar);
    expect(sidebar.recentTaskPage.status).toBe('error');
  });

  it('replaces the first page so a refresh cannot duplicate rows', () => {
    const first = [recentRow({ taskId: 'task-1' }), recentRow({ taskId: 'task-2' })];
    let sidebar = applyConversationRecentTaskPage(createEmptyConversationSidebar(), { tasks: first, nextCursor: null, errors: [] }, false);
    sidebar = applyConversationRecentTaskPage(sidebar, { tasks: [recentRow({ taskId: 'task-1' })], nextCursor: null, errors: [] }, false);
    expect(sidebar.recentTasks.map((task) => task.taskId)).toEqual(['task-1']);
  });

  it('restarts the recent page when the workspace set changes', () => {
    let sidebar = sidebarWithRecent([recentRow()]);
    sidebar = applyConversationSidebarBootstrap(sidebar, {
      workspaces: [{ projectId: 'b', name: 'Workspace B', workspacePath: '/b' }],
      pinRefs: [],
      lastActiveWorkspaceId: 'b',
      preferences: {},
    });
    expect(sidebar.recentTasks).toHaveLength(0);
    expect(sidebar.recentTaskPage).toEqual({ status: 'not-loaded', nextCursor: null });
  });

  it('projects activity updates into recent rows and moves advanced rows to the front', () => {
    const older = recentRow({ taskId: 'task-old', taskUuid: 'uuid-old', lastActivityAt: '2026-01-01T00:00:00Z' });
    const newer = recentRow({ projectId: 'b', taskId: 'task-new', taskUuid: 'uuid-new', workspaceName: 'Workspace B', lastActivityAt: '2026-06-01T00:00:00Z' });
    const sidebar = sidebarWithRecent([newer, older]);

    const updated = applyConversationSidebarTaskActivity(sidebar, 'a', 'task-old', { phase: 'prompt', stopping: false }, '2026-07-01T00:00:00Z');
    expect(updated.recentTasks[0].taskId).toBe('task-old');
    expect(updated.recentTasks[0].lastActivityAt).toBe('2026-07-01T00:00:00Z');
    expect(updated.recentTasks[0].workspaceName).toBe('Workspace A');
  });

  it('projects run-state updates into recent rows without reordering', () => {
    const row = recentRow({ runMode: 'workflow', taskId: 'task-1', latestRun: { runId: 'run-1', status: 'running', updatedAt: '2026-01-01T00:00:00Z' } as never });
    const sidebar = sidebarWithRecent([row]);
    const updated = applyConversationSidebarRunStateUpdate(sidebar, {
      projectId: 'a', taskId: 'task-1', taskUuid: 'uuid-1', runId: 'run-1', status: 'completed', outcome: 'success',
    } as never);
    expect(updated.recentTasks[0].latestRun?.status).toBe('completed');
    expect(updated.recentTasks[0].workspaceName).toBe('Workspace A');
  });

  it('upserts task snapshots into recent rows and adds new tasks to the front', () => {
    const existing = recentRow({ taskId: 'task-1', lastActivityAt: '2026-01-01T00:00:00Z' });
    let sidebar = sidebarWithRecent([existing]);

    // Existing row is updated in place, keeping the workspace label.
    const updated = applyConversationTaskSnapshot(sidebar, { ...existing, title: 'Renamed', workspaceName: undefined } as never);
    expect(updated.recentTasks).toHaveLength(1);
    expect(updated.recentTasks[0].title).toBe('Renamed');
    expect(updated.recentTasks[0].workspaceName).toBe('Workspace A');

    // A brand-new task joins the merged front with its workspace name resolved.
    const created = applyConversationTaskSnapshot(updated, {
      projectId: 'b', taskId: 'task-9', taskUuid: 'uuid-9', title: 'New B', autoTitle: true, runMode: 'direct',
      runs: [], runHistoryStatus: 'not-loaded', pinned: false,
    } as never);
    expect(created.recentTasks[0].taskId).toBe('task-9');
    expect(created.recentTasks[0].workspaceName).toBe('Workspace B');
  });
});
