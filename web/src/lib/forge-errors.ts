import type { TFunction } from 'i18next';
import type { ForgeErrorVm, ForgeSpecVm } from '../types';
import { FORGE_MAX_REPAIR_ATTEMPTS, type ForgeRepairState } from './forge-repair-loop';

const GOAL_ELEMENTS = new Set(['input', 'output', 'boundary', 'acceptance']);

type ForgeErrorContext = {
  stage?: string;
  element?: string;
  workflow?: string;
  role?: string;
  target?: string;
  stages?: string;
  name?: string;
  count?: number;
  limit?: number;
  detail?: string;
};

function readContext(error: ForgeErrorVm): ForgeErrorContext {
  const context = error.context ?? {};
  const text = (key: string) => {
    const value = context[key];
    return typeof value === 'string' || typeof value === 'number' ? String(value) : undefined;
  };
  const count = (key: string) => (typeof context[key] === 'number' ? (context[key] as number) : undefined);
  return {
    stage: text('stage'),
    element: text('element'),
    workflow: text('workflow'),
    role: text('role'),
    target: text('target'),
    stages: text('stages'),
    name: text('name'),
    count: count('count'),
    limit: count('limit'),
    detail: text('detail'),
  };
}

/** Spec 校验错误里引用的 stage 在草稿中的位置（跨工作流连续计数，与预览里的排列一致）。 */
export function forgeStageOrdinal(spec: ForgeSpecVm | null, stageId: string): number | null {
  const workflows = spec?.workflows ?? [];
  let offset = 0;
  for (const workflow of workflows) {
    const index = workflow.stages.findIndex((stage) => stage.id === stageId);
    if (index >= 0) return offset + index + 1;
    offset += workflow.stages.length;
  }
  return null;
}
/**
 * Spec 校验错误 → 本地化文案。后端只返回错误码与字段明细，对客表述在这里按界面
 * 语言生成；stage 以草稿中的序号指代，用户能直接对上预览里的那一行。
 */
export function forgeErrorText(t: TFunction, error: ForgeErrorVm, spec: ForgeSpecVm | null): string {
  const context = readContext(error);
  const stageId = context.stage ?? '';
  const ordinal = stageId ? forgeStageOrdinal(spec, stageId) : null;
  const stage = ordinal ? t('forge.preview.stageOrdinal', { index: ordinal }) : stageId;
  switch (error.code) {
    case 'forge/invalid-task-locator':
    case 'forge/not-generation-task': return t('errors.conversation.task-not-found');
    case 'forge/target-unavailable': return t('errors.forge.target-unavailable');
    case 'forge/source-session-unavailable': return t('errors.forge.source-session-unavailable');
    case 'forge/stale-message': return t('errors.forge.stale-message');
    case 'forge/revision-conflict': return t('errors.forge.revision-conflict');
    case 'forge/missing-goal-element': {
      if (context.element === 'goal') return t('errors.forge.missing-goal', { stage });
      const element = context.element ?? '';
      const elementLabel = GOAL_ELEMENTS.has(element) ? t(`forge.preview.element.${element}`) : element;
      return t('errors.forge.missing-goal-element', { stage, element: elementLabel });
    }
    case 'forge/missing-success-condition':
      return t('errors.forge.missing-success-condition', { stage });
    case 'forge/empty-stages':
      return t('errors.forge.empty-stages', { workflow: context.workflow ?? '' });
    case 'forge/too-many-stages':
      return t('errors.forge.too-many-stages', {
        workflow: context.workflow ?? '',
        count: context.count ?? 0,
        limit: context.limit ?? 0,
      });
    case 'forge/missing-name':
      return t('errors.forge.missing-name');
    case 'forge/compile-failed':
      return t('errors.forge.compile-failed', { detail: context.detail ?? '' });
    case 'forge/unresolved-questions':
      return t('errors.forge.unresolved-questions');
    case 'forge/unknown-failure-target':
      return t('errors.forge.unknown-failure-target', { stage, target: context.target ?? '' });
    case 'forge/role-id-conflict':
      return t('errors.forge.role-id-conflict', { role: context.role ?? '', stages: context.stages ?? '' });
    case 'forge/role-definition-conflict':
      return t('errors.forge.role-definition-conflict', { role: context.role ?? '', stages: context.stages ?? '' });
    case 'forge/unavailable-role':
      return t('errors.forge.unavailable-role', { stage, role: context.role ?? '' });
    case 'forge/readonly-role':
      return t('errors.forge.readonly-role', { stage, role: context.role ?? '' });
    case 'forge/duplicate-workflow-name':
      return t('errors.forge.duplicate-workflow-name', { name: context.name ?? '' });
    default:
      return t('errors.forge.invalid-spec', { detail: context.detail ?? error.code });
  }
}

/**
 * `forge_*` 命令直接拒绝时的错误（`forge/<variant>` 码 + 字段明细）：按码本地化，与预览里
 * 的校验错误共用同一套文案。不是 Forge 错误时返回 null，交给通用的错误显示。
 */
export function forgeCommandErrorText(t: TFunction, error: unknown): string | null {
  const code = (error as { code?: unknown } | null)?.code;
  if (typeof code !== 'string' || !code.startsWith('forge/')) return null;
  const params = (error as { params?: unknown }).params;
  return forgeErrorText(
    t,
    { code, context: params && typeof params === 'object' ? params as Record<string, unknown> : {} },
    null,
  );
}

/** 自动修正回路的进行态与停止原因文案。 */
export function forgeRepairText(t: TFunction, state: ForgeRepairState): string | null {
  switch (state.status) {
    case 'submitting':
    case 'awaiting-turn':
      return t('forge.repair.inProgress', { attempt: state.attempt, max: FORGE_MAX_REPAIR_ATTEMPTS });
    case 'recompiling':
      return t('forge.repair.recompiling');
    case 'stopped':
      switch (state.reason) {
        case 'exhausted':
          return t('forge.repair.exhausted', { attempt: state.attempt });
        case 'handed-back':
          return t('forge.repair.handedBack');
        case 'busy':
          return t('forge.repair.busy');
        case 'unavailable':
          return t('forge.repair.unavailable');
        case 'unconfirmed':
          return t('forge.repair.unconfirmed');
        case 'turn-cancelled':
          return t('forge.repair.turnCancelled');
        case 'turn-failed':
          return t('forge.repair.turnFailed');
        case 'no-change':
          return t('forge.repair.noChange');
        case 'invalid-delta':
          return t('forge.repair.invalidDelta');
        case 'compile-failed':
          return t('forge.repair.compileFailed');
        case 'aborted':
          return t('forge.repair.aborted');
        default:
          return null;
      }
    default:
      return null;
  }
}
