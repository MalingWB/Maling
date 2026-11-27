import type { WorkflowDsl, WorkflowEdgeDsl, WorkflowNodeDsl, WorkflowParallelGroupNodeDsl, WorkflowWorkerNodeDsl } from '@/types';

export const WORKFLOW_PARALLEL_DEFAULT = 3;
export const PARALLEL_CAPS = [1, 2, 3, 4, 5] as const;
export const MAX_GROUP_STAGES = 64;
export const MIN_BRANCHES = 2;
type Branch = WorkflowParallelGroupNodeDsl['branches'][number];
type Member = { group: WorkflowParallelGroupNodeDsl; branch: Branch; index: number };

export function parallelMembers(nodes: WorkflowNodeDsl[]) {
  const members = new Map<string, Member>();
  for (const group of nodes) if (group.type === 'parallel-group') {
    for (const branch of group.branches) branch.stages.forEach((id, index) => members.set(id, { group, branch, index }));
  }
  return members;
}

const groupFields = new Set(['type', 'id', 'branches', 'maxParallel']);
const branchFields = new Set(['id', 'stages']);

export function hasParallelShape(value: WorkflowParallelGroupNodeDsl) {
  return Object.keys(value).every(k => groupFields.has(k)) && typeof value.id === 'string' && Array.isArray(value.branches)
    && value.branches.every(b => b && Object.keys(b).every(k => branchFields.has(k)) && typeof b.id === 'string'
      && Array.isArray(b.stages) && b.stages.every(s => typeof s === 'string'));
}

