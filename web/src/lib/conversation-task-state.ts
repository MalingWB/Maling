import type { ConversationSidebarVm, ConversationTaskRowVm } from '@/types';
import { CONVERSATION_SIDEBAR_TASK_WINDOW, promoteConversationLocatedTask } from '@/lib/conversation-sidebar-loading';
import { parseTimestamp } from '@/lib/datetime';

function sameTask(
  task: Pick<ConversationTaskRowVm, 'projectId' | 'taskId'>,
  projectId: string,
  taskId: string,
) {
  return task.projectId === projectId && task.taskId === taskId;
}

export function findConversationTask(
  sidebar: ConversationSidebarVm,
  projectId: string,
  taskId: string,
) {
  return sidebar.tasksByWorkspace[projectId]?.find((task) => sameTask(task, projectId, taskId))
    ?? sidebar.pinnedTasks.find((task) => sameTask(task, projectId, taskId))
    ?? null;
}

export function applyConversationTaskSnapshot(
  sidebar: ConversationSidebarVm,
  task: ConversationTaskRowVm,
): ConversationSidebarVm {
  const workspaceTasks = sidebar.tasksByWorkspace[task.projectId] ?? [];
  const taskIndex = workspaceTasks.findIndex((candidate) => sameTask(candidate, task.projectId, task.taskId));
  const existing = taskIndex >= 0
    ? workspaceTasks[taskIndex]
    : sidebar.pinnedTasks.find((candidate) => sameTask(candidate, task.projectId, task.taskId));
  const mergedTask = existing && task.runHistoryStatus === 'not-loaded'
    ? {
        ...task,
        runs: existing.runs,
        runHistoryStatus: existing.runHistoryStatus,
        runsNextCursor: existing.runsNextCursor,
      }
    : task;
  const nextWorkspaceTasks = taskIndex >= 0
    ? workspaceTasks.map((candidate, index) => index === taskIndex ? mergedTask : candidate)
    : [mergedTask, ...workspaceTasks];
  const located = sidebar.locatedTask;
  let ordinaryCount = 0;
  const boundedWorkspaceTasks = nextWorkspaceTasks.filter((candidate) => {
    if (located && sameTask(candidate, located.projectId, located.taskId)
      && (!located.taskUuid || candidate.taskUuid === located.taskUuid)) return true;
    return ordinaryCount++ < CONVERSATION_SIDEBAR_TASK_WINDOW;
  });

  const remainingPinnedTasks = sidebar.pinnedTasks
    .filter((candidate) => !sameTask(candidate, task.projectId, task.taskId));
  const nextPinnedTasks = mergedTask.pinned
    ? [...remainingPinnedTasks, mergedTask].sort((left, right) =>
        (left.pinnedOrder ?? Number.MAX_SAFE_INTEGER) - (right.pinnedOrder ?? Number.MAX_SAFE_INTEGER))
    : remainingPinnedTasks;

  const next = {
    ...sidebar,
    pinnedTasks: nextPinnedTasks,
    tasksByWorkspace: {
      ...sidebar.tasksByWorkspace,
      [task.projectId]: boundedWorkspaceTasks,
    },
  };
  const activityAdvanced = existing
    && (parseTimestamp(task.lastActivityAt)?.getTime() ?? Number.NEGATIVE_INFINITY)
      > (parseTimestamp(existing.lastActivityAt)?.getTime() ?? Number.NEGATIVE_INFINITY);
  return activityAdvanced ? promoteConversationLocatedTask(next, mergedTask) : next;
}
