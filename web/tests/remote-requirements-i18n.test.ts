import { describe, expect, it } from 'vitest';

import i18n from '@/i18n';

describe('Multica requirements localization', () => {
  it('uses the requirements product name in both languages', () => {
    expect(i18n.t('remote.taskManagement.title', { lng: 'zh-CN' })).toBe('需求管理');
    expect(i18n.t('conversation.sidebar.remoteTaskManagement', { lng: 'zh-CN' })).toBe('需求管理');
    expect(i18n.t('conversation.sidebar.more', { lng: 'zh-CN' })).toBe('更多');

    expect(i18n.t('remote.taskManagement.title', { lng: 'en' })).toBe('Requirements');
    expect(i18n.t('conversation.sidebar.remoteTaskManagement', { lng: 'en' })).toBe('Requirements');
    expect(i18n.t('conversation.sidebar.more', { lng: 'en' })).toBe('More');
  });

  // story dev/test 拆分（§12.40）：类型徽标 / 类型过滤 / 未就绪三组 key 双语精确值。
  // 精确值断言的目的：文案是接口层验收面（用户可见文案不能悄悄漂移成 key 或英文兜底）。
  it('localizes issue kinds and the type filter in both languages', () => {
    for (const kind of ['dev', 'test', 'bug'] as const) {
      expect(i18n.t(`remote.taskManagement.issueKind.${kind}`, { lng: 'zh-CN' })).not.toBe(
        `remote.taskManagement.issueKind.${kind}`,
      );
      expect(i18n.t(`remote.taskManagement.issueKind.${kind}`, { lng: 'en' })).not.toBe(
        `remote.taskManagement.issueKind.${kind}`,
      );
    }
    expect(i18n.t('remote.taskManagement.issueKind.dev', { lng: 'zh-CN' })).toBe('开发');
    expect(i18n.t('remote.taskManagement.issueKind.dev', { lng: 'en' })).toBe('Dev');

    expect(i18n.t('remote.taskManagement.issueKindFilter.label', { lng: 'zh-CN' })).toBe('类型');
    expect(i18n.t('remote.taskManagement.issueKindFilter.label', { lng: 'en' })).toBe('Type');
    for (const filter of ['all', 'dev', 'test'] as const) {
      expect(i18n.t(`remote.taskManagement.issueKindFilter.${filter}`, { lng: 'zh-CN' })).not.toBe(
        `remote.taskManagement.issueKindFilter.${filter}`,
      );
      expect(i18n.t(`remote.taskManagement.issueKindFilter.${filter}`, { lng: 'en' })).not.toBe(
        `remote.taskManagement.issueKindFilter.${filter}`,
      );
    }
    expect(i18n.t('remote.taskManagement.issueKindFilter.all', { lng: 'zh-CN' })).toBe('全部');
    expect(i18n.t('remote.taskManagement.issueKindFilter.all', { lng: 'en' })).toBe('All');
  });

  it('localizes the ready badge and the DPMS pending marker in both languages', () => {
    // 正向就绪徽标（取代 §12.42「未就绪」负向提醒）：开发任务已完成的 test 任务才标绿。
    expect(i18n.t('remote.taskManagement.readiness.ready', { lng: 'zh-CN' })).toBe('已就绪');
    expect(i18n.t('remote.taskManagement.readiness.ready', { lng: 'en' })).toBe('Ready');
    expect(i18n.t('remote.taskManagement.readiness.readyHint', { lng: 'zh-CN' })).toBe(
      '对应开发任务已完成，可以开始执行',
    );
    expect(i18n.t('remote.taskManagement.readiness.readyHint', { lng: 'en' })).toBe(
      'The matching dev task is done; this test task can start now',
    );

    // DPMS 待回传徽标：DPMS 是专有名词，两种语言下均保留拉丁字样不翻译。
    expect(i18n.t('remote.taskManagement.dpms.badge', { lng: 'zh-CN' })).toBe('DPMS');
    expect(i18n.t('remote.taskManagement.dpms.badge', { lng: 'en' })).toBe('DPMS');
    expect(i18n.t('remote.taskManagement.dpms.hint', { lng: 'zh-CN' })).toBe(
      '该任务需创建 DPMS 系统需求并回传关联，回传完成后此标签自动消失',
    );
    expect(i18n.t('remote.taskManagement.dpms.hint', { lng: 'en' })).toBe(
      'This task must create a DPMS story and report its link; the badge clears once reported',
    );
  });
});
