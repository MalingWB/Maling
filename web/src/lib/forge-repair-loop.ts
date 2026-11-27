import type { ForgeErrorVm } from '../types';

/** 自动修正轮数上限（方案 §5.5）：仍失败则带着未收敛项回到用户对话。 */
export const FORGE_MAX_REPAIR_ATTEMPTS = 3;

/** 模型把问题写回 openQuestions、交还用户决定的错误码。 */
export const FORGE_ERROR_UNRESOLVED_QUESTIONS = 'forge/unresolved-questions';

export function forgeErrorsHandBackToUser(errors: ForgeErrorVm[]): boolean {
  return errors.some((error) => error.code === FORGE_ERROR_UNRESOLVED_QUESTIONS);
}

export type ForgeRepairStopReason =
  /** 用满轮数仍未通过校验。 */
  | 'exhausted'
  /** 模型把问题写回 openQuestions，交还用户决定。 */
  | 'handed-back'
  /** 会话正在回复，修正轮无法直接发出。 */
  | 'busy'
  /** 修正轮未能发出（会话不可用或被拒绝）。 */
  | 'unavailable'
  | 'unconfirmed'
  /** 修正轮被停止。 */
  | 'turn-cancelled'
  /** 修正轮执行失败。 */
  | 'turn-failed'
  /** 修正轮正常结束，但回复里没有可解析的 Spec 增量。 */
  | 'no-change'
  /** 修正轮回复里有增量，但 JSON 不合法、没有应用。 */
  | 'invalid-delta'
  /** 重新编译时命令失败。 */
  | 'compile-failed'
  /** 用户手动停止。 */
  | 'aborted';

/**
 * 编译未通过后的自动修正回路。
 *
 * 回路是打开着的铸造预览 Tab 的一次性自动操作，不是另一份业务状态：每一步都由
 * 权威事实驱动——修正轮是否发出看会话提交结果，是否结束看后端的轮次收口通知，
 * 是否修好看重新编译的结果。对话记录与 Spec 存档才是事实源，所以回路状态不落盘。
 *
 * 环节：发出修正轮（submitting）→ 等这一轮收口（awaiting-turn）→ 重新编译
 * （recompiling）；未通过则带着新的错误清单进入下一轮，直到通过或停止（stopped）。
 */
export type ForgeRepairState =
  | { status: 'idle' }
  | { status: 'submitting'; attempt: number; errors: ForgeErrorVm[]; prompt: string; promptId: string; pending?: RepairTerminal }
  | { status: 'awaiting-turn'; attempt: number; errors: ForgeErrorVm[]; promptId: string }
  /** `nextAttempt`：这次重新编译若仍不通过，将要发出的修正轮次。 */
  | { status: 'recompiling'; nextAttempt: number; promptId: string }
  /** `attempt`：已发出的修正轮数。 */
  | { status: 'stopped'; reason: ForgeRepairStopReason; attempt: number; errors: ForgeErrorVm[] };

export type ForgeRepairEvent =
  /** 用户重新发起编译：清掉上一轮的停止状态，回路回到起点。 */
  | { type: 'reset' }
  /** 编译未通过。`prompt` 是后端按轮次渲染好的修正提示词。 */
  | { type: 'repair-needed'; promptId: string; prompt: string; errors: ForgeErrorVm[] }
  | { type: 'submit-settled'; promptId: string; result: 'accepted' | 'busy' | 'unavailable' | 'unconfirmed' }
  | RepairTerminal
  | { type: 'recompiled'; promptId: string; outcome: 'saved' }
  | { type: 'recompiled'; promptId: string; outcome: 'failed' }
  | { type: 'recompiled'; promptId: string; outcome: 'repairable'; nextPromptId: string; errors: ForgeErrorVm[]; prompt: string }
  | { type: 'abort' };

type RepairTerminal = {
  type: 'turn-settled';
  promptId: string | null;
  unconfirmed?: boolean;
  outcome: 'completed' | 'cancelled' | 'failed';
  specUpdated: boolean;
  /** 回复里有增量但无法应用。 */
  invalidDelta?: boolean;
};

export const initialForgeRepairState: ForgeRepairState = { status: 'idle' };

