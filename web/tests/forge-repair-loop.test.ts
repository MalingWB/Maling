import { describe, expect, it } from 'vitest';
import {
  FORGE_MAX_REPAIR_ATTEMPTS,
  forgeRepairActive,
  forgeRepairReducer,
  initialForgeRepairState,
  type ForgeRepairEvent,
  type ForgeRepairState,
} from '@/lib/forge-repair-loop';
import type { ForgeErrorVm } from '@/types';

/**
 * 自动修正回路的决策逻辑（纯状态机）。回路本身不落盘：每一步都由权威事实驱动——
 * 修正轮是否发出、是否收口、重新编译是否通过——这里把这些转移固化下来。
 */
const missingBoundary = (stage: string): ForgeErrorVm => ({
  code: 'forge/missing-goal-element',
  context: { stage, element: 'boundary' },
});
const openQuestions = (): ForgeErrorVm => ({
  code: 'forge/unresolved-questions',
  context: { items: '产物格式未定' },
});
const errors = [missingBoundary('s1')];

const repairNeeded = (prompt = '编译未通过，正在修正 1/3'): ForgeRepairEvent => ({ type: 'repair-needed', promptId: 'repair', prompt, errors });
const accepted: ForgeRepairEvent = { type: 'submit-settled', promptId: 'repair', result: 'accepted' };
const repairedTurn: ForgeRepairEvent = { type: 'turn-settled', promptId: 'repair', outcome: 'completed', specUpdated: true };
const stillBroken = (prompt = '编译未通过，正在修正 2/3'): ForgeRepairEvent => ({ type: 'recompiled', promptId: 'repair', outcome: 'repairable', nextPromptId: 'repair', errors, prompt });
const saved: ForgeRepairEvent = { type: 'recompiled', promptId: 'repair', outcome: 'saved' };

function run(state: ForgeRepairState, ...events: ForgeRepairEvent[]): ForgeRepairState {
  return events.reduce(forgeRepairReducer, state);
}

/** 走完一轮修正并重新编译（结果由 `recompiled` 给出）。 */
function round(state: ForgeRepairState, recompiled: ForgeRepairEvent): ForgeRepairState {
  return run(state, accepted, repairedTurn, recompiled);
}