// Matches the DSL's portable component contract for group and stage IDs used in work directories.
const utf8 = new TextEncoder();
function portableComponent(id: string) {
  return !!id && id === id.trim() && !id.endsWith('.') && utf8.encode(id).length <= 128
    && !/[\p{Cc}<>:"/\\|?*]/u.test(id) && !/^(?:CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM\d|LPT\d)(?:\.|$)/i.test(id);
}

export function parallelEdgeAllowed(workflow: WorkflowDsl, edge: WorkflowEdgeDsl, members = parallelMembers(workflow.nodes)) {
  const source = members.get(edge.from), target = members.get(edge.to);
  if (source) return edge.on === 'failure' && (workflow.control.max_attempts ?? 0) > 0 && !edge.new_round_entry
    && target?.branch === source.branch && target.index <= source.index;
  if (target || (edge.new_round_entry && members.has(edge.new_round_entry.trim()))) return false;
  return edge.session !== 'continue' || !workflow.nodes.some(n => n.type === 'parallel-group' && (n.id === edge.from || n.id === edge.to));
}

export function invalidParallelGroups(nodes: WorkflowNodeDsl[]) {
  const byId = new Map(nodes.map(n => [n.id, n]));
  const seen = new Set<string>();
  const invalid: string[] = [];
  for (const group of nodes) if (group.type === 'parallel-group') {
    const ids = new Set<string>();
    let count = 0;
    let valid = hasParallelShape(group) && portableComponent(group.id) && PARALLEL_CAPS.some(cap => cap === group.maxParallel) && group.branches.length >= MIN_BRANCHES;
    for (const branch of group.branches) {
      if (!branch.id.trim() || ids.has(branch.id) || !branch.stages.length) valid = false;
      ids.add(branch.id);
      for (const id of branch.stages) {
        const node = byId.get(id);
        if (seen.has(id) || !portableComponent(id) || node?.type !== 'worker' || node.manual_check) valid = false;
        seen.add(id);
        count++;
      }
    }
    if (!valid || count > MAX_GROUP_STAGES) invalid.push(group.id);
  }
  return invalid;
}

export function availableId(ids: Iterable<string>, base: string) {
  const taken = new Set(ids);
  let candidate = base;
  for (let i = 2; taken.has(candidate); ++i) candidate = `${base}-${i}`;
  return candidate;
}

export function createAuthoringWorkerNode(workflow: Pick<WorkflowDsl, 'nodes'>, baseId: string, createSlotId = () => crypto.randomUUID() as string): WorkflowWorkerNodeDsl {
  return { type: 'worker', id: availableId(workflow.nodes.map(n => n.id), baseId), executionSlotId: createSlotId(), goal: null };
}

/** Insert in the single outer success path. Membership never becomes a persisted edge. */
export function insertParallelGroup(workflow: WorkflowDsl, after: string | null) {
  if (after !== null && (parallelMembers(workflow.nodes).has(after) || !workflow.nodes.some(n => n.id === after))) return null;
  const outgoing = workflow.edges.filter(e => e.from === after && e.on === 'success');
  if (outgoing.length > 1 || outgoing[0]?.session === 'continue') return null; // never silently discard a continuation contract
  const id = availableId(workflow.nodes.map(n => n.id), 'parallel');
  const group: WorkflowParallelGroupNodeDsl = { type: 'parallel-group', id, maxParallel: WORKFLOW_PARALLEL_DEFAULT, branches: [] };
  const nodes = [...workflow.nodes, group];
  for (let i = 1; i <= MIN_BRANCHES; ++i) {
    const branchId = `branch-${i}`;
    const leaf = createAuthoringWorkerNode({nodes}, `${id}-${branchId}`);
    nodes.push(leaf);
    group.branches.push({id: branchId, stages: [leaf.id]});
  }
  const next = after === null ? workflow.entry || '$end' : outgoing[0]?.to ?? '$end';
  const edges = workflow.edges.filter(e => e !== outgoing[0]);
  if (after !== null) edges.push({ from: after, to: id, on: 'success' });
  edges.push({ from: id, to: next, on: 'success' });
  return { group, workflow: { ...workflow, nodes, edges, entry: after === null ? id : workflow.entry } };
}

export type GroupEdit = { kind: 'cap'; value: number } | { kind: 'add-branch' } |
  { kind: 'add-step'; branch: string } | { kind: 'remove-branch'; branch: string } | { kind: 'remove-step'; branch: string; step: string } |
  { kind: 'move'; branch: string; step?: string; delta: -1 | 1 };

function move<T>(items: T[], i: number, delta: number) {
  const j = i + delta;
  if (i < 0 || j < 0 || j >= items.length) return items;
  const copy = [...items];
  [copy[i], copy[j]] = [copy[j], copy[i]];
  return copy;
}

export function editParallelGroup(workflow: WorkflowDsl, group: WorkflowParallelGroupNodeDsl, action: GroupEdit): WorkflowDsl {
  let branches = group.branches;
  let maxParallel = group.maxParallel;
  let nodes = workflow.nodes;
  const removed = new Set<string>();
  if (action.kind === 'cap') {
    if (!PARALLEL_CAPS.some(c => c === action.value)) return workflow;
    maxParallel = action.value;
  } else if (action.kind === 'add-branch' || action.kind === 'add-step') {
    if (branches.reduce((n, b) => n + b.stages.length, 0) >= MAX_GROUP_STAGES) return workflow;
    const branch = action.kind === 'add-step' ? branches.find(b => b.id === action.branch) : undefined;
    if (action.kind === 'add-step' && !branch) return workflow;
    const id = branch?.id ?? availableId(branches.map(b => b.id), `branch-${branches.length + 1}`);
    const leaf = createAuthoringWorkerNode(workflow, `${group.id}-${id}`);
    nodes = [...nodes, leaf];
    branches = branch ? branches.map(b => b === branch ? {...b, stages: [...b.stages, leaf.id]} : b) : [...branches, {id, stages: [leaf.id]}];
  } else {
    const i = branches.findIndex(b => b.id === action.branch);
    if (i < 0) return workflow;
    const branch = branches[i];
    if (action.kind === 'remove-branch') {
      if (branches.length <= MIN_BRANCHES) return workflow;
      branch.stages.forEach(s => removed.add(s));
      branches = branches.filter(b => b !== branch);
    } else if (action.kind === 'remove-step') {
      if (branch.stages.length <= 1 || !branch.stages.includes(action.step)) return workflow;
      removed.add(action.step);
      branches = branches.map(b => b === branch ? {...b, stages: b.stages.filter(s => !removed.has(s))} : b);
    } else if (action.step === undefined) branches = move(branches, i, action.delta);
    else {
      const stages = move(branch.stages, branch.stages.indexOf(action.step), action.delta);
      // A reorder may not silently invert a configured retry or remove its target.
      const positions = new Map(stages.map((s, index) => [s, index]));
      if (workflow.edges.some(e => positions.has(e.from) && positions.has(e.to) && positions.get(e.to)! > positions.get(e.from)!)) return workflow;
      branches = branches.map(b => b === branch ? {...b, stages} : b);
    }
  }
  return { ...workflow, nodes: nodes.filter(n => !removed.has(n.id)).map(n => n.id === group.id ? {...group, branches, maxParallel} : n),
    edges: removed.size ? workflow.edges.filter(e => !removed.has(e.from) && !removed.has(e.to)) : workflow.edges };
}
