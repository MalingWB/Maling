import { describe, expect, it } from 'vitest';
import ELK from 'elkjs/lib/elk.bundled.js';
import { layoutWorkflowGraph } from '@/components/workflowGraph';
import { authoringWorkflowTopologySignature, createAuthoringFlowProjection, createAuthoringGraph, normalizeWorkflowJsonForAuthoring, parseWorkflowJson, validateWorkflowForSave } from '@/components/WorkflowEditor';
import { readyWorkflowProfileCatalog } from '@/lib/workflow-profile-catalog';
import type { WorkflowDsl } from '@/types';

const group = { type: 'parallel-group' as const, id: 'fork', maxParallel: 3, branches: [{ id: 'api', stages: ['design', 'implement'] }, { id: 'ui', stages: ['view'] }] };
const definition: WorkflowDsl = { version: '0.1', id: 'editor-parallel', entry: 'fork', control: {},
  nodes: [group, ...['design', 'implement', 'view', 'report'].map(id => ({ type: 'worker' as const, id, profile: 'role', executionSlotId: `slot-${id}` }))],
  edges: [{ from: 'fork', to: 'report', on: 'success' }, { from: 'report', to: '$end', on: 'success' }] };
const validate = (dsl: WorkflowDsl) => validateWorkflowForSave(dsl, readyWorkflowProfileCatalog([{id: 'role', name: 'Role'}]), [], k => k, null, null, null, false, undefined, false);

describe('public editor contract for explicit parallel workflows', () => {
  it('validates, saves and reopens an entry fork without requiring a group slot or fake worker edges', () => {
    const {issues, sanitizedWorkflow} = validate(definition);
    expect(issues).toEqual([]);
    const reopened = normalizeWorkflowJsonForAuthoring(parseWorkflowJson(JSON.stringify(sanitizedWorkflow))!, definition);
    expect(reopened).toEqual(definition);
    const graph = createAuthoringGraph(reopened);
    const flow = createAuthoringFlowProjection(reopened, graph, null, null, null, new Set(), new Map(), k => k);
    const join = flow.nodes.find(n => n.data.label === 'workflowEditor.parallel.join')!.id;
    const expected = [[graph.startId, 'fork'], ['fork','design'], ['design','implement'], ['implement',join], ['fork','view'], ['view',join], [join,'report'], ['report','$end']];
    expect(flow.edges.map(e => [e.source,e.target]).sort()).toEqual(expected.sort());
    expect(graph.spec.edges.map(e => [e.from,e.to]).sort()).toEqual(expected.sort());
  });
  it.each(['RIGHT', 'DOWN'] as const)('routes both branches into the same join with ELK in %s layout', async direction => {
    const graph = createAuthoringGraph(definition, new Set(), k => k, direction);
    const layout = await layoutWorkflowGraph(graph.spec, new ELK());
    expect(layout.edges.size).toBe(8);
    expect([...layout.edges.values()].every(edge => edge.points.length >= 2)).toBe(true);
    const axis = direction === 'RIGHT' ? 'x' : 'y';
    const join = layout.nodes.get(graph.parallel.joinByGroup.get('fork')!)!;
    for (const path of [['fork','design','implement'], ['fork','view']]) {
      const positions = [...path.map(id => layout.nodes.get(id)![axis]), join[axis], layout.nodes.get('report')![axis]];
      expect(positions.every((p,i) => !i || positions[i-1] < p)).toBe(true);
    }
  });
  it('re-lays out branch order changes but not concurrency edits', () => {
    const signature = authoringWorkflowTopologySignature(definition);
    const change = (patch: Partial<typeof group>) => authoringWorkflowTopologySignature({ ...definition, nodes: [{...group,...patch}, ...definition.nodes.slice(1)] });
    expect(change({maxParallel: 1})).toBe(signature);
    expect(change({branches: [...group.branches].reverse()})).not.toBe(signature);
  });
  it('preserves a bounded retry to an earlier step in the same branch without interpreting it as an outer edge', () => {
    const local = { ...definition, control:{max_attempts:2}, edges:[...definition.edges,{from:'implement',to:'design',on:'failure'}] };
    expect(validate(local).issues).toEqual([]);
    expect(validate({ ...local, control:{} }).valid).toBe(false);
    expect(validate({ ...local, edges:[...definition.edges,{from:'design',to:'implement',on:'failure'}] }).valid).toBe(false);
  });
  it('rejects cross-branch edges, shared leaves, entry into a branch and out-of-range caps', () => {
    for (const broken of [
      {...definition, entry: 'view'},
      {...definition, nodes: [{...group, maxParallel: 6}, ...definition.nodes.slice(1)]},
      {...definition, nodes: [{...group, executionSlotId:'not-a-worker'}, ...definition.nodes.slice(1)]},
      {...definition, nodes: [{...group, branches:[{id:'api',stages:['AUX']},group.branches[1]]}, {id:'AUX',type:'worker' as const,profile:'role',executionSlotId:'slot-aux'}, ...definition.nodes.slice(3)]},
      {...definition, nodes: [{...group, branches: [...group.branches,{id:'extra',stages:['view']}]}, ...definition.nodes.slice(1)]},
      {...definition, edges: [...definition.edges, {from:'implement',to:'view',on:'success'}]},
    ]) expect(validate(broken).valid).toBe(false);
  });
});
