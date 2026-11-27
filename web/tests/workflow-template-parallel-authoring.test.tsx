/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
import { RunModeManagementPage } from '@/pages/RunModeManagementPage';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { WorkflowDsl, WorkflowTemplateStore } from '@/types';
import { saveWorkflowTemplate } from '@/api';

vi.mock('react-i18next', async original => {
  const t = (k: string, options?: { id?: string }) => options?.id ? `${k} ${options.id}` : k;
  return {...await original<typeof import('react-i18next')>(), useTranslation: () => ({t})};
});
vi.mock('@/api', async original => ({...await original<typeof import('@/api')>(),
  getAutoTemplates: async () => ({templates:[]}), getProfiles: async () => ({profiles:[{id:'role', name:'Role'}]}), saveWorkflowTemplate: vi.fn(),
}));
vi.mock('@/components/ui/resizable', async () => {
  const pass = ({children}: {children?: React.ReactNode}) => <>{children}</>;
  return {ResizablePanelGroup:pass, ResizablePanel:pass, ResizableHandle:() => null};
});
vi.mock('@xyflow/react', async original => ({...await original<typeof import('@xyflow/react')>(),
  ReactFlow: ({nodes, onNodeClick, children}: import('@xyflow/react').ReactFlowProps) => <>{nodes?.map(n => <button key={n.id} data-node={n.id} onClick={e => onNodeClick?.(e, n)}>{String(n.data.label)}</button>)}{children}</>,
  Panel: ({children}: {children:React.ReactNode}) => <>{children}</>, Background: () => null, Controls:() => null,
}));
// The public save path does not need pixel measurements; the projection test runs real ELK.
vi.mock('@/hooks/useWorkflowLayout', () => ({useWorkflowLayout: () => ({layout:null, status:'ready'})}));

const workflow: WorkflowDsl = {id:'plain', version:'0.1', entry:'report', control:{}, nodes:[{id:'report', type:'worker', executionSlotId:'slot-report',profile:'role'}], edges:[{from:'report',on:'success',to:'$end'}]};
const template = {id:'fixture', name:'Fixture', isBuiltIn:false, workflow, modelBindings:{definitionRevision:'definition',bindingRevision:0, bindings:[]}, createdAt:'',updatedAt:''};
const store: WorkflowTemplateStore = {version:'0.1', templates:[template]};

afterEach(() => {vi.restoreAllMocks(); vi.unstubAllGlobals();});

it('edits a fork in the public template page, rejects invalid visible JSON, saves and reopens ordered slots', async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('ResizeObserver', class {observe() {} unobserve() {} disconnect() {}});
  const orig = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth');
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {configurable:true,get:() => 1200});
  const host = document.createElement('div'); document.body.append(host);
  const root = createRoot(host);
  const open = async (templates: WorkflowTemplateStore, id: string) => act(async () => root.render(<TooltipProvider><RunModeManagementPage key={id} projectId="project" workspaceName="Fixture" workspaces={[]} runMode={{mode:'workflow',workflowTemplateId:id}} agentRegistry={null} workflowTemplates={templates} onProjectChange={() => {}} onSave={() => {}} /></TooltipProvider>));
  const button = (text: string) => Array.from(host.querySelectorAll('button')).find(b => b.textContent === text)!;
  const click = (el: HTMLElement) => act(async () => el.click());
  const tab = (label: string) => act(async () => button(label).dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',bubbles:true})));
  const text = (el: HTMLInputElement | HTMLTextAreaElement, value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value')!.set!.call(el,value);
    el.dispatchEvent(new Event('input',{bubbles:true}));
  });
  const json = () => host.querySelector('textarea')!;
  try {
    await open(store, 'fixture');
    await click(button('workflowEditor.nodeLabels.start'));
    await click(button('workflowEditor.parallel.insert'));
    await click(button('workflowEditor.parallel.addStep'));
    await click(host.querySelector('[aria-label="workflowEditor.parallel.up parallel-branch-1-2"]')!);
    await click(button('workflowEditor.parallel.addBranch'));
    await click(host.querySelector('[aria-label="workflowEditor.parallel.up branch-3"]')!);
    await click(host.querySelector('[aria-label="workflowEditor.parallel.remove branch-3"]')!);
    await tab('JSON');
    const edited = JSON.parse(json().value) as WorkflowDsl;
    const group = edited.nodes.find(n => n.type === 'parallel-group')!;
    expect(group).toMatchObject({id:edited.entry, maxParallel:3, branches:[{id:'branch-1',stages:['parallel-branch-1-2','parallel-branch-1']},{id:'branch-2',stages:['parallel-branch-2']}]});
    await text(json(), JSON.stringify({...edited, nodes:edited.nodes.map(n => n === group ? {...group, maxParallel:1} : n)}));
    await tab('workflowEditor.canvas');
    await click(host.querySelector('[aria-label="workflowEditor.undo"]')!);
    await tab('JSON');
    expect(JSON.parse(json().value).nodes.find((n: {type:string}) => n.type === 'parallel-group')).toEqual(group);
    for (const n of edited.nodes) if (n.type === 'worker') n.profile = 'role';
    group.maxParallel = 5;
    await text(host.querySelector('input[placeholder="taskList.create.workflowTemplateName"]')!, 'Saved');
    await text(json(),JSON.stringify({...edited,nodes:[{...group,branches:null}, ...edited.nodes.filter(n => n !== group)]}));
    await click(button('taskList.create.saveAsWorkflow'));
    expect(saveWorkflowTemplate).not.toHaveBeenCalled();
    expect(host.textContent).toContain('workflowEditor.outputSchemaInvalid');
    let saved: WorkflowTemplateStore | undefined;
    vi.mocked(saveWorkflowTemplate).mockImplementation(async (name, definition, modelBindings) => {
      saved = {...store, templates:[{...template,id:'saved-id',name,workflow:structuredClone(definition),modelBindings:modelBindings!}]};
      return saved;
    });
    await text(json(), JSON.stringify(edited));
    await click(button('taskList.create.saveAsWorkflow'));
    expect(saveWorkflowTemplate).toHaveBeenCalledOnce();
    await open(saved!, 'saved-id');
    await tab('JSON');
    const reopened = JSON.parse(json().value);
    expect(reopened.nodes).toEqual(edited.nodes);
    expect(reopened.edges).toEqual([{from:'report',on:'success',to:'$end'},{from:'parallel',to:'report',on:'success'}]);
  } finally {
    await act(async () => root.unmount());host.remove();
    if (orig) Object.defineProperty(HTMLElement.prototype,'clientWidth',orig); else Reflect.deleteProperty(HTMLElement.prototype,'clientWidth');
  }
}, 15_000);
