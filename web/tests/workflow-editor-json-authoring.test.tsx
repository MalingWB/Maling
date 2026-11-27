/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', async (importOriginal) => ({
  ...await importOriginal<typeof import('react-i18next')>(),
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@xyflow/react', async () => {
  const { Fragment } = await import('react');
  const PassThrough = ({ children }: { children?: React.ReactNode }) => <Fragment>{children}</Fragment>;
  return {
    Controls: () => null,
    ControlButton: () => null,
    Background: () => null,
    BaseEdge: () => null,
    EdgeLabelRenderer: PassThrough,
    Handle: () => null,
    MarkerType: { ArrowClosed: 'arrowclosed' },
    NodeToolbar: PassThrough,
    Panel: PassThrough,
    Position: { Bottom: 'bottom', Left: 'left', Right: 'right', Top: 'top' },
    // Exercise the editor's public canvas events without measuring SVG paths in jsdom.
    ReactFlow: ({ nodes, edges, onNodeClick, onEdgeClick, children }: import('@xyflow/react').ReactFlowProps) => <>
      {nodes?.map(node => <button key={node.id} data-node={node.id} aria-pressed={!!node.selected}
        onClick={event => onNodeClick?.(event, node)}>{String(node.data.label)}</button>)}
      {edges?.map(edge => <button key={edge.id} data-from={edge.source} data-to={edge.target}
        onClick={event => onEdgeClick?.(event, edge)}>{String(edge.label ?? '')}</button>)}
      {children}
    </>,
    getSmoothStepPath: () => ['', 0, 0],
    useUpdateNodeInternals: () => () => undefined,
  };
});

vi.mock('@/components/workflowLayoutEngine', async () => {
  const { default: ELK } = await import('elkjs/lib/elk.bundled.js');
  return { workflowLayoutEngine: async () => new ELK() };
});

vi.mock('@/components/ui/resizable', async () => {
  const { Fragment } = await import('react');
  const PassThrough = ({ children }: { children?: React.ReactNode }) => <Fragment>{children}</Fragment>;
  return {
    ResizableHandle: () => null,
    ResizablePanel: PassThrough,
    ResizablePanelGroup: PassThrough,
  };
});

import { WorkflowEditor, type WorkflowEditorSessionDraft } from '@/components/WorkflowEditor';
import { TooltipProvider } from '@/components/ui/tooltip';
import { readyWorkflowProfileCatalog } from '@/lib/workflow-profile-catalog';
import type { AgentRegistryVm, WorkflowDsl, WorkflowModelBindings } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const workflow: WorkflowDsl = {
  version: '0.1',
  id: 'json-authoring-workflow',
  entry: 'accept',
  control: {},
  nodes: [{
    type: 'worker',
    id: 'accept',
    executionSlotId: 'slot-accept',
    profile: 'pf-builtin-accept',
  }],
  edges: [{ from: 'accept', to: '$end', on: 'success' }],
};

const modelBindings: WorkflowModelBindings = {
  definitionRevision: 'definition-1',
  bindingRevision: 1,
  bindings: [{ executionSlotId: 'slot-accept', agentId: 'claude-acp' }],
};

const agentRegistry = {
  agents: [{
    agentType: 'claude-acp',
    displayName: 'Claude',
    diagnostic: { available: true },
    supportedModels: [],
    supportedModes: [],
    configOptions: [],
  }],
  catalog: [],
} as AgentRegistryVm;

let host: HTMLDivElement;
let root: Root;
let originalClientWidth: PropertyDescriptor | undefined;

