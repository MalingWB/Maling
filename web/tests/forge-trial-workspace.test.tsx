/** @vitest-environment jsdom */
import { act, type ComponentProps, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '@/App';
import { browserApi } from '@/api/browser';
import { desktopApi } from '@/api/desktop';
import { ForgePreviewPanel } from '@/components/workspace/ForgePreviewPanel';
import type { ConversationRunPage } from '@/pages/ConversationRunPage';
import { mockErrorBlockedConversationRun } from '@/mockData';

const ipc = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', async (original) => ({ ...await original<typeof import('@tauri-apps/api/core')>(), invoke: ipc }));
vi.mock('@/components/Shell', () => ({ Shell: ({ children }: { children: ReactNode }) => <>{children}</> }));
vi.mock('@/components/acp/ACPChatDialog', () => ({ configureAcpResourceCacheSessionCount: vi.fn() }));
vi.mock('@/components/GraphView', () => ({ GraphView: () => null }));
vi.mock('@/lib/use-intervention-notifications', () => ({ useInterventionNotifications: vi.fn() }));
vi.mock('@/lib/use-scheduled-notifications', () => ({ useScheduledNotifications: vi.fn() }));
vi.mock('@/components/scheduled-tasks/useScheduledRuntimeSettings', () => ({ prefetchScheduledRuntimeSettings: vi.fn() }));
vi.mock('@/components/settings/useImSettings', () => ({ prefetchImSettings: vi.fn() }));
// Mount the real trial button inside App's actual page/start/navigation boundary.
vi.mock('@/pages/ConversationRunPage', () => ({ ConversationRunPage: ({ run, onForgeTrialSessionStarted }: ComponentProps<typeof ConversationRunPage>) => run.taskId === 'forge-task'
  ? <ForgePreviewPanel resource={{ kind: 'forge-preview', key: 'preview', scopeKey: 'scope', title: '', attention: false, projectId: run.projectId, taskId: run.taskId }} onTrialSessionStarted={onForgeTrialSessionStarted} />
  : <output>{`${run.projectId}/${run.taskId}/${run.runId}`}</output> }));

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it.each([true, false])('opens a trial only after start_run succeeds (startup success: %s)', async (canStart) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener() {}, removeEventListener() {} })));
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const owner = 'project-owner', other = 'project-default', taskId = 'task-shared';
  const tasks = new Map([[other, new Map([[taskId, [] as string[]]])], [owner, new Map<string, string[]>()]]);
  ipc.mockImplementation(async (name, args) => {
    const pid = args.projectId ?? other;
    const project = tasks.get(pid)!;
    if (name === 'forge_start_trial') {
      project.set(taskId, []);
      return { projectId: pid, taskId, title: 'Trial' };
    }
    if (name === 'start_run') {
      if (!canStart) throw new Error('synthetic-start-failure');
      const id = `run-${pid}`;
      project.get(args.taskId)!.push(id);
      return { id, taskId: args.taskId };
    }
    throw new Error(`Unexpected command ${name}`);
  });
  // Keep the public API and both desktop IPC encoders; no real runtime or user state.
  vi.spyOn(browserApi, 'forgeStartTrial').mockImplementation(desktopApi.forgeStartTrial);
  vi.spyOn(browserApi, 'startRun').mockImplementation(desktopApi.startRun);
  vi.spyOn(browserApi, 'getConversationSidebarBootstrap').mockResolvedValue({
    workspaces: [other, owner].map(projectId => ({ projectId, workspacePath: `C:/${projectId}`, name: projectId })),
    preferences: {}, lastActiveWorkspaceId: other, pinRefs: [],
  });
  vi.spyOn(browserApi, 'getConversationRun').mockImplementation(async (projectId, taskId, runId) => ({
    ...structuredClone(mockErrorBlockedConversationRun), projectId: projectId!, taskId, runId,
  }));
  vi.spyOn(browserApi, 'forgeGetSpec').mockResolvedValue({
    specVersion: '0.1', name: 'Trial', intent: 'sample requirement', sourceSkills: [], openQuestions: [],
    workflows: [{ name: 'Trial', stages: [{ id: 'first', role: 'dev', goal: 'check', outputKind: 'none', onFailure: 'end', manualCheck: false }] }],
  });
  vi.spyOn(browserApi, 'forgeCompile').mockResolvedValue({ status: 'saved', trial: { templateId: 'workflow', revision: 'review-A', group: null }, templateIds: ['workflow'], createdCount: 1, updatedCount: 0,
    templates: { version: '0.1', lastUsedTemplateId: 'workflow', lastCreatedWorkflow: null, templates: [] } });
  const host = document.createElement('div');
  document.body.append(host);
  const root = createRoot(host);
  try {
    window.history.replaceState(null, '', `/chat/projects/${owner}/tasks/forge-task/runs/source`);
    await act(async () => root.render(<App />));
    await act(async () => host.querySelector<HTMLButtonElement>('[data-forge-save]')!.click());
    await act(async () => host.querySelector<HTMLButtonElement>('[data-forge-saved] button')!.click());
    expect(ipc).toHaveBeenCalledWith('forge_start_trial', { projectId: owner, templateId: 'workflow', revision: 'review-A', intent: 'sample requirement' });
    expect(tasks.get(other)!.get(taskId)).toEqual([]);
    if (!canStart) {
      expect(tasks.get(owner)!.get(taskId)).toEqual([]);
      expect(window.location.pathname).toBe(`/chat/projects/${owner}/tasks/forge-task/runs/source`);
      expect(document.body.textContent).toContain('synthetic-start-failure');
      return;
    }
    expect(tasks.get(owner)!.get(taskId)).toEqual([`run-${owner}`]);
    expect(window.location.pathname).toBe(`/chat/projects/${owner}/tasks/${taskId}/runs/run-${owner}`);
    expect(host.querySelector('output')?.textContent).toBe(`${owner}/${taskId}/run-${owner}`);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
