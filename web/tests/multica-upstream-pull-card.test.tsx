/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MulticaUpstreamPullIntervention } from '@/components/conversation/MulticaUpstreamPullCard';
import type { MulticaUpstreamPullEventVm, MulticaUpstreamPullPendingVm, UpstreamPullDecisionKind } from '@/types';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

type PullEventListener = (event: MulticaUpstreamPullEventVm) => void;

const apiMocks = vi.hoisted(() => ({
  listener: null as PullEventListener | null,
  subscribe: vi.fn(() => Promise.resolve(() => {})),
  decide: vi.fn((_taskId: string, _nodeId: string, _decision: UpstreamPullDecisionKind, _manualText?: string | null) => Promise.resolve()),
  listPendings: vi.fn((_taskId: string) => Promise.resolve<MulticaUpstreamPullPendingVm[]>([])),
}));

vi.mock('@/api', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/api')>(),
  subscribeUpstreamPullEvents: (listener: PullEventListener) => {
    apiMocks.listener = listener;
    return apiMocks.subscribe();
  },
  decideUpstreamPull: apiMocks.decide,
  listUpstreamPullPendings: apiMocks.listPendings,
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

function pendingRow(
  overrides: Partial<MulticaUpstreamPullPendingVm> = {},
): MulticaUpstreamPullPendingVm {
  return {
    taskId: 'task-1',
    runId: 'run-1',
    nodeId: 'node-1',
    parentIssueId: 'ISSUE-100',
    taskTitle: '上游交付任务',
    retried: false,
    ...overrides,
  };
}

describe('MulticaUpstreamPullIntervention（会话内拉取决策卡片）', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    apiMocks.listener = null;
    apiMocks.subscribe.mockClear().mockReturnValue(Promise.resolve(() => {}));
    apiMocks.decide.mockClear().mockResolvedValue(undefined);
    apiMocks.listPendings.mockClear().mockResolvedValue([]);
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
  const waitingCards = () =>
    Array.from(
      container.querySelectorAll('[data-multica-upstream-pull-waiting="true"]'),
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

  it('水合恢复重进会话前的待决槽；retried 槽恢复为等待态', async () => {
    apiMocks.listPendings.mockResolvedValue([
      pendingRow({ nodeId: 'node-1' }),
      pendingRow({ nodeId: 'node-2', retried: true }),
    ]);
    render();
    await act(async () => {});
    expect(cards()).toHaveLength(2);
    // retried 槽直接进入等待态；未 retried 槽保持选择态（仍有「继续等待」）。
    expect(waitingCards()).toHaveLength(1);
    expect(buttonByText('继续等待拉取')).toBeTruthy();
    expect(buttonByText('跳过拉取')).toBeTruthy();
    expect(buttonByText('手动提供信息')).toBeTruthy();
  });

  it('settled 先于水合响应到达时，迟到水合行不复活已收口卡片', async () => {
    let resolveHydration: (rows: MulticaUpstreamPullPendingVm[]) => void = () => {};
    apiMocks.listPendings.mockReturnValue(
      new Promise<MulticaUpstreamPullPendingVm[]>((resolve) => {
        resolveHydration = resolve;
      }),
    );
    render();
    emit(pendingEvent());
    emit(settledEvent());
    expect(cards()).toHaveLength(0);

    await act(async () => {
      resolveHydration([pendingRow()]);
    });
    expect(cards()).toHaveLength(0);
  });

  it('水合失败静默（事件流仍是主数据源，不阻塞挂载）', async () => {
    apiMocks.listPendings.mockRejectedValue(new Error('ipc down'));
    render();
    await act(async () => {});
    expect(cards()).toHaveLength(0);

    emit(pendingEvent());
    expect(cards()).toHaveLength(1);
  });

  it('跳过/手动提交后收起卡片；「继续等待」转等待态且不收起', async () => {
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
    // M5-cj：槽保留——卡片转等待态而非收起，settled 到达才收口。
    expect(cards()).toHaveLength(1);
    expect(waitingCards()).toHaveLength(1);
    expect(buttonByText('继续等待拉取')).toBeUndefined();

    emit(settledEvent());
    expect(cards()).toHaveLength(0);
  });

  it('等待态下手动提交仍可收口卡片', async () => {
    apiMocks.listPendings.mockResolvedValue([pendingRow({ retried: true })]);
    render();
    await act(async () => {});
    expect(waitingCards()).toHaveLength(1);

    await act(async () => {
      buttonByText('手动提供信息')!.click();
    });
    const textarea = container.querySelector('textarea')!;
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      'value',
    )?.set;
    await act(async () => {
      valueSetter?.call(textarea, '构建产物：gold-band.msi');
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

  it('手动模式：空文本禁止提交，提交时传 trim 后的交付说明，返回回到进入前模式', async () => {
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

    // 等待态进入手动 → 返回应回到等待态（而非选择态）。
    emit(pendingEvent());
    await act(async () => {
      buttonByText('继续等待拉取')!.click();
    });
    await act(async () => {
      buttonByText('手动提供信息')!.click();
    });
    await act(async () => {
      buttonByText('返回')!.click();
    });
    expect(waitingCards()).toHaveLength(1);
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
