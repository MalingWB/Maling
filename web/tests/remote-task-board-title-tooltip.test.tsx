/** @vitest-environment jsdom */

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// `t` 跨渲染稳定；返回 key 本身便于断言（徽章/aria-label 都走 i18n key）。
const stableMocks = vi.hoisted(() => ({ t: (key: string) => key }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: stableMocks.t }),
  initReactI18next: { type: '3rdParty', init: () => {} },
}));

vi.mock('lucide-react', () => ({
  Ban: () => null,
  Loader2: () => null,
  Play: () => null,
  RotateCcw: () => null,
  Trash2: () => null,
}));

vi.mock('@/lib/utils', () => ({
  cn: (...args: unknown[]) => args.filter(Boolean).join(' '),
}));

vi.mock('@/components/ui/card', () => ({
  Card: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  CardContent: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));
vi.mock('@/components/ui/badge', () => ({
  Badge: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
}));
vi.mock('@/components/ui/button', () => ({
  Button: (props: Record<string, unknown> & { children?: ReactNode }) => (
    <button {...(props as object)}>{props.children}</button>
  ),
}));

import { TooltipProvider } from '@/components/ui/tooltip';
import { RemoteTaskBoard } from '@/components/conversation/RemoteTaskBoard';
import type { RemoteTaskVm } from '@/types';

const FULL_TITLE = '很长的任务名：支付网关重构与灰度切换全链路回归验证（含回滚预案）';

function task(overrides: Partial<RemoteTaskVm> = {}): RemoteTaskVm {
  return {
    id: 'rt-1',
    issueRef: null,
    status: 'queued',
    workspaceId: 'ws-1',
    title: FULL_TITLE,
    requirement: null,
    lastActivityAt: null,
    localTaskId: null,
    runId: null,
    projectId: null,
    kind: null,
    readiness: null,
    attachments: [],
    issueStatusCategory: null,
    issueStatusName: null,
    rerunOfTaskId: null,
    dpmsLinkPending: false,
    ...overrides,
  };
}

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  host.remove();
});

async function renderBoard(tasks: RemoteTaskVm[], onSelectRun = vi.fn()) {
  await act(async () => root.render(
    <TooltipProvider delayDuration={0}>
      <RemoteTaskBoard
        tasks={tasks}
        busyTaskId={null}
        onPrepare={vi.fn()}
        onCancel={vi.fn()}
        onRerun={vi.fn()}
        onRemove={vi.fn()}
        onSelectRun={onSelectRun}
      />
    </TooltipProvider>,
  ));
  // 整板只有一张卡片时，第一个 tooltip trigger 即该卡片主体（标题 + 徽标区）。
  const trigger = host.querySelector('[data-slot="tooltip-trigger"]') as HTMLElement | null;
  expect(trigger).toBeTruthy();
  return { trigger: trigger!, onSelectRun };
}

function measureTitle(trigger: HTMLElement, { truncated }: { truncated: boolean }) {
  const title = trigger.querySelector('[data-remote-task-title="true"]') as HTMLElement | null;
  expect(title).toBeTruthy();
  Object.defineProperties(title!, {
    clientWidth: { configurable: true, value: 120 },
    scrollWidth: { configurable: true, get: () => (truncated ? 480 : 120) },
  });
  return title!;
}

function pointer(target: Element, type: string, pointerType = 'mouse') {
  const event = new MouseEvent(type, { bubbles: true, relatedTarget: document.body });
  Object.defineProperty(event, 'pointerType', { value: pointerType });
  target.dispatchEvent(event);
}

async function hover(target: Element) {
  await act(async () => {
    pointer(target, 'pointerover');
    pointer(target, 'pointermove');
  });
  await act(async () => vi.advanceTimersByTimeAsync(0));
}

function tooltip() {
  return document.body.querySelector('[data-slot="tooltip-content"]');
}

describe('remote task card title tooltip', () => {
  it('shows the full task name on hover when the card title is actually truncated', async () => {
    const { trigger } = await renderBoard([task({ status: 'queued', title: FULL_TITLE })]);
    measureTitle(trigger, { truncated: true });
    await hover(trigger);
    expect(tooltip()?.textContent).toBe(FULL_TITLE);
  });

  it('does not open for a title that fits (no meaningless empty-hover bubble)', async () => {
    const { trigger } = await renderBoard([task({ status: 'queued', title: FULL_TITLE })]);
    measureTitle(trigger, { truncated: false });
    await hover(trigger);
    expect(tooltip()).toBeNull();
  });

  it('anchors the trigger on the whole card body (title + badges), not the inner title text or action buttons', async () => {
    const { trigger } = await renderBoard([task({ status: 'queued', title: FULL_TITLE })]);
    // 标题与状态徽标都在 trigger 内：悬停卡片主体任意一处都能看到全名。
    expect(trigger.querySelector('[data-remote-task-title="true"]')).toBeTruthy();
    expect(trigger.textContent).toContain('conversation.sidebar.remoteTasks.status.queued');
    // 操作按钮保持独立控制，不并入标题 Tooltip 的命中区。
    const executeButton = host.querySelector(
      'button[aria-label="conversation.sidebar.remoteTasks.executeTask"]',
    );
    expect(executeButton).toBeTruthy();
    expect(trigger.contains(executeButton)).toBe(false);
  });

  it('keeps the full clickable terminal control as the trigger and preserves click-to-open-run', async () => {
    const onSelectRun = vi.fn();
    const { trigger } = await renderBoard(
      [task({
        id: 'c',
        status: 'completed',
        title: FULL_TITLE,
        projectId: 'proj-1',
        localTaskId: 't-1',
        runId: 'r-1',
      })],
      onSelectRun,
    );
    expect(trigger.tagName).toBe('BUTTON');
    measureTitle(trigger, { truncated: true });
    await hover(trigger);
    expect(tooltip()?.textContent).toBe(FULL_TITLE);
    await act(async () => { (trigger as HTMLButtonElement).click(); });
    expect(onSelectRun).toHaveBeenCalledWith('proj-1', 't-1', 'r-1');
  });
});
