import { useEffect, useLayoutEffect, useState } from 'react';
import { getConversationTaskSummary } from '@/api';
import type { ConversationSidebarVm, ConversationTaskRowVm } from '@/types';
import { findConversationTask } from '@/lib/conversation-task-state';
import type { RefreshSingleFlight } from '@/lib/refresh-single-flight';
import { useStableCallbacks } from './use-stable-callbacks';

type Locator = Pick<ConversationTaskRowVm, 'projectId' | 'taskId' | 'taskUuid'>;
type Header = Locator & Pick<ConversationTaskRowVm, 'title' | 'runMode' | 'resident'>;
function matches(a: Locator | null, b: Locator | null): boolean {
  return Boolean(a && b && a.projectId === b.projectId && a.taskId === b.taskId && a.taskUuid === b.taskUuid);
}

/** An open run outlives any list page. Keep only its displayed metadata, never its history. */
export function usePresentedTaskHeader(run: Locator | null, sidebar: ConversationSidebarVm, flights: RefreshSingleFlight) {
  const [retained, setRetained] = useState<Header | null>(null);
  const { projectId, taskId, taskUuid } = run ?? {};
  const row = projectId && taskId ? findConversationTask(sidebar, projectId, taskId, taskUuid) : null;
  const { observe } = useStableCallbacks({ observe: (task: Header, onlyIfMissing = false) => {
    if (!matches(run, task)) return;
    const { projectId, taskId, taskUuid, title, runMode, resident } = task;
    setRetained(prev => matches(prev, task) && (onlyIfMissing || (prev?.title === title && prev.runMode === runMode && prev.resident === resident))
      ? prev : { projectId, taskId, taskUuid, title, runMode, resident });
  }});
  useLayoutEffect(() => {
    if (row) observe(row);
    else if (!projectId) setRetained(null);
  }, [row, observe, projectId]);
  const header = row ?? (matches(run, retained) ? retained : null);
  const missing = !header;
  useEffect(() => {
    if (!missing || !projectId || !taskId) return;
    let active = true;
    void flights.run(`summary:${projectId}:${taskUuid ?? taskId}`, () => getConversationTaskSummary(projectId, taskId, taskUuid))
      .then(task => { if (active) observe(task, true); }).catch(() => {});
    return () => { active = false; };
  }, [projectId, taskId, taskUuid, missing, flights, observe]);
  return { header, observe };
}
