/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n, { i18nInitialized } from '@/i18n';
import en from '@/locales/en.json';
import type { GraphNodeVm, GraphVm } from '@/types';
import type { WorkflowLayoutSpec } from '@/components/workflowGraph';

const flowProps = vi.hoisted(() => [] as Array<{
  nodes: Array<{ id: string; type: string; data: Record<string, unknown> }>;
  nodeTypes: Record<string, React.ComponentType<{ data: Record<string, unknown> }>>;
}>);
const layoutSpecs = vi.hoisted(() => [] as WorkflowLayoutSpec[]);

vi.mock('@xyflow/react', async () => {
  const ReactModule = await import('react');
  return {
    // 渲染节点组件本身：紧凑密度的断言落在真实渲染出的卡片上。
    ReactFlow: (props: unknown) => {
      flowProps.push(props as never);
      const { nodes, nodeTypes } = props as (typeof flowProps)[number];
      return ReactModule.createElement(
        'div',
        { 'data-testid': 'react-flow' },
        nodes.map((node) => ReactModule.createElement(nodeTypes[node.type], { key: node.id, id: node.id, data: node.data })),
      );
    },
    Background: () => null,
    Handle: ({ type, isConnectable }: { type: string; isConnectable?: boolean }) => <i data-handle={type} data-connectable={isConnectable} />,
    BaseEdge: () => null,
    EdgeLabelRenderer: ({ children }: { children: React.ReactNode }) => children,
    getSmoothStepPath: () => ['', 0, 0],
    MarkerType: { ArrowClosed: 'arrowclosed' },
    Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
  };
});

vi.mock('@/hooks/useWorkflowLayout', () => ({
  useWorkflowLayout: (spec: WorkflowLayoutSpec) => {
    layoutSpecs.push(spec);
    return {
      layout: {
        direction: 'DOWN',
        nodes: new Map(spec.nodes.map((node, index) => [node.id, { x: 0, y: index * 100, width: node.width, height: node.height }])),
        groups: new Map(),
        edges: new Map(),
        bounds: { x: 0, y: 0, width: 220, height: spec.nodes.length * 100 },
      },
      pending: false,
      failed: false,
    };
  },
}));

vi.mock('@/components/GraphControls', () => ({ GraphControls: () => null }));
vi.mock('@/components/GraphFullscreenDialog', () => ({ GraphFullscreenDialog: () => null }));
vi.mock('@/components/WorkflowLayoutStatus', () => ({ WorkflowLayoutStatus: () => null }));

import { GraphView } from '@/components/GraphView';
import { TooltipProvider } from '@/components/ui/tooltip';

const idle = { code: 'draft', tone: 'neutral', icon: 'dot', resumable: false, blockingError: false } as const;
const stage = (id: string, label: string, detail?: string): GraphNodeVm => ({
  id,
  nodeId: id,
  label,
  nodeType: 'worker',
  runtimeDisplay: { ...idle, terminal: false },
  artifactCount: 0,
  attachmentCount: 0,
  current: false,
  retryable: false,
  attemptCount: 0,
  attempts: [],
  ...(detail ? { detail } : {}),
});
const graph: GraphVm = {
  nodes: [
    stage('plan-1', '发布范围清单', '规划 · 自动重试'),
    stage('custom-1', '审计结论'),
    { ...stage('$end', '结束'), nodeId: null, nodeType: 'terminal', runtimeDisplay: { ...idle, terminal: true } },
  ],
  edges: [
    { from: 'plan-1', to: 'custom-1', label: 'success' },
    { from: 'custom-1', to: '$end', label: 'failure' },
    { from: 'custom-1', to: '$end', label: 'success' },
  ],
  groups: [],
};

/**
 * 图组件的节点卡片密度：默认是执行图的状态卡片；`compact` 给空间有限的嵌入面
 * （侧栏里的工作流蓝图）用两行卡片和端点标记，并让布局按同样尺寸排布。
 */
describe('GraphView node density', () => {
  let host: HTMLDivElement;
  let root: Root;

  const render = async (props: Record<string, unknown>) => {
    await act(async () => root.render(<TooltipProvider><GraphView graph={graph} variant="workflow" allowFullscreen={false} {...props} /></TooltipProvider>));
  };
  const latestSpec = () => layoutSpecs[layoutSpecs.length - 1];

  beforeEach(async () => {
    await i18nInitialized;
    if (!i18n.hasResourceBundle('en', 'translation')) {
      i18n.addResourceBundle('en', 'translation', en, true, true);
    }
    await i18n.changeLanguage('en');
    flowProps.length = 0;
    layoutSpecs.length = 0;
    vi.stubGlobal('ResizeObserver', class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it('keeps the runtime status card by default', async () => {
    await render({});

    expect(host.querySelector('[data-graph-density]')).toBeNull();
    expect(host.querySelector('.h-\\[138px\\]')).not.toBeNull();
    expect(latestSpec().nodes.find((node) => node.id === 'plan-1')).toMatchObject({ width: 220, height: 138 });
  });

  it('renders two-line cards and terminal markers in the compact density', async () => {
    await render({ density: 'compact', fitPadding: 0.05 });

    const cards = [...host.querySelectorAll('[data-graph-density="compact"]')];
    expect(cards).toHaveLength(3);
    const [first, second, end] = cards;
    // 名称 + 「角色 · 失败策略」；没有 detail 时回落到 `节点 id · 类型`。
    expect(first!.textContent).toContain('发布范围清单');
    expect(first!.textContent).toContain('规划 · 自动重试');
    expect(first!.className).toContain('h-[66px]');
    expect(first!.className).toContain('w-[220px]');
    expect(second!.textContent).toContain('custom-1 · worker');
    // 端点是被动标记，不是卡片。
    expect(end!.textContent).toBe(en.workflowEditor.nodeLabels.end);
    expect(end!.querySelector('[data-handle="source"]')).toBeNull();
    expect(end!.querySelector('[data-handle="target"][data-connectable="false"]')).not.toBeNull();
    expect(end!.className).toContain('h-[44px]');
    expect((end as HTMLElement).style.width).toBe('140px');
    expect(host.querySelector('.h-\\[138px\\]')).toBeNull();
    // 边照常投影（失败边与成功边都保留）。
    expect(flowProps[0]!.nodes).toHaveLength(3);
  });

  it('keeps the compact blueprint caption in its own row above the measured canvas', async () => {
    await render({ density: 'compact' });
    const canvas = host.querySelector('[data-testid="react-flow"]')!.parentElement!;
    expect(canvas.previousElementSibling?.textContent).toBe(en.graph.workflowBlueprint);
    expect(canvas.previousElementSibling?.classList.contains('absolute')).toBe(false);
  });

  it('lays the compact cards out at their rendered size', async () => {
    await render({ density: 'compact' });

    // 布局尺寸必须与卡片一致，否则连线端口与卡片错位。
    expect(latestSpec().nodes.find((node) => node.id === 'plan-1')).toMatchObject({ width: 220, height: 66 });
    expect(latestSpec().nodes.find((node) => node.id === '$end')).toMatchObject({ width: 140, height: 44 });
  });
});