describe('forge repair loop', () => {
  it('stays idle until a compile needs repair', () => {
    // 用户自己的对话轮收口时回路并不在等待：忽略。
    expect(forgeRepairReducer(initialForgeRepairState, repairedTurn)).toEqual({ status: 'idle' });
    expect(forgeRepairReducer(initialForgeRepairState, accepted)).toEqual({ status: 'idle' });
    expect(forgeRepairReducer(initialForgeRepairState, saved)).toEqual({ status: 'idle' });
  });

  it('sends the backend-rendered prompt, waits for the turn, then recompiles and stops once saved', () => {
    const submitting = forgeRepairReducer(initialForgeRepairState, repairNeeded());
    expect(submitting).toMatchObject({ status: 'submitting', attempt: 1, prompt: '编译未通过，正在修正 1/3', errors });
    expect(forgeRepairActive(submitting)).toBe(true);

    const awaiting = forgeRepairReducer(submitting, accepted);
    expect(awaiting).toEqual({ status: 'awaiting-turn', promptId: 'repair', attempt: 1, errors });
    // 修正轮收口后的重新编译要用“将要发出的下一轮次”调用 forge_compile。
    expect(forgeRepairReducer(awaiting, repairedTurn)).toEqual({ status: 'recompiling', promptId: 'repair', nextAttempt: 2 });
    expect(forgeRepairReducer({ status: 'recompiling', promptId: 'repair', nextAttempt: 2 }, saved)).toEqual({ status: 'idle' });
  });

  it('keeps only the first matching early terminal and ignores stale callbacks', () => {
    const sending = run(initialForgeRepairState, repairNeeded());
    const pending = forgeRepairReducer(sending, repairedTurn);
    expect(forgeRepairReducer(pending, { ...repairedTurn, outcome: 'failed' })).toBe(pending);
    expect(forgeRepairReducer(pending, { ...repairedTurn, promptId: 'old' })).toBe(pending);
    expect(forgeRepairReducer(pending, { ...accepted, promptId: 'old' })).toBe(pending);
    expect(forgeRepairReducer(pending, accepted)).toMatchObject({ status: 'recompiling' });
    expect(forgeRepairReducer(pending, { ...accepted, result: 'busy' })).toMatchObject({ status: 'stopped', reason: 'busy' });
  });

  it('stops explicitly when the terminal identity or complete reply cannot be confirmed', () => {
    const awaiting = run(initialForgeRepairState, repairNeeded(), accepted);
    for (const terminal of [{ ...repairedTurn, promptId: null }, { ...repairedTurn, unconfirmed: true }]) {
      expect(forgeRepairReducer(awaiting, terminal)).toMatchObject({ status: 'stopped', reason: 'unconfirmed' });
    }
  });

  it('counts repair rounds and gives up after the limit with the remaining errors', () => {
    let state = forgeRepairReducer(initialForgeRepairState, repairNeeded());
    for (let attempt = 1; attempt < FORGE_MAX_REPAIR_ATTEMPTS; attempt += 1) {
      state = round(state, stillBroken());
      expect(state).toMatchObject({ status: 'submitting', attempt: attempt + 1 });
    }
    // 第 3 轮修正后重新编译仍不通过：不再发第 4 轮。
    expect(FORGE_MAX_REPAIR_ATTEMPTS).toBe(3);
    state = round(state, stillBroken('编译未通过，正在修正 3/3'));
    expect(state).toEqual({ status: 'stopped', reason: 'exhausted', attempt: 3, errors });
  });

  it('never auto-answers questions the user still has to decide', () => {
    // 首次编译就带未收敛项：不发修正轮。
    expect(
      forgeRepairReducer(initialForgeRepairState, { type: 'repair-needed', promptId: 'repair', prompt: 'p', errors: [openQuestions()] }),
    ).toEqual({ status: 'stopped', reason: 'handed-back', attempt: 0, errors: [openQuestions()] });
    // 修正过程中模型把问题写回 openQuestions：同样交还用户。
    const handedBack = round(
      forgeRepairReducer(initialForgeRepairState, repairNeeded()),
      { type: 'recompiled', promptId: 'repair', outcome: 'repairable', nextPromptId: 'repair', errors: [missingBoundary('s2'), openQuestions()], prompt: 'p' },
    );
    expect(handedBack).toEqual({ status: 'stopped', reason: 'handed-back', attempt: 1, errors: [missingBoundary('s2'), openQuestions()] });
  });

  it('stops when the repair prompt cannot be sent', () => {
    const submitting = forgeRepairReducer(initialForgeRepairState, repairNeeded());
    // 第二轮发出时会话正忙：已完成的轮数照实保留。
    const second = round(submitting, stillBroken());
    expect(forgeRepairReducer(second, { type: 'submit-settled', promptId: 'repair', result: 'busy' })).toEqual({
      status: 'stopped', reason: 'busy', attempt: 1, errors,
    });
    expect(forgeRepairReducer(submitting, { type: 'submit-settled', promptId: 'repair', result: 'unavailable' })).toEqual({
      status: 'stopped', reason: 'unavailable', attempt: 0, errors,
    });
  });

  it('stops when the repair turn ends without a usable delta', () => {
    const awaiting = run(initialForgeRepairState, repairNeeded(), accepted);
    expect(forgeRepairReducer(awaiting, { type: 'turn-settled', promptId: 'repair', outcome: 'cancelled', specUpdated: false })).toMatchObject({
      status: 'stopped', reason: 'turn-cancelled', attempt: 1,
    });
    expect(forgeRepairReducer(awaiting, { type: 'turn-settled', promptId: 'repair', outcome: 'failed', specUpdated: true })).toMatchObject({
      status: 'stopped', reason: 'turn-failed', attempt: 1,
    });
    // 正常收口但回复里没有增量：没有可重新编译的东西。
    expect(forgeRepairReducer(awaiting, { type: 'turn-settled', promptId: 'repair', outcome: 'completed', specUpdated: false })).toMatchObject({
      status: 'stopped', reason: 'no-change', attempt: 1,
    });
  });

  it('distinguishes a delta the model failed to format from a turn without a delta', () => {
    // 真机回归：模型输出的增量 JSON 漏了右括号，后端带着原因收口。回路必须停下来并
    // 说明是“没应用”，而不是一直等，也不是含糊地说“没有调整”。
    const awaiting = run(initialForgeRepairState, repairNeeded(), accepted);
    expect(forgeRepairReducer(awaiting, {
      type: 'turn-settled', promptId: 'repair', outcome: 'completed', specUpdated: false, invalidDelta: true,
    })).toEqual({ status: 'stopped', reason: 'invalid-delta', attempt: 1, errors });
  });

  it('stops when the recompile command itself fails', () => {
    const recompiling = run(initialForgeRepairState, repairNeeded(), accepted, repairedTurn);
    expect(forgeRepairReducer(recompiling, { type: 'recompiled', promptId: 'repair', outcome: 'failed' })).toEqual({
      status: 'stopped', reason: 'compile-failed', attempt: 1, errors: [],
    });
  });

  it('lets the user abort an active loop, and only an active loop', () => {
    const awaiting = run(initialForgeRepairState, repairNeeded(), accepted);
    expect(forgeRepairReducer(awaiting, { type: 'abort' })).toEqual({ status: 'stopped', reason: 'aborted', attempt: 1, errors });
    expect(forgeRepairReducer({ status: 'recompiling', promptId: 'repair', nextAttempt: 2 }, { type: 'abort' })).toEqual({
      status: 'stopped', reason: 'aborted', attempt: 1, errors: [],
    });
    expect(forgeRepairReducer(initialForgeRepairState, { type: 'abort' })).toEqual({ status: 'idle' });
  });

  it('ignores events that do not belong to the current step', () => {
    const awaiting = run(initialForgeRepairState, repairNeeded(), accepted);
    // 重复的收口通知、提前到达的编译结果、重复的发送结果都不改变状态。
    expect(forgeRepairReducer(awaiting, accepted)).toBe(awaiting);
    expect(forgeRepairReducer(awaiting, saved)).toBe(awaiting);
    expect(forgeRepairReducer(awaiting, repairNeeded('again'))).toBe(awaiting);
    const recompiling = forgeRepairReducer(awaiting, repairedTurn);
    expect(forgeRepairReducer(recompiling, repairedTurn)).toBe(recompiling);
  });

  it('restarts from a clean slate when the user compiles again after a stop', () => {
    const stopped = forgeRepairReducer(
      run(initialForgeRepairState, repairNeeded(), accepted),
      { type: 'turn-settled', promptId: 'repair', outcome: 'cancelled', specUpdated: false },
    );
    const idle = forgeRepairReducer(stopped, { type: 'reset' });
    expect(idle).toEqual({ status: 'idle' });
    expect(forgeRepairReducer(idle, repairNeeded())).toMatchObject({ status: 'submitting', attempt: 1 });
    // 回路进行中不接受重置（那等于绕过“停止”）。
    const awaiting = run(idle, repairNeeded(), accepted);
    expect(forgeRepairReducer(awaiting, { type: 'reset' })).toBe(awaiting);
  });
});
