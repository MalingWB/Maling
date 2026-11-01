import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  NODE_WIDTH,
  autoWorkflowLayoutDirection,
  buildElkGraph,
  entryAnchoredViewport,
  workflowHandlePositions,
  type WorkflowLayoutSpec,
} from '@/components/workflowGraph';
import { resolveWorkflowLayoutDirection } from '@/components/workflow-direction-preference';

function spec(direction?: 'RIGHT' | 'DOWN'): WorkflowLayoutSpec {
  return {
    ...(direction ? { direction } : {}),
    nodes: [
      { id: 'dev', width: NODE_WIDTH, height: 66, sourceHandles: 'split' },
      { id: '$end', width: 140, height: 44, sourceHandles: 'none' },
      { id: '$new-round', width: 140, height: 44, sourceHandles: 'none', pinBelow: 'dev' },
    ],
    edges: [
      { id: 'e1', from: 'dev', to: '$end', outcome: 'success' },
      { id: 'e2', from: 'dev', to: '$new-round', outcome: 'failure' },
    ],
    groups: [],
  };
}

describe('workflow graph layout direction', () => {
  it('picks the initial direction from the container aspect ratio', () => {
    expect(autoWorkflowLayoutDirection(800, 600)).toBe('RIGHT');
    expect(autoWorkflowLayoutDirection(600, 800)).toBe('DOWN');
    expect(autoWorkflowLayoutDirection(0, 800)).toBe('RIGHT');
  });

  it('applies hysteresis so dragging across the 1:1 ratio does not thrash layouts', () => {
    // Already horizontal: stays horizontal until the ratio clearly exceeds 1.1.
    expect(autoWorkflowLayoutDirection(100, 105, 'RIGHT')).toBe('RIGHT');
    expect(autoWorkflowLayoutDirection(100, 115, 'RIGHT')).toBe('DOWN');
    // Already vertical: stays vertical until the ratio clearly drops below 0.9.
    expect(autoWorkflowLayoutDirection(100, 95, 'DOWN')).toBe('DOWN');
    expect(autoWorkflowLayoutDirection(100, 85, 'DOWN')).toBe('RIGHT');
  });

  it('maps the user preference onto the effective direction', () => {
    expect(resolveWorkflowLayoutDirection('horizontal', 600, 800)).toBe('RIGHT');
    expect(resolveWorkflowLayoutDirection('vertical', 800, 600)).toBe('DOWN');
    expect(resolveWorkflowLayoutDirection('auto', 800, 600)).toBe('RIGHT');
    expect(resolveWorkflowLayoutDirection('auto', 600, 800)).toBe('DOWN');
  });

  it('mirrors React Flow handle positions per direction', () => {
    expect(workflowHandlePositions('RIGHT')).toEqual({ source: 'right', target: 'left' });
    expect(workflowHandlePositions('DOWN')).toEqual({ source: 'bottom', target: 'top' });
  });

  it('defaults omitted directions to the horizontal flow', () => {
    const graph = buildElkGraph(spec());
    expect(graph.layoutOptions?.['elk.direction']).toBe('RIGHT');
  });

  it('builds a vertical ELK graph with mirrored ports and pinned position', () => {
    const graph = buildElkGraph(spec('DOWN'));
    expect(graph.layoutOptions?.['elk.direction']).toBe('DOWN');

    const dev = graph.children!.find((node) => node.id === 'dev')!;
    const end = graph.children!.find((node) => node.id === '$end')!;
    const newRound = graph.children!.find((node) => node.id === '$new-round')!;

    const portSides = (node: typeof dev) => node.ports!.map((port) => port.layoutOptions?.['elk.port.side']);
    expect(portSides(dev)).toEqual(['NORTH', 'SOUTH', 'SOUTH']);
    expect(portSides(end)).toEqual(['NORTH']);
    // The pinned new-round target accepts backward edges on its downstream side.
    expect(portSides(newRound)).toEqual(['SOUTH']);

    // Split success/failure source ports split along the width axis in DOWN flow.
    const devSuccess = dev.ports!.find((port) => port.id === 'dev::success')!;
    const devFailure = dev.ports!.find((port) => port.id === 'dev::failure')!;
    expect(devSuccess.x).toBe(NODE_WIDTH * 0.34);
    expect(devFailure.x).toBe(NODE_WIDTH * 0.66);
    expect(devSuccess.y).toBe(66);
    expect(devFailure.y).toBe(66);

    // The pinned node is placed far right in the first (top) layer.
    expect(newRound.layoutOptions?.['elk.position']).toBe('(100000,0)');
    expect(newRound.layoutOptions?.['elk.layered.layering.layerConstraint']).toBe('FIRST');
  });

  it('keeps the entry side visible when the graph cannot fit', () => {
    const bounds = { x: 0, y: 0, width: 2000, height: 600 };
    const viewport = { width: 400, height: 800 };
    const horizontal = entryAnchoredViewport(bounds, viewport, { padding: 0.2, minZoom: 0.3, maxZoom: 0.92, direction: 'RIGHT' });
    expect(horizontal.x).toBeGreaterThan(-0.001);
    const vertical = entryAnchoredViewport(bounds, viewport, { padding: 0.2, minZoom: 0.3, maxZoom: 0.92, direction: 'DOWN' });
    expect(vertical.y).toBeGreaterThan(-0.001);
    // In the vertical flow the horizontal axis stays centered.
    expect(vertical.x).toBeLessThan(horizontal.x);
  });
});

describe('workflow direction preference persistence', () => {
  it('wires the preference into the personalization schema through the app shell', () => {
    const app = readFileSync(path.resolve(__dirname, '../src/App.tsx'), 'utf8');
    expect(app).toContain('WorkflowGraphDirectionProvider');
    expect(app).toContain('workflowGraphDirection: next');

    const types = readFileSync(path.resolve(__dirname, '../src/types.ts'), 'utf8');
    expect(types).toContain("schemaVersion: 5;");
    expect(types).toContain('interfaceZoomPercent: number;');
    expect(types).toContain("workflowGraphDirection: WorkflowGraphDirectionPreference;");
  });
});
