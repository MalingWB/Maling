import { describe, expect, it } from 'vitest';
import i18n from 'i18next';
import { displayAppError } from '@/i18n';
import {
  forgeErrorText,
  forgeRepairText,
  forgeStageOrdinal,
} from '@/lib/forge-errors';
import { FORGE_MAX_REPAIR_ATTEMPTS, type ForgeRepairState } from '@/lib/forge-repair-loop';
import type { ForgeErrorVm, ForgeSpecVm } from '@/types';

/**
 * Spec 校验错误与修正回路状态的对客文案。后端只返回错误码与字段明细，文案在这里按
 * 界面语言生成；stage 用草稿里的序号指代，用户能直接对上预览里的那一行。
 */
const t = i18n.t.bind(i18n);

const spec: ForgeSpecVm = {
  specVersion: '0.1',
  name: '接口开发',
  intent: '实现接口并验收',
  workflows: [
    { name: '接口开发', stages: [
      { id: 's1', role: 'dev', goal: '目标一', outputKind: 'none', onFailure: 'retry', manualCheck: false },
      { id: 's2', role: 'test', goal: '目标二', outputKind: 'json', successCondition: '$.ok == true', onFailure: 'retry', manualCheck: false },
    ] },
    { name: '发布', stages: [
      { id: 's9', role: 'accept', goal: '目标三', outputKind: 'none', onFailure: 'end', manualCheck: false },
    ] },
  ],
  openQuestions: [],
};

const error = (code: string, context: Record<string, unknown> = {}): ForgeErrorVm => ({ code, context });

describe('forgeStageOrdinal', () => {
  it('locates a stage by id across workflows and reports unknown ids', () => {
    expect(forgeStageOrdinal(spec, 's1')).toBe(1);
    expect(forgeStageOrdinal(spec, 's2')).toBe(2);
    // 第二条工作流里的阶段接着第一条的序号，与预览里的排列一致。
    expect(forgeStageOrdinal(spec, 's9')).toBe(3);
    expect(forgeStageOrdinal(spec, 'nope')).toBeNull();
    expect(forgeStageOrdinal(null, 's1')).toBeNull();
  });
});

describe('forgeErrorText', () => {
  it('shows a recovery action for source scope errors from the preview or the create command', () => {
    for (const [code, message] of [
      ['forge/source-session-unavailable', '无法读取所选会话的消息。请重新打开源会话后重试。'],
      ['forge/stale-message', '所选消息已更改。请回到源会话，重新选择起始消息。'],
    ]) {
      expect(displayAppError(t, { code, params: {} })).toBe(message);
      expect(forgeErrorText(t, error(code), null)).toBe(message);
    }
  });
  it('names the failing stage by its position and localizes the goal element', () => {
    expect(forgeErrorText(t, error('forge/missing-goal-element', { stage: 's2', element: 'boundary' }), spec))
      .toBe('阶段「第 2 个阶段」的目标缺少「边界」');
    // 空目标：后端同样发 missing-goal-element，element 为 goal。
    expect(forgeErrorText(t, error('forge/missing-goal-element', { stage: 's1', element: 'goal' }), spec))
      .toBe('阶段「第 1 个阶段」还没有目标');
  });

  it('falls back to the raw id when the stage is not in the draft', () => {
    expect(forgeErrorText(t, error('forge/missing-goal-element', { stage: '#3', element: 'output' }), spec))
      .toBe('阶段「#3」的目标缺少「产出」');
  });

  it('covers every validation code the backend can return', () => {
    expect(forgeErrorText(t, error('forge/missing-success-condition', { stage: 's1' }), spec))
      .toBe('阶段「第 1 个阶段」的 JSON 产物还没有成功条件');
    expect(forgeErrorText(t, error('forge/empty-stages', { workflow: '发布' }), spec)).toBe('工作流「发布」还没有阶段');
    expect(forgeErrorText(t, error('forge/too-many-stages', { workflow: '发布', count: 12, limit: 10 }), spec))
      .toBe('工作流「发布」有 12 个阶段，超过上限 10');
    expect(forgeErrorText(t, error('forge/missing-name'), spec)).toBe('工作流还没有名称');
    expect(forgeErrorText(t, error('forge/compile-failed', { detail: 'success condition path not declared' }), spec))
      .toBe('工作流结构未通过检查：success condition path not declared');
    expect(forgeErrorText(t, error('forge/unresolved-questions', { items: 'a; b' }), spec)).toBe('还有问题待对话确认');
  });

  it('identifies an unavailable role by stage and persona', () => {
    expect(forgeErrorText(t, error('forge/unavailable-role', { stage: 's2', role: '复核员' }), spec))
      .toBe('第 2 个阶段引用的角色「复核员」已不可用');
  });

  it('keeps the error detail for unknown codes instead of hiding the failure', () => {
    expect(forgeErrorText(t, error('forge/some-new-code', { detail: 'x' }), spec)).toBe('生成草稿格式有误：x');
    expect(forgeErrorText(t, error('forge/some-new-code'), null)).toBe('生成草稿格式有误：forge/some-new-code');
  });
});

describe('forgeRepairText', () => {
  it('describes the running loop with its round counter', () => {
    expect(forgeRepairText(t, { status: 'submitting', promptId: 'repair', attempt: 1, errors: [], prompt: '' }))
      .toBe(`编译未通过，正在自动修正（第 1/${FORGE_MAX_REPAIR_ATTEMPTS} 轮）…`);
    expect(forgeRepairText(t, { status: 'awaiting-turn', promptId: 'repair', attempt: 2, errors: [] }))
      .toBe(`编译未通过，正在自动修正（第 2/${FORGE_MAX_REPAIR_ATTEMPTS} 轮）…`);
    expect(forgeRepairText(t, { status: 'recompiling', promptId: 'repair', nextAttempt: 2 })).toBe('修正完成，正在重新编译…');
    expect(forgeRepairText(t, { status: 'idle' })).toBeNull();
  });

  it('gives every stop reason a next step the user can act on', () => {
    const reasons = ['exhausted', 'handed-back', 'busy', 'unavailable', 'unconfirmed', 'turn-cancelled', 'turn-failed', 'no-change', 'invalid-delta', 'compile-failed', 'aborted'] as const;
    const stopped: ForgeRepairState = { status: 'stopped', reason: 'exhausted', attempt: 2, errors: [] };
    for (const reason of reasons) {
      const text = forgeRepairText(t, { ...stopped, reason });
      expect(text, reason).toBeTruthy();
      expect(text, reason).not.toBe(`forge.repair.${reason}`);
    }
    expect(forgeRepairText(t, { ...stopped, reason: 'exhausted' })).toBe('已自动修正 2 轮仍未通过，请回到对话逐项确认后再编译。');
  });
});
