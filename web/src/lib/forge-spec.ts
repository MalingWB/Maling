import type { ForgeOnFailureVm, ForgeQuestionVm, ForgeSpecVm, ForgeStageVm } from '../types';

/** 待澄清项的统一形态：后端序列化为对象，字符串形态只出现在旧存档里。 */
export function forgeQuestion(item: string | ForgeQuestionVm): ForgeQuestionVm {
  return typeof item === 'string' ? { question: item, recommendation: null } : item;
}

export type ForgeIndexedStage = {
  stage: ForgeStageVm;
  /** 阶段的稳定身份；旧存档里没有 id 的阶段按所在位置标识。 */
  id: string;
  workflowIndex: number;
  stageIndex: number;
  /** 在全部工作流里的连续序号，从 1 开始（与预览里的编号一致）。 */
  ordinal: number;
};

/** 跨工作流连续编号的阶段清单。 */
export function forgeIndexedStages(spec: ForgeSpecVm | null | undefined): ForgeIndexedStage[] {
  const result: ForgeIndexedStage[] = [];
  (spec?.workflows ?? []).forEach((workflow, workflowIndex) => {
    workflow.stages.forEach((stage, stageIndex) => {
      result.push({
        stage,
        id: stage.id ?? `${workflowIndex}:${stageIndex}`,
        workflowIndex,
        stageIndex,
        ordinal: result.length + 1,
      });
    });
  });
  return result;
}

/** 影响保存结果的草稿内容（不含待澄清项）：内容变过，上一次保存就不再代表当前草稿。 */
function forgeSpecContent(spec: ForgeSpecVm | null | undefined): unknown {
  return {
    name: spec?.name ?? null,
    intent: spec?.intent ?? '',
    workflows: (spec?.workflows ?? []).map((workflow) => ({
      name: workflow.name,
      id: workflow.id,
      parallelGroups: workflow.parallelGroups ?? [],
      stages: workflow.stages.map((stage) => stage),
    })),
  };
}

export function forgeSpecContentKey(spec: ForgeSpecVm | null | undefined): string {
  return JSON.stringify(forgeSpecContent(spec));
}

/**
 * 两次读取之间被改动的阶段：新增、删除，或内容变化。预览据此标出“本轮修改”的阶段；
 * 只改了待澄清项不算。阶段以 id 对齐，没有 id 的阶段（旧存档）按位置对齐。
 */
export function changedForgeStageIds(
  before: ForgeSpecVm | null | undefined,
  after: ForgeSpecVm | null | undefined,
): ReadonlySet<string> {
  const changed = new Set<string>();
  if (before === after) return changed;
  if (!before || !after) {
    if (before || after) forgeIndexedStages(after).forEach(({ id }) => changed.add(id));
    return changed;
  }
  const keyOf = (entry: ForgeIndexedStage) => entry.stage.id ?? `#${entry.workflowIndex}:${entry.stageIndex}`;
  const previous = new Map(forgeIndexedStages(before).map((entry) => [keyOf(entry), entry]));
  const current = forgeIndexedStages(after);
  for (const entry of current) {
    const old = previous.get(keyOf(entry));
    if (!old || JSON.stringify(old.stage) !== JSON.stringify(entry.stage)) changed.add(entry.id);
  }
  const present = new Set(current.map(keyOf));
  for (const [key, old] of previous) {
    if (!present.has(key)) changed.add(old.id);
  }
  return changed;
}

export type ForgeOnFailureKind = 'retry' | 'manual-check' | 'end' | 'new-round' | 'new-round-from' | 'jump';

/** 失败策略的种类；跳转与“从指定阶段重开”另带一个目标阶段。 */
export function forgeOnFailureKind(value: ForgeOnFailureVm | undefined): ForgeOnFailureKind {
  if (!value || typeof value === 'string') return value ?? 'end';
  return 'new-round' in value ? 'new-round-from' : 'jump';
}

/** 失败去向的目标阶段；不带目标的策略返回 null。 */
export function forgeOnFailureTarget(value: ForgeOnFailureVm | undefined): string | null {
  if (!value || typeof value === 'string') return null;
  return 'new-round' in value ? value['new-round'] : value.goto;
}

export function forgeOnFailureContinuesSession(value: ForgeOnFailureVm | undefined): boolean {
  return Boolean(value && typeof value !== 'string' && 'goto' in value && value.continueSession === true);
}

/**
 * 失败去向只能指向同一条工作流里的其他阶段，按草稿顺序排列。没有其他阶段时为空，
 * 这两种策略不可选。
 */
export function forgeFailureTargets(spec: ForgeSpecVm, stageId: string): ForgeIndexedStage[] {
  const located = forgeIndexedStages(spec).find((entry) => entry.id === stageId);
  if (!located) return [];
  const entries = forgeIndexedStages(spec).filter(e => e.workflowIndex === located.workflowIndex && e.id !== located.id);
  const branches = spec.workflows?.[located.workflowIndex]?.parallelGroups?.flatMap(g => g.branches) ?? [];
  const branch = branches.find(b => b.stages.includes(stageId));
  if (branch) {
    const earlier = new Set(branch.stages.slice(0, branch.stages.indexOf(stageId)));
    return entries.filter(e => earlier.has(e.id));
  }
  const members = new Set(branches.flatMap(b => b.stages));
  return entries.filter(e => !members.has(e.id));
}

/**
 * 把策略种类落成 Spec 取值。跳转与“从指定阶段重开”需要一个目标：没给时取给定阶段，
 * 仍然没有就退回 `retry`（单阶段工作流里没有可去的地方）。跳转默认续接目标阶段上一次的会话。
 */
export function forgeOnFailureValue(
  kind: ForgeOnFailureKind,
  options: { target?: string | null; continueSession?: boolean },
): ForgeOnFailureVm | null {
  switch (kind) {
    case 'retry':
    case 'manual-check':
    case 'end':
    case 'new-round':
      return kind;
    case 'new-round-from':
      return options.target ? { 'new-round': options.target } : null;
    case 'jump': {
      if (!options.target) return null;
      return options.continueSession === false
        ? { goto: options.target }
        : { goto: options.target, continueSession: true };
    }
  }
}
