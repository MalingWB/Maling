/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MulticaUpstreamPullIntervention } from '@/components/conversation/MulticaUpstreamPullCard';
import type { MulticaUpstreamPullEventVm, UpstreamPullDecisionKind } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

type PullEventListener = (event: MulticaUpstreamPullEventVm) => void;

const apiMocks = vi.hoisted(() => ({
  listener: null as PullEventListener | null,
  subscribe: vi.fn(() => Promise.resolve(() => {})),
  decide: vi.fn((_taskId: string, _nodeId: string, _decision: UpstreamPullDecisionKind, _manualText?: string | null) => Promise.resolve()),
}));

vi.mock('@/api', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/api')>(),
  subscribeUpstreamPullEvents: (listener: PullEventListener) => {
    apiMocks.listener = listener;
    return apiMocks.subscribe();
  },
  decideUpstreamPull: apiMocks.decide,
}));

function pendingEvent(
  overrides: Partial<MulticaUpstreamPullEventVm> = {},
): MulticaUpstreamPullEventVm {
  return {
    taskId: 'task-1',
    runId: 'run-1',
    nodeId: 'node-1',
    parentIssueId: 'ISSUE-100',
    taskTitle: '上游交付任务',
    outcome: null,
    ...overrides,
  };
}

function settledEvent(
  overrides: Partial<MulticaUpstreamPullEventVm> = {},
): MulticaUpstreamPullEventVm {
  return pendingEvent({ outcome: 'resolved', ...overrides });
}

describe('MulticaUpstreamPullIntervention（会话内拉取决策卡片）', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    apiMocks.listener = null;
    apiMocks.subscribe.mockClear().mockReturnValue(Promise.resolve(() => {}));
    apiMocks.decide.mockClear().mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  function render(taskId = 'task-1', readOnly = false) {
    act(() => {
      root.render(
        <MulticaUpstreamPullIntervention taskId={taskId} readOnly={readOnly} />,
      );
    });
  }

  function emit(event: MulticaUpstreamPullEventVm) {
    act(() => {
      apiMocks.listener?.(event);
    });
  }

  const cards = () =>
    Array.from(
      container.querySelectorAll('[data-multica-upstream-pull-card="true"]'),
    );
  const buttons = () =>
    Array.from(container.querySelectorAll('button')).filter(
      (button) => !button.disabled,
    );
  const buttonByText = (text: string) =>
    Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent === text,
    );

  it('pending 事件只进入匹配 taskId 的会话，重复 pending 幂等，settled 收口移除', () => {
    render('task-1');
    expect(cards()).toHaveLength(0);

    emit(pendingEvent({ taskId: 'task-other' }));
    expect(cards()).toHaveLength(0);

    emit(pendingEvent());
    expect(cards()).toHaveLength(1);

    emit(pendingEvent());
    expect(cards()).toHaveLength(1);

    emit(settledEvent());
    expect(cards()).toHaveLength(0);
  });

  it('多个在飞拉取各渲染一张卡片，settled 按槽位精确移除', () => {
    render();
    emit(pendingEvent({ nodeId: 'node-1' }));
    emit(pendingEvent({ nodeId: 'node-2' }));
    expect(cards()).toHaveLength(2);

    emit(settledEvent({ nodeId: 'node-1' }));
    expect(cards()).toHaveLength(1);
  });

  it('三选一把决策提交到 decideUpstreamPull 并本地收起卡片', async () => {
    render();
    emit(pendingEvent());

    await act(async () => {
      buttonByText('跳过拉取')!.click();
    });
    expect(apiMocks.decide).toHaveBeenCalledWith(
      'task-1',
      'node-1',
      'skip',
      null,
    );
    expect(cards()).toHaveLength(0);

    emit(pendingEvent());
    await act(async () => {
      buttonByText('继续等待拉取')!.click();
    });
    expect(apiMocks.decide).toHaveBeenCalledWith(
      'task-1',
      'node-1',
      'retry',
      null,
    );
    expect(cards()).toHaveLength(0);
  });

  it('手动模式：空文本禁止提交，提交时传 trim 后的交付说明', async () => {
    render();
    emit(pendingEvent());

    await act(async () => {
      buttonByText('手动提供信息')!.click();
    });

    const submit = buttonByText('使用此信息继续')!;
    expect(submit.disabled).toBe(true);

    const textarea = container.querySelector('textarea')!;
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      'value',
    )?.set;
    await act(async () => {
      valueSetter?.call(textarea, '  构建产物：gold-band.msi  ');
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
    });

    await act(async () => {
      buttonByText('使用此信息继续')!.click();
    });
    expect(apiMocks.decide).toHaveBeenCalledWith(
      'task-1',
      'node-1',
      'manual',
      '构建产物：gold-band.msi',
    );
    expect(cards()).toHaveLength(0);
  });

  it('决策迟到（upstream-pull-not-pending）幂等收起，其他错误保留卡片并展示', async () => {
    render();
    emit(pendingEvent());

    apiMocks.decide.mockRejectedValueOnce({
      code: 'multica.upstream-pull-not-pending',
    });
    await act(async () => {
      buttonByText('跳过拉取')!.click();
    });
    expect(cards()).toHaveLength(0);

    emit(pendingEvent());
    apiMocks.decide.mockRejectedValueOnce({
      code: 'multica.network-failed',
      params: {},
    });
    await act(async () => {
      buttonByText('跳过拉取')!.click();
    });
    expect(cards()).toHaveLength(1);
    expect(container.querySelector('.text-destructive')?.textContent).toBeTruthy();

    // 失败后可重试提交。
    apiMocks.decide.mockResolvedValueOnce(undefined);
    await act(async () => {
      buttonByText('跳过拉取')!.click();
    });
    expect(cards()).toHaveLength(0);
  });

  it('readOnly 会话禁用全部决策入口', () => {
    render('task-1', true);
    emit(pendingEvent());

    expect(cards()).toHaveLength(1);
    expect(buttons()).toHaveLength(0);
  });
});