/** 回路进行中的三个状态：发出修正轮、等待收口、重新编译。 */
export type ForgeRepairActiveState = Extract<
  ForgeRepairState,
  { status: 'submitting' | 'awaiting-turn' | 'recompiling' }
>;

export function forgeRepairActive(state: ForgeRepairState): state is ForgeRepairActiveState {
  return state.status === 'submitting' || state.status === 'awaiting-turn' || state.status === 'recompiling';
}

export function forgeRepairReducer(state: ForgeRepairState, event: ForgeRepairEvent): ForgeRepairState {
  switch (event.type) {
    case 'reset':
      // 回路进行中不接受重置：那等于绕过“停止”直接另起一轮。
      return forgeRepairActive(state) ? state : { status: 'idle' };
    case 'repair-needed': {
      if (state.status !== 'idle') return state;
      // 问题需要用户决定时不自动修：模型会照着提示词编造答案。
      if (forgeErrorsHandBackToUser(event.errors)) {
        return { status: 'stopped', reason: 'handed-back', attempt: 0, errors: event.errors };
      }
      return { status: 'submitting', attempt: 1, errors: event.errors, prompt: event.prompt, promptId: event.promptId };
    }
    case 'submit-settled': {
      if (state.status !== 'submitting' || state.promptId !== event.promptId) return state;
      if (event.result === 'accepted') {
        const awaiting = { status: 'awaiting-turn', attempt: state.attempt, errors: state.errors, promptId: state.promptId } as const;
        return state.pending ? forgeRepairReducer(awaiting, state.pending) : awaiting;
      }
      return { status: 'stopped', reason: event.result, attempt: state.attempt - 1, errors: state.errors };
    }
    case 'turn-settled': {
      if (state.status !== 'submitting' && state.status !== 'awaiting-turn') return state;
      if (state.status === 'submitting' && state.pending) return state;
      if (event.promptId === null) return { status: 'stopped', reason: 'unconfirmed', attempt: state.attempt, errors: state.errors };
      if (event.promptId !== state.promptId) return state;
      if (state.status === 'submitting') return { ...state, pending: event };
      if (event.outcome === 'cancelled') {
        return { status: 'stopped', reason: 'turn-cancelled', attempt: state.attempt, errors: state.errors };
      }
      if (event.outcome === 'failed') {
        return { status: 'stopped', reason: 'turn-failed', attempt: state.attempt, errors: state.errors };
      }
      if (event.unconfirmed) return { status: 'stopped', reason: 'unconfirmed', attempt: state.attempt, errors: state.errors };
      if (!event.specUpdated) {
        // 模型输出了增量但不合法：说清楚是“没应用”，而不是“没改动”。
        return {
          status: 'stopped',
          reason: event.invalidDelta ? 'invalid-delta' : 'no-change',
          attempt: state.attempt,
          errors: state.errors,
        };
      }
      return { status: 'recompiling', nextAttempt: state.attempt + 1, promptId: state.promptId };
    }
    case 'recompiled': {
      if (state.status !== 'recompiling' || state.promptId !== event.promptId) return state;
      if (event.outcome === 'saved') return { status: 'idle' };
      if (event.outcome === 'failed') {
        return { status: 'stopped', reason: 'compile-failed', attempt: state.nextAttempt - 1, errors: [] };
      }
      const errors = event.errors;
      if (forgeErrorsHandBackToUser(errors)) {
        return { status: 'stopped', reason: 'handed-back', attempt: state.nextAttempt - 1, errors };
      }
      if (state.nextAttempt > FORGE_MAX_REPAIR_ATTEMPTS) {
        return { status: 'stopped', reason: 'exhausted', attempt: state.nextAttempt - 1, errors };
      }
      return { status: 'submitting', attempt: state.nextAttempt, errors, prompt: event.prompt, promptId: event.nextPromptId };
    }
    case 'abort': {
      if (!forgeRepairActive(state)) return state;
      return {
        status: 'stopped',
        reason: 'aborted',
        attempt: state.status === 'recompiling' ? state.nextAttempt - 1 : state.attempt,
        errors: state.status === 'recompiling' ? [] : state.errors,
      };
    }
    default:
      return state;
  }
}
