/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { RunModeManagementPage } from '@/pages/RunModeManagementPage';
import { browserApi } from '@/api/browser';
import { workflowTemplateDisplayName } from '@/lib/workflow-template';
import i18n from '@/i18n';

vi.mock('@/components/WorkflowEditor', async (load) => ({ ...await load<typeof import('@/components/WorkflowEditor')>(), WorkflowEditor: () => null }));
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

it('starts a copy of a built-in workflow from the picker without enabling deletion', async () => {
  const store = await browserApi.getWorkflowTemplates();
  const source = store.templates.find(template => template.isBuiltIn)!;
  const name = workflowTemplateDisplayName(source, i18n.t.bind(i18n));
  const modify = vi.fn();
  const host = document.body.appendChild(document.createElement('div'));
  const root = createRoot(host);
  try {
    await act(async () => root.render(<TooltipProvider><RunModeManagementPage projectId="workspace" workspaceName="Workspace" workspaces={[]}
      runMode={{ mode: 'workflow', workflowTemplateId: source.id }} agentRegistry={null} workflowTemplates={store}
      onProjectChange={() => {}} onSave={() => {}} onForgeTemplate={modify} /></TooltipProvider>));
    const button = (label: string) => [...document.querySelectorAll('button')].find(b => b.getAttribute('aria-label') === label);
    await act(async () => button(i18n.t('taskList.create.workflowTemplate'))!.click());
    expect(button(i18n.t('taskList.create.defaultWorkflowReadonly'))?.disabled).toBe(true);
    const action = button(i18n.t('runMode.forgeModifyTemplate', { name }));
    expect(action, 'built-in templates should offer conversational editing as a copy').toBeDefined();
    await act(async () => action!.click());
    expect(modify).toHaveBeenCalledWith(source.id, name, true);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
