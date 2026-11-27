import { browserApi } from '@/api/browser';
import type { RuntimeApi } from '@/api/client';
import { mockErrorBlockedConversationRun } from '@/mockData';
import type { ConversationRecentTaskRowVm } from '@/types';

// Standalone fixture for the real App, also usable as a Vite browser preload.
// The workbench has no selected workspace. Locally numbered tasks collide across owners.
export function forgeHistoryFixture(registered = true) {
  const workspaces = registered ? [
    { projectId: 'atlas', name: '星图平台', workspacePath: 'D:/Projects/Atlas' },
    { projectId: 'ledger', name: '账务中心', workspacePath: 'D:/Projects/Ledger' },
  ] : [];
  const rows = workspaces.map(({ projectId, name }, index): ConversationRecentTaskRowVm => {
    const lastActivityAt = index ? '2026-11-25T09:30:00Z' : '2026-11-26T08:15:00Z';
    const run = { runId: 'run-001', status: 'completed', outcome: 'success', startedAt: lastActivityAt, updatedAt: lastActivityAt, resumable: false };
    return { projectId, workspaceName: name, taskId: 'task-001', taskUuid: `${projectId}-instance`,
      title: index ? '对账流程试运行' : '发布回归工作流', origin: index ? 'forge-trial' : 'forge', lastActivityAt,
      runMode: index ? 'workflow' : 'direct', latestRun: index ? null : run, runs: index ? [] : [run],
      runHistoryStatus: 'ready', autoTitle: false, pinned: true };
  });
  const bootstrap = () => ({ workspaces, pinRefs: rows.map(({projectId, taskId}) => ({projectId, taskId})), lastActiveWorkspaceId: null, preferences: {} });
  const getAppBootstrap = browserApi.getAppBootstrap.bind(browserApi);
  const methods = {
    getAppBootstrap: async () => ({ ...await getAppBootstrap(), repoRoot: '', recentWorkspaces: [], needsWorkspace: true }),
    getConversationWorkspaces: async () => workspaces,
    getConversationSidebarBootstrap: async () => bootstrap(),
    getGenerationHistoryPage: async ({ projectId, origin, query }) => ({ tasks: rows.filter(row => (!projectId || row.projectId === projectId) && (!origin || row.origin === origin) && (!query || row.title.includes(query))), errors: [], nextCursor: null }),
    getConversationTaskPage: async (projectId) => ({ projectId, tasks: rows.filter(row => row.projectId === projectId), errors: [], nextCursor: null }),
    getConversationRecentTaskPage: async () => ({ tasks: [...rows], errors: [], nextCursor: null }),
    getConversationPinnedTaskPage: async () => ({ tasks: [...rows], errors: [], nextCursor: null }),
    getConversationRun: async (projectId, taskId, runId) => {
      const row = rows.find(r => r.projectId === projectId && r.taskId === taskId && r.latestRun?.runId === runId);
      if (!row) throw new Error('unknown run');
      return { ...structuredClone(mockErrorBlockedConversationRun), projectId: row.projectId, taskId, taskUuid: row.taskUuid!, runId };
    },
    updateTaskMetadata: async (projectId, taskId, taskUuid, title) => {
      const index = rows.findIndex(row => row.projectId === projectId && row.taskId === taskId && row.taskUuid === taskUuid);
      if (index < 0) throw new Error('unknown task');
      return rows[index] = { ...rows[index], title };
    },
    deleteConversationTask: async (projectId, taskId, taskUuid) => {
      const index = rows.findIndex(row => row.projectId === projectId && row.taskId === taskId && row.taskUuid === taskUuid);
      if (index < 0) throw new Error('unknown task');
      rows.splice(index, 1);
      return bootstrap();
    },
  } satisfies Partial<RuntimeApi>;
  return methods;
}