beforeEach(() => {
  originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth');
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 1024 });
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  document.body.replaceChildren();
  if (originalClientWidth) Object.defineProperty(HTMLElement.prototype, 'clientWidth', originalClientWidth);
  else delete (HTMLElement.prototype as HTMLElement & { clientWidth?: number }).clientWidth;
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe('WorkflowEditor JSON authoring', () => {
  const renderEditor = async (value: WorkflowDsl) => act(async () => root.render(
    <TooltipProvider><WorkflowEditor value={value} modelBindings={modelBindings} agentRegistry={agentRegistry}
      profileCatalog={readyWorkflowProfileCatalog([])} onSave={() => undefined} /></TooltipProvider>,
  ));
  const button = (text: string) => Array.from(host.querySelectorAll('button')).find(b => b.textContent === text);
  const json = () => host.querySelector<HTMLTextAreaElement>('textarea')!;

  it('keeps both boundaries selectable but not deletable in a new, empty workflow', async () => {
    await renderEditor({ ...workflow, nodes: [], edges: [], entry: '' });
    for (const label of ['workflowEditor.nodeLabels.start', 'workflowEditor.nodeLabels.end']) {
      const boundary = button(label);
      expect(boundary, label).toBeDefined();
      await act(async () => boundary!.click());
      expect(boundary?.getAttribute('aria-pressed')).toBe('true');
      expect(button('workflowEditor.deleteNode')?.disabled).toBe(true);
      await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete' })));
      expect(button(label)?.getAttribute('aria-pressed')).toBe('true');
    }
    expect(host.querySelectorAll('[data-from]')).toHaveLength(0);
    await act(async () => activateTab(button('JSON')!));
    expect(JSON.parse(json().value)).toMatchObject({ nodes: [], edges: [], entry: '' });
  });

  it('projects the declared entry on load and JSON apply without serializing the start link or changing retry edges', async () => {
    const value: WorkflowDsl = { ...workflow, nodes: [...workflow.nodes, { ...workflow.nodes[0], id: 'review' }],
      entry: 'review', edges: [{ from: 'accept', to: 'review', on: 'success' },
        { from: 'review', to: 'accept', on: 'failure' }, { from: 'review', to: '$end', on: 'success' }] };
    await renderEditor(value);
    const start = button('workflowEditor.nodeLabels.start')?.dataset.node;
    expect(start).toBeTruthy();
    expect(start).not.toBe('$entry');
    const link = () => Array.from(host.querySelectorAll<HTMLElement>('[data-from]')).find(e => e.dataset.from === start);
    expect(link()?.dataset.to).toBe('review');
    await act(async () => button('workflowEditor.nodeLabels.end')!.click());
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace' })));
    await act(async () => activateTab(button('JSON')!));
    const saved = JSON.parse(json().value);
    expect(saved).toMatchObject({ entry: 'review', edges: value.edges });
    expect(saved.nodes.map((node: {id: string}) => node.id)).toEqual(['accept', 'review']);
    await act(async () => {
      setNativeTextareaValue(json(), JSON.stringify({ ...saved, entry: 'accept' }));
      json().dispatchEvent(new InputEvent('input', { bubbles: true }));
    });
    await act(async () => activateTab(button('workflowEditor.canvas')!));
    expect(link()?.dataset.to).toBe('accept');
    expect(host.querySelector('[data-from="review"][data-to="accept"]')).not.toBeNull();
  });

  it('drops a real edge selection when JSON replaces the workflow and keeps the disconnected End', async () => {
    await renderEditor(workflow);
    await act(async () => host.querySelector<HTMLButtonElement>('[data-from="accept"][data-to="$end"]')!.click());
    expect(button('workflowEditor.deleteEdge')?.disabled).toBe(false);
    const start = button('workflowEditor.nodeLabels.start')!;
    const entryLink = Array.from(host.querySelectorAll<HTMLButtonElement>('[data-from]')).find(e => e.dataset.from === start.dataset.node)!;
    await act(async () => entryLink.click());
    expect(start.getAttribute('aria-pressed')).toBe('true');
    expect(button('workflowEditor.deleteNode')?.disabled).toBe(true);
    await act(async () => host.querySelector<HTMLButtonElement>('[data-from="accept"][data-to="$end"]')!.click());
    await act(async () => activateTab(button('JSON')!));
    await act(async () => {
      setNativeTextareaValue(json(), JSON.stringify({ ...workflow, edges: [] }));
      json().dispatchEvent(new InputEvent('input', { bubbles: true }));
    });
    await act(async () => activateTab(button('workflowEditor.canvas')!));
    expect(button('workflowEditor.nodeLabels.end')).toBeDefined();
    expect(button('workflowEditor.deleteNode')?.disabled).toBe(true);
    expect(button('workflowEditor.deleteEdge')).toBeUndefined();
  });

  it('preserves a worker slot when pasted JSON is projected to the canvas', async () => {
    const onSessionDraftChange = vi.fn<(draft: WorkflowEditorSessionDraft) => void>();
    const initialSessionDraft: WorkflowEditorSessionDraft = {
      workflow,
      modelBindings,
      tab: 'json',
      jsonDraft: JSON.stringify(workflow, null, 2),
    };

    await act(async () => root.render(
      <TooltipProvider>
        <WorkflowEditor
          value={workflow}
          modelBindings={modelBindings}
          agentRegistry={agentRegistry}
          profileCatalog={readyWorkflowProfileCatalog([{ id: 'pf-builtin-accept', name: 'Accept' }])}
          initialSessionDraft={initialSessionDraft}
          onSessionDraftChange={onSessionDraftChange}
          onSave={() => undefined}
          showSaveAction={false}
        />
      </TooltipProvider>,
    ));

    const pastedWorkflow: WorkflowDsl = {
      ...workflow,
      nodes: workflow.nodes.map((node) => node.type === 'worker'
        ? { ...node, executionSlotId: undefined }
        : node),
    };
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea');
    expect(textarea).not.toBeNull();

    await act(async () => {
      setNativeTextareaValue(textarea!, JSON.stringify(pastedWorkflow, null, 2));
      textarea!.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }));
    });

    const canvasTab = Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tab"]'))
      .find((button) => button.textContent === 'workflowEditor.canvas');
    expect(canvasTab).toBeDefined();
    await act(async () => activateTab(canvasTab!));
    await act(async () => new Promise((resolve) => window.setTimeout(resolve, 380)));

    const latestDraft = onSessionDraftChange.mock.calls.at(-1)?.[0];
    const accept = latestDraft?.workflow.nodes.find((node) => node.id === 'accept');
    expect(accept).toMatchObject({ type: 'worker', executionSlotId: 'slot-accept' });
    expect(host.textContent).not.toContain('workflowEditor.validationSlotRequired');
  });
});

function setNativeTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  setter?.call(textarea, value);
}

function activateTab(tab: HTMLButtonElement) {
  tab.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
}
