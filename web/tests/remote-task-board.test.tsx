/** @vitest-environment jsdom */

import { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

// `t` 跨渲染稳定；返回 key 本身便于断言（列头/徽章/aria-label 都走 i18n key）。
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
vi.mock('@/components/ui/tooltip', () => ({
  Tooltip: ({ children }: { children?: ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
}));

import { formatLocalDateTime } from '@/lib/datetime';
import {
  RemoteTaskBoard,
  bucketTasksByStatus,
  visibleIssueKind,
  isTaskReady,
  orderQueuedReadyFirst,
  canRerunRemoteTask,
  showsIssueStatusChip,
  RERUN_ELIGIBLE_ISSUE_CATEGORIES,
  ISSUE_STATUS_CHIP_CATEGORIES,
  BOARD_COLUMNS,
  REMOTE_STATUS_TONE,
  REMOTE_ISSUE_KIND_TONE,
} from '@/components/conversation/RemoteTaskBoard';
import type { RemoteTaskVm } from '@/types';

function task(overrides: Partial<RemoteTaskVm> = {}): RemoteTaskVm {
  return {
    id: 'rt-1',
    issueRef: null,
    status: 'queued',
    workspaceId: 'ws-1',
    title: 'Task',
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

afterEach(() => {
  document.body.innerHTML = '';
});

// 纯函数分桶：看板的接口层不变量（4 列正确 + 未知状态丢弃 + 保序 + 空输入）。
describe('bucketTasksByStatus', () => {
  it('exposes exactly the 4 canonical-status columns and returns empty buckets for empty input', () => {
    expect(BOARD_COLUMNS).toEqual(['queued', 'running', 'completed', 'failed']);
    const buckets = bucketTasksByStatus([]);
    for (const status of BOARD_COLUMNS) {
      expect(buckets[status]).toEqual([]);
    }
  });

  it('distributes tasks into the matching canonical-status bucket', () => {
    const queued = task({ id: 'q', status: 'queued' });
    const running = task({ id: 'r', status: 'running' });
    const completed = task({ id: 'c', status: 'completed' });
    const failed = task({ id: 'f', status: 'failed' });
    const buckets = bucketTasksByStatus([queued, running, completed, failed]);
    expect(buckets.queued).toEqual([queued]);
    expect(buckets.running).toEqual([running]);
    expect(buckets.completed).toEqual([completed]);
    expect(buckets.failed).toEqual([failed]);
  });

  it('drops tasks whose status is not one of the 4 canonical values (normalize 兜底)', () => {
    const unknown = task({ id: 'u', status: 'wat' as RemoteTaskVm['status'] });
    const queued = task({ id: 'q', status: 'queued' });
    const buckets = bucketTasksByStatus([unknown, queued]);
    expect(buckets.queued).toEqual([queued]);
    expect(buckets.running).toEqual([]);
    expect(buckets.completed).toEqual([]);
    expect(buckets.failed).toEqual([]);
  });

  it('preserves insertion order within each bucket', () => {
    const a = task({ id: 'a', status: 'queued' });
    const b = task({ id: 'b', status: 'queued' });
    const c = task({ id: 'c', status: 'failed' });
    const buckets = bucketTasksByStatus([c, a, b]);
    expect(buckets.queued.map((t) => t.id)).toEqual(['a', 'b']);
    expect(buckets.failed.map((t) => t.id)).toEqual(['c']);
  });
});

// 4 canonical status → 看板词汇配色（待办=灰、进行中=黄、已完成=绿、失败=红）。
describe('remote status tone config', () => {
  it('maps every canonical status to its board-vocabulary color', () => {
    expect(REMOTE_STATUS_TONE.queued).toMatch(/muted/);
    expect(REMOTE_STATUS_TONE.running).toMatch(/amber/);
    expect(REMOTE_STATUS_TONE.completed).toMatch(/emerald/);
    expect(REMOTE_STATUS_TONE.failed).toMatch(/destructive/);
    expect(Object.keys(REMOTE_STATUS_TONE).sort()).toEqual(['completed', 'failed', 'queued', 'running']);
  });
});

// issue 类型徽标 + 已就绪谓词（story dev/test 拆分；正向确认——取代 §12.42「未就绪」提醒）。
describe('remote issue kind badge and ready predicate', () => {
  it('renders badges only for dev/test/bug and hides general/missing kinds', () => {
    expect(visibleIssueKind('dev')).toBe('dev');
    expect(visibleIssueKind('test')).toBe('test');
    expect(visibleIssueKind('bug')).toBe('bug');
    // general 与缺字段不渲染徽标（对齐 multica 自有 views 的 hideGeneral 惯例）。
    expect(visibleIssueKind('general')).toBeNull();
    expect(visibleIssueKind(null)).toBeNull();
    // 未知类型同样不渲染（前向兼容：新类型不猜文案）。
    expect(visibleIssueKind('epic')).toBeNull();
  });

  it('maps every badge kind to its own tone', () => {
    expect(REMOTE_ISSUE_KIND_TONE.dev).toMatch(/sky/);
    expect(REMOTE_ISSUE_KIND_TONE.test).toMatch(/violet/);
    expect(REMOTE_ISSUE_KIND_TONE.bug).toMatch(/destructive/);
  });

  it('flags only ready test tasks (正向谓词：test 且 readiness === true 才标「已就绪」)', () => {
    // 非 test（含旧 server 的 null）恒无就绪徽标，不受 readiness 影响。
    expect(isTaskReady({ kind: 'dev', readiness: true })).toBe(false);
    expect(isTaskReady({ kind: 'bug', readiness: true })).toBe(false);
    expect(isTaskReady({ kind: null, readiness: true })).toBe(false);
    // test：只有 readiness === true 才正向确认（false / null 缺失 = 无信号不标）。
    expect(isTaskReady({ kind: 'test', readiness: true })).toBe(true);
    expect(isTaskReady({ kind: 'test', readiness: false })).toBe(false);
    expect(isTaskReady({ kind: 'test', readiness: null })).toBe(false);
  });
});

// 待办列排序：「已就绪」稳定靠前（与绿色徽标同一用户逻辑），非就绪保持后端顺序。
describe('orderQueuedReadyFirst (待办列就绪优先排序)', () => {
  it('floats ready test tasks to the front and keeps the rest in server order', () => {
    const plain1 = task({ id: 'p1', kind: 'dev' });
    const ready1 = task({ id: 'r1', kind: 'test', readiness: true });
    const plain2 = task({ id: 'p2', kind: 'bug' });
    const ready2 = task({ id: 'r2', kind: 'test', readiness: true });
    const notReady = task({ id: 'nr', kind: 'test', readiness: false });
    const ordered = orderQueuedReadyFirst([plain1, ready1, plain2, ready2, notReady]);
    // 就绪优先（相对顺序稳定），非就绪（含未就绪 test）保持原顺序。
    expect(ordered.map((t) => t.id)).toEqual(['r1', 'r2', 'p1', 'p2', 'nr']);
  });

  it('returns an equal-order list when nothing is ready (纯排序，不改内容)', () => {
    const a = task({ id: 'a' });
    const b = task({ id: 'b', kind: 'test', readiness: false });
    expect(orderQueuedReadyFirst([a, b]).map((t) => t.id)).toEqual(['a', 'b']);
    expect(orderQueuedReadyFirst([])).toEqual([]);
  });
});

// 重新执行判定（multica 任务重试改造）：终态 + 对账回填的 issue 状态分类闭集。
describe('canRerunRemoteTask (rerun eligibility)', () => {
  it('allows rerun only for terminal tasks whose reconciled issue category is todo/backlog/done', () => {
    for (const category of RERUN_ELIGIBLE_ISSUE_CATEGORIES) {
      expect(canRerunRemoteTask({ status: 'completed', issueStatusCategory: category })).toBe(true);
      expect(canRerunRemoteTask({ status: 'failed', issueStatusCategory: category })).toBe(true);
    }
  });

  it('rejects in_progress (issue 正被他端执行，multica 按 issue+agent 串行派发)', () => {
    expect(canRerunRemoteTask({ status: 'completed', issueStatusCategory: 'in_progress' })).toBe(false);
    expect(canRerunRemoteTask({ status: 'failed', issueStatusCategory: 'in_progress' })).toBe(false);
  });

  it('rejects null category (未对账 / 旧 server batch-status 404 降级) and non-terminal tasks', () => {
    // 未对账（null）→ 不可：不猜 issue 状态。
    expect(canRerunRemoteTask({ status: 'completed', issueStatusCategory: null })).toBe(false);
    expect(canRerunRemoteTask({ status: 'failed', issueStatusCategory: null })).toBe(false);
    // active 行（queued/running）不论 issue 状态都不给重跑入口。
    expect(canRerunRemoteTask({ status: 'queued', issueStatusCategory: 'todo' })).toBe(false);
    expect(canRerunRemoteTask({ status: 'running', issueStatusCategory: 'todo' })).toBe(false);
  });
});

// issue 状态标记谓词：终态 + todo/backlog（回退待办场景）；done/canceled/未对账不渲染。
describe('showsIssueStatusChip (issue 状态标记)', () => {
  it('shows the chip only for terminal tasks with a todo/backlog issue category', () => {
    expect(ISSUE_STATUS_CHIP_CATEGORIES).toEqual(['todo', 'backlog']);
    expect(showsIssueStatusChip({ status: 'completed', issueStatusCategory: 'todo' })).toBe(true);
    expect(showsIssueStatusChip({ status: 'failed', issueStatusCategory: 'backlog' })).toBe(true);
  });

  it('hides the chip for done (与任务终态一致，无提示价值), in_progress, canceled, null and active tasks', () => {
    expect(showsIssueStatusChip({ status: 'completed', issueStatusCategory: 'done' })).toBe(false);
    expect(showsIssueStatusChip({ status: 'completed', issueStatusCategory: 'in_progress' })).toBe(false);
    expect(showsIssueStatusChip({ status: 'completed', issueStatusCategory: 'canceled' })).toBe(false);
    expect(showsIssueStatusChip({ status: 'completed', issueStatusCategory: null })).toBe(false);
    expect(showsIssueStatusChip({ status: 'queued', issueStatusCategory: 'todo' })).toBe(false);
    expect(showsIssueStatusChip({ status: 'running', issueStatusCategory: 'backlog' })).toBe(false);
  });
});

async function renderBoard(props: {
  tasks: RemoteTaskVm[];
  busyTaskId?: string | null;
  onPrepare?: (t: RemoteTaskVm) => void;
  onCancel?: (t: RemoteTaskVm) => void;
  onRerun?: (t: RemoteTaskVm) => void;
  onRemove?: (t: RemoteTaskVm) => void;
  onSelectRun?: (p: string, t: string, r: string) => void;
}) {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  const onPrepare = props.onPrepare ?? vi.fn();
  const onCancel = props.onCancel ?? vi.fn();
  const onRerun = props.onRerun ?? vi.fn();
  const onRemove = props.onRemove ?? vi.fn();
  const onSelectRun = props.onSelectRun ?? vi.fn();
  await act(async () => {
    root.render(
      <RemoteTaskBoard
        tasks={props.tasks}
        busyTaskId={props.busyTaskId ?? null}
        onPrepare={onPrepare}
        onCancel={onCancel}
        onRerun={onRerun}
        onRemove={onRemove}
        onSelectRun={onSelectRun}
      />,
    );
  });
  return { container, onPrepare, onCancel, onRerun, onRemove, onSelectRun };
}

describe('RemoteTaskBoard render', () => {
  it('renders the 4 column headers and a task in each column', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'q', status: 'queued', title: 'Todo' }),
        task({ id: 'r', status: 'running', title: 'Doing' }),
        task({ id: 'c', status: 'completed', title: 'Done' }),
        task({ id: 'f', status: 'failed', title: 'Boom' }),
      ],
    });
    // 4 列头（status 标签走 i18n key，mock t 返回 key）。
    for (const status of BOARD_COLUMNS) {
      expect(container.textContent).toContain(`conversation.sidebar.remoteTasks.status.${status}`);
    }
    expect(container.textContent).toContain('Todo');
    expect(container.textContent).toContain('Doing');
    expect(container.textContent).toContain('Done');
    expect(container.textContent).toContain('Boom');
  });

  it('shows the empty hint for every column when there are no tasks', async () => {
    const { container } = await renderBoard({ tasks: [] });
    expect(container.textContent).toContain('remote.taskManagement.column.empty');
  });

  it('renders a prepare button only for queued tasks and forwards onPrepare', async () => {
    const onPrepare = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 'q', status: 'queued', title: 'Todo' })],
      onPrepare,
    });
    const claimBtn = container.querySelector('button[aria-label="conversation.sidebar.remoteTasks.executeTask"]') as HTMLButtonElement;
    expect(claimBtn).toBeTruthy();
    await act(async () => { claimBtn.click(); });
    expect(onPrepare).toHaveBeenCalledTimes(1);
    expect((onPrepare.mock.calls[0] as [RemoteTaskVm])[0].id).toBe('q');
  });

  it('renders a cancel button only for running tasks and forwards onCancel', async () => {
    const onCancel = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 'r', status: 'running', title: 'Doing' })],
      onCancel,
    });
    const cancelBtn = container.querySelector('button[aria-label="conversation.sidebar.remoteTasks.cancelTask"]') as HTMLButtonElement;
    expect(cancelBtn).toBeTruthy();
    await act(async () => { cancelBtn.click(); });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect((onCancel.mock.calls[0] as [RemoteTaskVm])[0].id).toBe('r');
  });

  it('makes a terminal task with a local run link clickable → onSelectRun', async () => {
    const onSelectRun = vi.fn();
    const { container } = await renderBoard({
      tasks: [
        task({
          id: 'c', status: 'completed', title: 'Done',
          projectId: 'proj-1', localTaskId: 't-1', runId: 'r-1',
        }),
      ],
      onSelectRun,
    });
    // 终态行整块内容包成 button：按文本定位。
    const btn = Array.from(container.querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').includes('Done'),
    ) as HTMLButtonElement;
    expect(btn).toBeTruthy();
    await act(async () => { btn.click(); });
    expect(onSelectRun).toHaveBeenCalledWith('proj-1', 't-1', 'r-1');
  });

  it('does not wrap a terminal task without a local run link in a click handler', async () => {
    const onSelectRun = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 'c', status: 'failed', title: 'NoLink' })],
      onSelectRun,
    });
    // 无 projectId/localTaskId/runId → 内容直接渲染为文本，非 button。
    const btn = Array.from(container.querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').includes('NoLink'),
    );
    expect(btn).toBeUndefined();
    expect(container.textContent).toContain('NoLink');
  });

  it('renders the issue-kind badge for dev/test/bug but not for general', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'd', status: 'queued', title: 'DevTask', kind: 'dev' }),
        task({ id: 't', status: 'queued', title: 'TestTask', kind: 'test', readiness: true }),
        task({ id: 'b', status: 'queued', title: 'BugTask', kind: 'bug' }),
        task({ id: 'g', status: 'queued', title: 'GeneralTask', kind: 'general' }),
      ],
    });
    expect(container.textContent).toContain('remote.taskManagement.issueKind.dev');
    expect(container.textContent).toContain('remote.taskManagement.issueKind.test');
    expect(container.textContent).toContain('remote.taskManagement.issueKind.bug');
    // general 不渲染类型徽标（仅有状态徽标）。
    expect(container.textContent).not.toContain('remote.taskManagement.issueKind.general');
  });

  it('marks a queued ready test task with the green ready badge and still allows execution', async () => {
    const onPrepare = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 't', status: 'queued', title: 'TestTask', kind: 'test', readiness: true })],
      onPrepare,
    });
    // 正向徽标 + 语义提示（Tooltip 文案）都在卡片上（仅提示，不阻断执行）。
    expect(container.textContent).toContain('remote.taskManagement.readiness.ready');
    expect(container.textContent).toContain('remote.taskManagement.readiness.readyHint');
    const claimBtn = container.querySelector('button[aria-label="conversation.sidebar.remoteTasks.executeTask"]') as HTMLButtonElement;
    expect(claimBtn).toBeTruthy();
    expect(claimBtn.disabled).toBe(false);
    await act(async () => { claimBtn.click(); });
    expect(onPrepare).toHaveBeenCalledTimes(1);
  });

  it('shows no readiness badge for a not-ready test task (未就绪不再负向标记，执行入口照常)', async () => {
    const onPrepare = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 't', status: 'queued', title: 'TestTask', kind: 'test', readiness: false })],
      onPrepare,
    });
    // 反转后未就绪 = 无徽标（无信号不标）；旧「未就绪」key 已删除。
    expect(container.textContent).not.toContain('remote.taskManagement.readiness');
    const claimBtn = container.querySelector('button[aria-label="conversation.sidebar.remoteTasks.executeTask"]') as HTMLButtonElement;
    expect(claimBtn).toBeTruthy();
    expect(claimBtn.disabled).toBe(false);
    await act(async () => { claimBtn.click(); });
    expect(onPrepare).toHaveBeenCalledTimes(1);
  });

  it('shows no readiness badge for a queued dev task even when the server reports readiness=true', async () => {
    // 非 test 不受就绪字段影响（就绪语义只属于 story 拆分的 test 行）。
    const { container } = await renderBoard({
      tasks: [task({ id: 'd', status: 'queued', title: 'DevTask', kind: 'dev', readiness: true })],
    });
    expect(container.textContent).not.toContain('remote.taskManagement.readiness');
  });

  // 待办列排序：已就绪优先靠前（与徽标同一用户逻辑），其余保持后端顺序。
  it('orders the queued column ready-first while other columns keep server order', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'plain-1', status: 'queued', title: 'PlainOne' }),
        task({ id: 'ready-1', status: 'queued', title: 'ReadyOne', kind: 'test', readiness: true }),
        task({ id: 'plain-2', status: 'queued', title: 'PlainTwo' }),
        task({ id: 'run-1', status: 'running', title: 'RunOne' }),
      ],
    });
    // 待办列内：已就绪的 ReadyOne 排最前，其余保持传入顺序。
    const titles = Array.from(container.querySelectorAll('section'))
      .find((s) => s.textContent?.includes('conversation.sidebar.remoteTasks.status.queued'))
      ?.textContent ?? '';
    const iReady = titles.indexOf('ReadyOne');
    const iPlain1 = titles.indexOf('PlainOne');
    const iPlain2 = titles.indexOf('PlainTwo');
    expect(iReady).toBeGreaterThanOrEqual(0);
    expect(iReady).toBeLessThan(iPlain1);
    expect(iPlain1).toBeLessThan(iPlain2);
  });

  // DPMS 待回传徽标：三源合成 dpmsLinkPending（VM 判定），看板只按标记渲染，全状态列适用。
  it('renders the red DPMS badge with its hint on dpmsLinkPending tasks', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'q', status: 'queued', title: 'QueuedDpms', dpmsLinkPending: true }),
        task({ id: 'r', status: 'running', title: 'RunningDpms', dpmsLinkPending: true }),
      ],
    });
    // 徽标 + 语义提示都在卡片上；queued 与 running 行都渲染（提醒覆盖任务全生命周期）。
    expect(container.textContent?.match(/remote\.taskManagement\.dpms\.badge/g)?.length).toBe(2);
    expect(container.textContent).toContain('remote.taskManagement.dpms.hint');
  });

  it('renders no DPMS badge when the link is reported or DPMS is not required', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'q', status: 'queued', title: 'PlainQueued' }),
        task({ id: 'c', status: 'completed', title: 'Reported', dpmsLinkPending: false }),
      ],
    });
    expect(container.textContent).not.toContain('remote.taskManagement.dpms.badge');
    expect(container.textContent).not.toContain('remote.taskManagement.dpms.hint');
  });

  it('renders task timestamps in the local timezone, not raw UTC', async () => {
    const ts = '2026-08-06T02:30:00Z';
    const { container } = await renderBoard({
      tasks: [task({ id: 'q', status: 'queued', title: 'Todo', lastActivityAt: ts })],
    });
    expect(container.textContent).toContain(formatLocalDateTime(ts));
    expect(container.textContent).not.toContain('2026-08-06T02:30:00Z');
  });

  // 移出列表（全状态可用）：纯本地视图过滤、刷新即恢复——此处固化「每列都有入口 + 转发 onRemove」。
  it('renders a remove button for every status and forwards onRemove', async () => {
    const onRemove = vi.fn();
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'q', status: 'queued', title: 'Todo' }),
        task({ id: 'r', status: 'running', title: 'Doing' }),
        task({ id: 'c', status: 'completed', title: 'Done' }),
        task({ id: 'f', status: 'failed', title: 'Boom' }),
      ],
      onRemove,
    });
    const removeButtons = container.querySelectorAll(
      'button[aria-label="conversation.sidebar.remoteTasks.removeTask"]',
    );
    // 四个 canonical 状态各一张卡片 → 四个移出入口。
    expect(removeButtons.length).toBe(4);
    await act(async () => { (removeButtons[2] as HTMLButtonElement).click(); });
    expect(onRemove).toHaveBeenCalledTimes(1);
    expect((onRemove.mock.calls[0] as [RemoteTaskVm])[0].id).toBe('c');
  });

  it('disables the remove button while the card is busy', async () => {
    const { container } = await renderBoard({
      tasks: [task({ id: 'q', status: 'queued', title: 'Todo' })],
      busyTaskId: 'q',
    });
    const removeBtn = container.querySelector(
      'button[aria-label="conversation.sidebar.remoteTasks.removeTask"]',
    ) as HTMLButtonElement;
    expect(removeBtn.disabled).toBe(true);
  });

  // 重新执行入口（multica 任务重试改造）：终态 + 对账回填的 issue 分类允许时渲染并转发 onRerun。
  it('renders a rerun button for a terminal task with an eligible issue category and forwards onRerun', async () => {
    const onRerun = vi.fn();
    const { container } = await renderBoard({
      tasks: [task({ id: 'c', status: 'completed', title: 'Done', issueStatusCategory: 'todo' })],
      onRerun,
    });
    const rerunBtn = container.querySelector(
      'button[aria-label="conversation.sidebar.remoteTasks.rerunTask"]',
    ) as HTMLButtonElement;
    expect(rerunBtn).toBeTruthy();
    expect(rerunBtn.disabled).toBe(false);
    await act(async () => { rerunBtn.click(); });
    expect(onRerun).toHaveBeenCalledTimes(1);
    expect((onRerun.mock.calls[0] as [RemoteTaskVm])[0].id).toBe('c');
  });

  it('renders no rerun button when the issue category is un-reconciled or ineligible', async () => {
    const { container } = await renderBoard({
      tasks: [
        // 未对账（旧 server / 对账前）：不猜 issue 状态 → 无入口。
        task({ id: 'a', status: 'completed', title: 'Unreconciled' }),
        // in_progress：issue 正被他端执行（multica 按 issue+agent 串行）→ 无入口。
        task({ id: 'b', status: 'failed', title: 'InProgressElsewhere', issueStatusCategory: 'in_progress' }),
        // active 行（queued/running）不论 issue 状态 → 无入口。
        task({ id: 'd', status: 'queued', title: 'Queued', issueStatusCategory: 'todo' }),
      ],
    });
    expect(container.querySelector('button[aria-label="conversation.sidebar.remoteTasks.rerunTask"]')).toBeNull();
  });

  it('disables the rerun button while the card is busy', async () => {
    const { container } = await renderBoard({
      tasks: [task({ id: 'c', status: 'failed', title: 'Boom', issueStatusCategory: 'done' })],
      busyTaskId: 'c',
    });
    const rerunBtn = container.querySelector(
      'button[aria-label="conversation.sidebar.remoteTasks.rerunTask"]',
    ) as HTMLButtonElement;
    expect(rerunBtn).toBeTruthy();
    expect(rerunBtn.disabled).toBe(true);
  });

  // rerun 产生的新任务（queued + rerunOfTaskId）渲染「重新执行」徽标（multica 归组语义的看板投影）。
  it('renders the rerun badge on a queued task carrying rerun lineage, not on plain queued tasks', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'n', status: 'queued', title: 'NewRun', rerunOfTaskId: 'src-1' }),
        task({ id: 'p', status: 'queued', title: 'PlainQueued' }),
      ],
    });
    expect(container.textContent).toContain('conversation.sidebar.remoteTasks.rerunBadge');
    // 仅 rerun 血缘行渲染徽标：两行都在板，但徽标 key 只出现一次（普通 queued 行不渲染）。
    expect(container.textContent?.match(/conversation\.sidebar\.remoteTasks\.rerunBadge/g)?.length).toBe(1);
  });

  // issue 状态标记（对账回填）：todo/backlog 分类文案；自定义状态名优先于分类文案。
  it('renders the issue-status chip with the category label for a todo task', async () => {
    const { container } = await renderBoard({
      tasks: [task({ id: 'c', status: 'completed', title: 'Done', issueStatusCategory: 'backlog' })],
    });
    expect(container.textContent).toContain('remote.taskManagement.issueStatus.backlog');
  });

  it('prefers the reconciled custom issue status name over the category label', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({
          id: 'c', status: 'failed', title: 'Boom',
          issueStatusCategory: 'todo', issueStatusName: '  待验证  ',
        }),
      ],
    });
    expect(container.textContent).toContain('待验证');
    expect(container.textContent).not.toContain('remote.taskManagement.issueStatus.todo');
  });

  it('renders no issue-status chip for done / un-reconciled terminal tasks', async () => {
    const { container } = await renderBoard({
      tasks: [
        task({ id: 'a', status: 'completed', title: 'DoneIssue', issueStatusCategory: 'done' }),
        task({ id: 'b', status: 'completed', title: 'Unreconciled' }),
      ],
    });
    expect(container.textContent).not.toContain('remote.taskManagement.issueStatus.todo');
    expect(container.textContent).not.toContain('remote.taskManagement.issueStatus.backlog');
  });
});
