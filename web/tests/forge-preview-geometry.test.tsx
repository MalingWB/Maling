/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { Edge, Node } from '@xyflow/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ForgePreviewPanel } from '@/components/workspace/ForgePreviewPanel';
import {
  estimateLabelWidth,
  WORKFLOW_EDGE_LABEL_HEIGHT,
  type Rect,
  type WorkflowLayoutRoute,
} from '@/components/workflowGraph';
import type { ForgeSpecVm } from '@/types';

// Only substitute the IPC, worker transport and final paint surface. The panel, its
// direction switch, GraphView projection, layout hook and ELK layered all run.
const { getSpec } = vi.hoisted(() => ({ getSpec: vi.fn() }));
vi.mock('@/api', () => ({
  forgeGetSpec: getSpec,
  subscribeForgeTurnSettled: async () => () => {},
  forgeCompile: vi.fn(),
  forgeSetStageFailure: vi.fn(),
  forgeStartTrial: vi.fn(),
}));
vi.mock('@/components/workflowLayoutEngine', async () => {
  const { default: ELK } = await import('elkjs/lib/elk.bundled.js');
  const engine = new ELK();
  return { workflowLayoutEngine: async () => engine };
});
let frame: { nodes: Node[]; edges: Edge<{ route?: WorkflowLayoutRoute; label?: string }>[] };
vi.mock('@xyflow/react', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@xyflow/react')>()),
  ReactFlow: (props: typeof frame & { children: React.ReactNode }) => {
    frame = props;
    return props.children;
  },
  Background: () => null,
}));
vi.mock('@/components/GraphControls', () => ({
  GraphControls: ({ direction }: { direction: { onChange: (next: string) => void } }) => (
    <button data-horizontal onClick={() => direction.onChange('horizontal')} />
  ),
}));

const fixture = (onFailure: 'end' | 'retry'): ForgeSpecVm => ({
  specVersion: '0.1',
  intent: 'Delivery checks',
  workflows: [
    {
      id: 'delivery',
      name: 'Delivery checks',
      stages: ['api', 'ui', 'report'].map((id) => ({
        id,
        role: 'test',
        artifact: 'implementation-result',
        goal: id,
        outputKind: 'json',
        onFailure: 'end',
        manualCheck: false,
      })),
      parallelGroups: [
        {
          id: 'checks',
          maxParallel: 2,
          branches: [
            { id: 'api-check', stages: ['api'] },
            { id: 'ui-check', stages: ['ui'] },
          ],
          onFailure,
        },
      ],
    },
  ],
});
const start = '$forge:delivery:start',
  fork = '$forge:delivery:fork:checks',
  join = '$forge:delivery:join:checks';
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const host = document.createElement('div');
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  document.body.append(host);
  root = createRoot(host);
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
async function render(direction: 'RIGHT' | 'DOWN', onFailure: 'end' | 'retry') {
  getSpec.mockResolvedValue(fixture(onFailure));
  await act(async () =>
    root.render(
      <ForgePreviewPanel
        resource={{
          kind: 'forge-preview',
          key: 'fixture',
          scopeKey: 'fixture',
          title: 'Fixture',
          attention: false,
          projectId: 'p',
          taskId: 't',
        }}
      />,
    ),
  );
  if (direction === 'RIGHT') await act(async () => host.querySelector<HTMLButtonElement>('[data-horizontal]')!.click());
  await vi.waitFor(async () => {
    await act(async () => {});
    expect(frame.nodes[0]?.hidden).toBe(false);
    expect(frame.nodes[0]?.data.direction).toBe(direction);
  });
}

it.each(['DOWN', 'RIGHT'] as const)(
  'lays every edge out and separates fork → siblings → join → report in %s',
  async (direction) => {
    await render(direction, 'end');
    const axis = direction === 'DOWN' ? 'y' : 'x';
    const rank = (id: string) => frame.nodes.find((n) => n.id === id)!.position[axis];
    const layers = [[start], [fork], ['api', 'ui'], [join], ['report'], ['$end']];
    const ranks = layers.map((ids) => ids.map(rank));
    expect(ranks[2][0]).toBe(ranks[2][1]);
    for (let i = 1; i < ranks.length; i++) expect(ranks[i][0], JSON.stringify(ranks)).toBeGreaterThan(ranks[i - 1][0]);
    expect(frame.edges).toHaveLength(9);
    expect(frame.edges.filter((e) => !e.data?.route).map((e) => [e.source, e.target])).toEqual([]);
    expect(
      frame.edges.filter((e) => e.style?.stroke === 'var(--destructive)').map((e) => [e.source, e.target]),
    ).toEqual([
      [join, '$end'],
      ['report', '$end'],
    ]);
  },
);

it('uses the existing canvas height budget for all the controls in the expanded blueprint', async () => {
  await render('DOWN', 'end');
  const bounds = frame.nodes.map((n) => {
    const layoutNode = n.data.layoutNode as { height: number };
    return { y: n.position.y, bottom: n.position.y + layoutNode.height };
  });
  const contentHeight = Math.max(...bounds.map((n) => n.bottom)) - Math.min(...bounds.map((n) => n.y));
  const height = parseFloat(host.querySelector<HTMLElement>('[data-forge-spec-graph]')!.style.height);
  // A seven-node split must not be scaled as if it were three workers and a single endpoint.
  expect(height).toBeGreaterThanOrEqual(Math.min(560, contentHeight * 0.8));
});

it('keeps the success topology in model order when the join retries the fork', async () => {
  await render('DOWN', 'retry');
  const y = (id: string) => frame.nodes.find((n) => n.id === id)!.position.y;
  expect([y(fork), y('api'), y(join), y('report')]).toEqual(
    [y(fork), y('api'), y(join), y('report')].sort((a, b) => a - b),
  );
  expect(frame.edges.find((e) => e.source === join && e.target === fork)?.data?.route).toBeDefined();
});

it.each(['DOWN', 'RIGHT'] as const)(
  'keeps visible label footprints off other labels and cards in %s',
  async (direction) => {
    await render(direction, 'end');
    const overlap = (a: Rect, b: Rect) =>
      a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    const boxes = frame.nodes.map((n) => ({
      id: n.id,
      ...n.position,
      ...(n.data.layoutNode as Pick<Rect, 'width' | 'height'>),
    }));
    expect(frame.edges.filter((e) => e.data?.label)).toHaveLength(4);
    for (const e of frame.edges) {
      const route = e.data?.route,
        label = e.data?.label;
      if (!label || !route) continue;
      const width = estimateLabelWidth(label, 11, 18),
        height = WORKFLOW_EDGE_LABEL_HEIGHT;
      const box = { id: e.id, x: route.labelX - width / 2, y: route.labelY - height / 2, width, height };
      expect(
        boxes.filter((b) => overlap(box, b)),
        JSON.stringify(box),
      ).toEqual([]);
      boxes.push(box);
    }
  },
);
