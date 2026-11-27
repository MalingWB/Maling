/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const forgeGetSpecMock = vi.fn();
const forgeCompileMock = vi.fn();
const forgeStartTrialMock = vi.fn();
let settledListener: ((event: ForgeTurnSettledEventVm) => void) | null = null;
const graphViewRender = vi.hoisted(() => ({ count: 0 }));

vi.mock('../src/api', async () => {
  // 宿主面板(ConversationRunWorkspaceResourcePanel)还用到其余 API:保留真实模块,只替换铸造相关入口。
  const actual = await vi.importActual<typeof import('../src/api')>('../src/api');
  return {
    ...actual,
    forgeGetSpec: (...args: unknown[]) => forgeGetSpecMock(...args),
    forgeCompile: (...args: unknown[]) => forgeCompileMock(...args),
    forgeStartTrial: (...args: unknown[]) => forgeStartTrialMock(...args),
    subscribeForgeTurnSettled: (listener: (event: ForgeTurnSettledEventVm) => void) => {
      settledListener = listener;
      return Promise.resolve(() => {});
    },
  };
});

vi.mock('@/components/GraphView', () => ({
  GraphView: ({graph}: {graph: import('../src/types').GraphVm}) => {
    graphViewRender.count += 1;
    return <div data-testid="graph-view">{graph.nodes.map(n => <span key={n.id} data-blueprint-node={n.id}>{n.label}</span>)}{graph.edges.map((e, i) => <span key={i} data-route={`${e.from}/${e.label}/${e.to}`} />)}</div>;
  },
}));

vi.mock('@/components/acp/ACPChatDialog', () => ({
  RawFrameViewer: () => null,
  SystemPromptPanel: () => null,
}));

import { ForgePreviewPanel } from '@/components/workspace/ForgePreviewPanel';
import { ConversationRunWorkspaceResourcePanel } from '@/components/workspace/ConversationRunWorkspaceResourcePanel';
import type { ForgePreviewWorkspaceResource } from '@/components/workspace/right-workspace-context';
import type { ForgeTurnSettledEventVm } from '../src/api/client';
import type { AgentRegistryVm, ConversationRunVm, ForgeCompileOutcomeVm, ForgeErrorVm, ForgeSpecVm, ForgeStageVm } from '../src/types';
import '@/i18n';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

// GraphView(@xyflow)依赖 ResizeObserver;jsdom 环境补桩。
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
if (!('ResizeObserver' in globalThis)) {
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = ResizeObserverStub;
}

/**
 * 铸造预览 Tab(方案 §5.2)契约:Spec 是会话唯一事实源,Tab 是其只读投影——存档为空
 * 显示空态引导;有 Spec 时阶段四要素与未收敛项必须完整可见(角色/产物形态/成功条件/
 * 失败策略);编译未通过时由这里驱动自动修正回路。
 */
const resource: ForgePreviewWorkspaceResource = {
  kind: 'forge-preview',
  key: 'forge-preview:project-a:task-001',
  scopeKey: 'conversation:project-a:task-001:run-1',
  title: '铸造预览',
  attention: false,
  projectId: 'project-a',
  taskId: 'task-001',
};

const stage: ForgeStageVm = {
  id: 's1',
  role: 'dev',
  goal: '<输入>需求\n<产出>代码\n<边界>不做部署\n<验收>测试通过',
  artifact: 'code',
  outputKind: 'json',
  schemaHint: { ok: 'boolean' },
  successCondition: '$.ok == true',
  onFailure: 'retry',
  manualCheck: false,
};

const sampleSpec: ForgeSpecVm = {
  specVersion: '0.1',
  name: '登录功能流水线',
  intent: '实现登录并验证',
  sourceSkills: [],
  workflows: [
    {
      name: '登录功能流水线',
      stages: [
        stage,
        {
          id: 's2',
          role: { custom: { profileName: '联调' } },
          goal: '<输入>接口\n<产出>联调报告\n<边界>不改代码\n<验收>报告齐全',
          outputKind: 'none',
          onFailure: 'manual-check',
          manualCheck: true,
        },
      ],
    },
  ],
  openQuestions: ['失败后是否自动开新轮?'],
};
const converged: ForgeSpecVm = { ...sampleSpec, openQuestions: [] };

const templates = { version: '0.1', lastUsedTemplateId: 'wf-1', lastCreatedWorkflow: null, templates: [] };
const trial = { templateId: 'wf-1', revision: 'review-A', group: null };
const savedCreated: ForgeCompileOutcomeVm = { status: 'saved', trial, templateIds: ['wf-1'], createdCount: 1, updatedCount: 0, templates };
const savedUpdated: ForgeCompileOutcomeVm = { status: 'saved', trial, templateIds: ['wf-1'], createdCount: 0, updatedCount: 1, templates };

const missingBoundary = { code: 'forge/missing-goal-element', context: { stage: 's1', element: 'boundary' } };
const repairable = (
  attempt: number,
  errors: ForgeErrorVm[] = [missingBoundary],
): Extract<ForgeCompileOutcomeVm, { status: 'repairable' }> => ({
  status: 'repairable',
  errors,
  repairPrompt: `编译未通过，正在修正 ${attempt}/3\n\n<hidden data-gold-band-hidden="true" title="Forge Spec 修正">修复要求</hidden>`,
});

const settled = (outcome: 'completed' | 'cancelled' | 'failed', specUpdated: boolean, promptId: string | null = null): ForgeTurnSettledEventVm => ({
  projectId: 'project-a',
  taskId: 'task-001',
  runId: 'run-1',
  roundId: 'round-1',
  nodeId: 'node-1',
  attemptId: 'attempt-1',
  promptId,
  outcome,
  specUpdated,
});

/** 等待条件成立：每轮让已排队的微任务与 effect 跑完。 */
async function until(condition: () => boolean, rounds = 20) {
  for (let round = 0; round < rounds; round += 1) {
    if (condition()) return;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  if (!condition()) throw new Error('condition not reached');
}

const button = (host: HTMLElement, text: string) =>
  [...host.querySelectorAll('button')].find((candidate) => candidate.textContent?.includes(text));

describe('ForgePreviewPanel', () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    forgeGetSpecMock.mockReset();
    forgeCompileMock.mockReset();
    forgeStartTrialMock.mockReset();
    settledListener = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it('keeps the newer archived draft when an older compile response arrives later', async () => {
    const old = { ...converged, revision: 4, name: 'Earlier' };
    const current = { ...converged, revision: 6, name: 'Latest' };
    forgeGetSpecMock.mockResolvedValueOnce(old).mockResolvedValueOnce(current);
    let finish: (value: ForgeCompileOutcomeVm) => void = () => {};
    forgeCompileMock.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!host.querySelector('[data-forge-save]'));
    await act(async () => (host.querySelector('[data-forge-save]') as HTMLElement).click());
    await act(async () => settledListener?.(settled('completed', true)));
    await until(() => host.querySelector('[data-forge-workflow-name]')?.textContent === 'Latest');
    await act(async () => finish({ ...savedCreated, spec: { ...old, revision: 5 } }));
    expect(host.querySelector('[data-forge-workflow-name]')?.textContent).toBe('Latest');
    expect(host.querySelector('[data-forge-saved]')).toBeNull();
  });

  it('can save a healthy workflow as a named copy without first failing an update', async () => {
    const workflow = { ...converged.workflows![0], id: 'delivery-draft', name: '交付前并行检查' };
    const draft = { ...converged, revision: 2, name: workflow.name, workflows: [workflow] };
    const saved = { ...draft, revision: 3 };
    const name = '交付前并行检查（无障碍版）';
    const copied = { ...saved, name, revision: 4, workflows: [{ ...workflow, name }] };
    forgeGetSpecMock.mockResolvedValue(draft);
    forgeCompileMock.mockResolvedValueOnce({ ...savedCreated, spec: saved })
      .mockResolvedValueOnce({ ...savedCreated, templateIds: ['copy-id'], spec: copied });
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!host.querySelector('[data-forge-save]'));
    await act(async () => host.querySelector<HTMLButtonElement>('[data-forge-save]')!.click());
    const openCopy = button(host, '另存为新工作流');
    expect(openCopy).toBeDefined();
    await act(async () => openCopy!.click());
    expect(forgeCompileMock).toHaveBeenCalledTimes(1);
    const form = host.querySelector<HTMLFormElement>('[data-forge-copy]')!;
    const input = form.querySelector('input')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, name);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => form.querySelector<HTMLButtonElement>('button[type="submit"]')!.click());
    expect(forgeCompileMock).toHaveBeenLastCalledWith(resource.projectId, saved, resource.taskId, 1, { workflowId: workflow.id, name });
    expect(host.querySelector('[data-forge-copy]')).toBeNull();
  });

  it('offers an explicit copy only for the unavailable bound workflow', async () => {
    const workflow = { ...converged.workflows![0], id: 'draft-workflow' };
    const spec = { ...converged, revision: 7, workflows: [workflow] };
    forgeGetSpecMock.mockResolvedValue(spec);
    forgeCompileMock.mockRejectedValueOnce({ code: 'forge/target-unavailable', params: { workflow: workflow.id } })
      .mockResolvedValueOnce({ ...savedCreated, spec: { ...spec, revision: 8 } });
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!host.querySelector('[data-forge-save]'));
    await act(async () => (host.querySelector('[data-forge-save]') as HTMLElement).click());
    expect(forgeCompileMock).toHaveBeenCalledTimes(1);
    const input = host.querySelector('[data-forge-copy] input') as HTMLInputElement;
    expect(input).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'Recovered');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => (host.querySelector('[data-forge-copy] button') as HTMLElement).click());
    expect(forgeCompileMock).toHaveBeenLastCalledWith(resource.projectId, spec, resource.taskId, 1, { workflowId: workflow.id, name: 'Recovered' });
    expect(host.querySelector('[data-forge-copy]')).toBeNull();
  });

  it('shows the empty guidance while the session has produced no spec archive', async () => {
    forgeGetSpecMock.mockResolvedValue(null);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => host.textContent?.includes('对话开始梳理流程') ?? false);
  });

  it('renders stage contracts and open questions from the archived spec', async () => {
    forgeGetSpecMock.mockResolvedValue(sampleSpec);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => host.textContent?.includes('实现登录并验证') ?? false);

    // goal 四要素解析为结构化行:要素标签与内容分行呈现(角标形式样例)。
    const rows = [...host.querySelectorAll('dl div')];
    expect(rows.length).toBeGreaterThanOrEqual(4);
    expect(rows[0].textContent).toContain('输入');
    expect(rows[0].textContent).toContain('需求');
    expect(rows[3].textContent).toContain('验收');
    expect(rows[3].textContent).toContain('测试通过');
    // 产物契约与失败策略投影。
    expect(host.textContent).not.toContain('JSON 产物');
    await act(async () => button(host, '技术细节')?.click());
    expect(host.textContent).toContain('JSON 产物');
    expect(host.textContent!.replace(/\s/g, '')).toContain('$.ok==true');
    expect(host.textContent).toContain('自动重试');
    expect(host.textContent).toContain('人工复核');
    // custom 角色显示 profile 名(后端线上的字段是 profileName)。
    expect(host.textContent).toContain('联调');
    expect(host.textContent).not.toContain('undefined');
    // 未收敛项可见。
    expect(host.textContent).toContain('失败后是否自动开新轮?');
    // 蓝图画布必须有确定高度：GraphView 跟随父容器高度，滚动区里的父容器若不给出
    // 高度，画布会塌缩成一条线（真机：蓝图看起来像一条分割线）。
    const graphFrame = host.querySelector('[data-forge-spec-graph]') as HTMLElement;
    // 两个阶段 + 起止端点按成功路径纵向堆叠（不计失败回路）。
    expect(parseFloat(graphFrame.style.height)).toBe(388);
  });

  it('labels structured goals without parsing the author’s language and preserves projected prose', async () => {
    const goal = { input: 'Requisitos', output: 'Relatório', boundary: 'Não implantar', acceptance: 'Verificações aprovadas' };
    const original = 'Input: an editor note, not a generated goal.';
    forgeGetSpecMock.mockResolvedValue({ ...sampleSpec, workflows: [{ ...sampleSpec.workflows![0], stages: [
      { ...stage, id: 'typed', goal }, { ...stage, id: 'projected', role: 'unassigned', goal: { original } },
    ] }] });
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => host.textContent?.includes('Verificações aprovadas') ?? false);
    const rows = [...host.querySelectorAll('[data-forge-stage-card="typed"] dd')];
    expect(rows.map(row => row.textContent)).toEqual(Object.values(goal));
    expect(host.querySelector('[data-forge-stage-card="projected"] p')?.textContent).toBe(original);
    expect(host.querySelector('[data-forge-stage-card="projected"]')?.textContent).toContain('无角色');
  });

  it('shows distinct step names on the blueprint and cards when workers share an artifact key', async () => {
    const ids = ['api-check', 'ui-check', 'report'];
    forgeGetSpecMock.mockResolvedValue({ ...converged, workflows: [{ name: '交付前并行检查',
      stages: ids.map(id => ({ ...stage, id, artifact: 'implementation-result' })),
      parallelGroups: [{ id: 'checks', branches: [{ id: 'api', stages: [ids[0]] }, { id: 'ui', stages: [ids[1]] }], maxParallel: 2, onFailure: 'end' }],
    }] });
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!host.querySelector('[data-blueprint-node="report"]'));
    for (const id of ids) {
      expect(host.querySelector(`[data-blueprint-node="${id}"]`)?.textContent).toBe(id);
      expect(host.querySelector(`[data-forge-stage-card="${id}"] button`)?.textContent).toContain(id);
    }
    expect(host.textContent).not.toContain('implementation-result');
    await act(async () => button(host, '技术细节')?.click());
    expect(host.textContent).toContain('implementation-result');
  });

  it('previews a fork, ordered branch and shared join without connecting siblings', async () => {
    forgeGetSpecMock.mockResolvedValue({...converged, workflows: [{name: 'fixture',
      stages: ['api', 'tests', 'ui', 'report'].map(id => ({...stage, id})),
      parallelGroups: [{id: 'fork', branches: [{id: 'backend', stages: ['api', 'tests']}, {id: 'interface', stages: ['ui']}], maxParallel: 1, onFailure: 'end'}],
    }]});
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!host.querySelector('[data-route]'));
    const routes = [...host.querySelectorAll('[data-route]')].map(e => e.getAttribute('data-route')!);
    const destination = (id: string) => routes.filter(r => r.startsWith(`${id}/success/`)).map(r => r.split('/')[2]);
    expect(destination('tests')).not.toContain('ui');
    expect(destination('api')).toEqual(['tests']);
    expect(destination('tests')).toEqual(destination('ui'));
    expect(destination(destination('tests')[0])).toEqual(['report']);
    expect(routes.some(r => r.endsWith('/success/api') && routes.includes(r.replace('/success/api', '/success/ui')))).toBe(true);
  });

  it('structures the natural colon form of goal elements into labeled rows', async () => {
    // 真机验收回归:模型常以「输入：…；产出：…」段落表达,同样结构化展示。
    const spec: ForgeSpecVm = {
      ...sampleSpec,
      workflows: [{
        name: '文章摘要事实核查',
        stages: [{
          ...stage,
          goal: '输入：用户提供的文章原文；产出：中文 markdown 要点总结；边界：只提炼要点，不做事实真伪判断，不改写原文观点；验收：要点覆盖文章全部核心信息，无遗漏。',
          artifact: '要点总结',
          outputKind: 'markdown',
          schemaHint: null,
          successCondition: null,
        }],
      }],
      openQuestions: [],
    };
    forgeGetSpecMock.mockResolvedValue(spec);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => host.textContent?.includes('文章摘要事实核查') ?? false);

    const rows = [...host.querySelectorAll('dl div')];
    expect(rows).toHaveLength(4);
    expect(rows[0].textContent).toContain('用户提供的文章原文');
    expect(rows[1].textContent).toContain('中文 markdown 要点总结');
    expect(rows[2].textContent).toContain('不做事实真伪判断');
    expect(rows[3].textContent).toContain('无遗漏');
    // markdown 产物行可见,无成功条件时不渲染该码片段。
    await act(async () => button(host, '技术细节')?.click());
    expect(host.textContent).toContain('Markdown 产物');
  });

  it('distinguishes a failed archive read from an empty archive, and retries in place', async () => {
    forgeGetSpecMock.mockRejectedValueOnce(new Error('io'));
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => host.textContent?.includes('生成草稿读取失败') ?? false);
    // 读取失败不能伪装成“会话还没有草稿”。
    expect(host.textContent).toContain('生成草稿读取失败');
    expect(host.textContent).not.toContain('对话开始梳理流程');

    forgeGetSpecMock.mockResolvedValue(converged);
    await act(async () => button(host, '重试')?.click());
    await until(() => host.textContent?.includes('实现登录并验证') ?? false);
    expect(host.textContent).not.toContain('生成草稿读取失败');
  });

  it('blocks compile while open questions remain, then compiles and offers the trial entry', async () => {
    forgeGetSpecMock.mockResolvedValue(sampleSpec);
    forgeCompileMock.mockResolvedValue(savedCreated);
    forgeStartTrialMock.mockResolvedValue({ projectId: 'project-a', taskId: 'task-trial', title: '试运行' });
    const onTemplatesChange = vi.fn();
    const onTrialStarted = vi.fn();
    const props = { onWorkflowTemplatesChange: onTemplatesChange, onTrialSessionStarted: onTrialStarted };
    await act(async () => root.render(<ForgePreviewPanel resource={resource} {...props} />));
    await until(() => host.querySelector('[data-forge-open-questions]') !== null);

    // 未收敛(openQuestions 非空):编译禁用,试运行不出现。
    expect(button(host, '保存为工作流')).toBeUndefined();
    expect(button(host, '试运行第一步')).toBeUndefined();

    // 收敛后:卸载重挂载(新会话轮/重开 Tab 的等价形态),编译可用,调用
    // forgeCompile 并刷新模板库;试运行入口出现。
    forgeGetSpecMock.mockResolvedValue(converged);
    await act(async () => root.render(null));
    await act(async () => root.render(<ForgePreviewPanel resource={resource} {...props} />));
    await until(() => button(host, '保存为工作流')?.disabled === false);
    await act(async () => button(host, '保存为工作流')?.click());
    await until(() => host.textContent?.includes('已创建 1 个工作流模板') ?? false);
    expect(forgeCompileMock).toHaveBeenCalledWith('project-a', converged, 'task-001', 1);
    expect(onTemplatesChange).toHaveBeenCalledWith(templates);

    await act(async () => button(host, '试运行第一步')?.click());
    expect(forgeStartTrialMock).toHaveBeenCalledWith('project-a', 'wf-1', trial.revision, converged.intent);
    expect(onTrialStarted).toHaveBeenCalledWith('project-a', 'task-trial');

    expect(host.querySelector('[data-forge-saved]')).not.toBeNull();
    expect(host.textContent).not.toContain('升级对照');
  });

  it('discloses saved entry group trial scope before starting any branch', async () => {
    const workflow = {entry: 'gate', nodes: [{type: 'parallel-group', id: 'gate', maxParallel: 1,
      branches: [{id: 'api', stages: ['api']}, {id: 'ui', stages: ['ui']}]}]};
    forgeGetSpecMock.mockResolvedValue(converged);
    forgeCompileMock.mockResolvedValue({...savedCreated, trial: {...trial, group: {branches: 2, cap: 1}}, templates:{...templates, templates:[{id:'wf-1', name:'saved', workflow}]}});
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => !!button(host, '保存为工作流'));
    await act(async () => button(host, '保存为工作流')?.click());
    await until(() => !!host.querySelector('[data-forge-saved]'));
    expect(forgeStartTrialMock).not.toHaveBeenCalled();
    expect(host.querySelector('[data-forge-group-trial]')?.textContent).toContain('2 个分支');
    expect(host.querySelector('[data-forge-group-trial]')?.textContent).toContain('最多同时运行 1 个');
    await act(async () => button(host, '试运行首个并行组')?.click());
    expect(forgeStartTrialMock).toHaveBeenCalledWith('project-a', 'wf-1', trial.revision, converged.intent);
  });

  it('refreshes a changed trial disclosure without starting until a second click', async () => {
    const first = { templateId: 'wf-1', revision: 'review-A', group: { branches: 2, cap: 1 } };
    const latest = { ...first, revision: 'review-B', group: { branches: 3, cap: 2 } };
    const workflow = { entry: 'gate', nodes: [{ type: 'parallel-group', id: 'gate', maxParallel: 1, branches: [{}, {}] }] };
    forgeGetSpecMock.mockResolvedValue(converged);
    forgeCompileMock.mockResolvedValue({ ...savedCreated, trial: first, templates: { ...templates, templates: [{ id: first.templateId, name: 'saved', workflow }] } });
    forgeStartTrialMock.mockRejectedValueOnce({ code: 'forge/stale-trial', params: { current: latest } })
      .mockResolvedValueOnce({ projectId: resource.projectId, taskId: 'trial-b' });
    const started = vi.fn();
    await act(async () => root.render(<ForgePreviewPanel resource={resource} onTrialSessionStarted={started} />));
    await act(async () => button(host, '保存为工作流')!.click());
    expect(host.querySelector('[data-forge-group-trial]')?.textContent).toContain('2 个分支');
    await act(async () => button(host, '试运行首个并行组')!.click());
    expect(forgeStartTrialMock.mock.calls).toEqual([[resource.projectId, first.templateId, first.revision, converged.intent]]);
    expect(started).not.toHaveBeenCalled();
    expect(host.querySelector('[data-forge-group-trial]')?.textContent).toContain('3 个分支');
    expect(host.querySelector('[data-forge-group-trial]')?.textContent).toContain('最多同时运行 2 个');
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('重新确认');
    await act(async () => button(host, '试运行首个并行组')!.click());
    expect(forgeStartTrialMock.mock.calls[1]).toEqual([resource.projectId, latest.templateId, latest.revision, converged.intent]);
    expect(started).toHaveBeenCalledExactlyOnceWith(resource.projectId, 'trial-b');
  });

  it('sends only the chosen recommendation and waits for the archived answer', async () => {
    const question = { question: '测试失败后去哪里？', recommendation: '回到开发' };
    forgeGetSpecMock.mockResolvedValue({ ...sampleSpec, openQuestions: [question] });
    const submit = vi.fn().mockResolvedValue('accepted');
    await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
    await until(() => !!button(host, '采用推荐'));
    await act(async () => button(host, '采用推荐')?.click());
    expect(submit).toHaveBeenCalledTimes(1);
    expect(submit.mock.calls[0][0]).toContain(question.recommendation);
    expect(host.querySelector('[data-forge-adopt-pending]')).not.toBeNull();
    expect(button(host, '保存为工作流')).toBeUndefined();
    forgeGetSpecMock.mockResolvedValue(converged);
    await act(async () => settledListener?.(settled('completed', true)));
    await until(() => !!button(host, '保存为工作流'));
    expect(host.querySelector('[data-forge-open-questions]')).toBeNull();
  });

  it('reports an in-place template update instead of a created one', async () => {
    forgeGetSpecMock.mockResolvedValue(converged);
    forgeCompileMock.mockResolvedValue(savedUpdated);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => button(host, '保存为工作流')?.disabled === false);
    await act(async () => button(host, '保存为工作流')?.click());
    await until(() => host.textContent?.includes('已更新原工作流模板') ?? false);
    expect(host.textContent).not.toContain('已创建');
  });

  it('localizes a command failure instead of printing the raw error', async () => {
    forgeGetSpecMock.mockResolvedValue(converged);
    forgeCompileMock.mockRejectedValue({ code: 'workspace.not-found', params: { projectId: 'project-a' } });
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    await until(() => button(host, '保存为工作流')?.disabled === false);
    await act(async () => button(host, '保存为工作流')?.click());
    await until(() => host.querySelector('[role="alert"]') !== null);
    expect(host.textContent).not.toContain('workspace.not-found');
  });

  describe('automatic repair', () => {
    function pendingSubmission() {
      let acknowledge!: (result: 'accepted' | 'busy') => void;
      const submit = vi.fn((_prompt: string, _promptId?: string) => new Promise<'accepted' | 'busy'>(resolve => { acknowledge = resolve; }));
      return { submit, ack: (result: 'accepted' | 'busy') => acknowledge(result), id: () => submit.mock.lastCall![1]! };
    }

    it.each(['completed', 'cancelled', 'failed'] as const)('settles an early %s repair only after its acknowledgment', async outcome => {
      const send = pendingSubmission();
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValueOnce(repairable(1)).mockResolvedValueOnce(savedCreated);
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={send.submit} />));
      await act(async () => button(host, '保存为工作流')!.click());
      const terminal = { ...settled(outcome, true), promptId: send.id() };
      await act(async () => {
        settledListener!(terminal);
        settledListener!({ ...terminal, outcome: 'failed', promptId: 'previous' });
      });
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
      await act(async () => send.ack('accepted'));
      if (outcome === 'completed') {
        expect(forgeCompileMock).toHaveBeenCalledTimes(2);
        expect(host.querySelector('[data-forge-saved]')).not.toBeNull();
      } else {
        expect(button(host, '保存为工作流')).toBeDefined();
        expect(host.querySelector('[data-forge-repair-status]')?.textContent).toContain(outcome === 'cancelled' ? '被中断' : '执行失败');
      }
      await act(async () => settledListener!(terminal));
      expect(forgeCompileMock).toHaveBeenCalledTimes(outcome === 'completed' ? 2 : 1);
    });

    it('ignores previous turn terminals after acceptance and recompiles only its own turn', async () => {
      const send = pendingSubmission();
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValueOnce(repairable(1)).mockResolvedValueOnce(savedCreated);
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={send.submit} />));
      await act(async () => button(host, '保存为工作流')!.click());
      await act(async () => send.ack('accepted'));
      await act(async () => settledListener!({ ...settled('completed', true), promptId: 'previous' }));
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
      await act(async () => settledListener!({ ...settled('completed', true), promptId: send.id() }));
      expect(forgeCompileMock).toHaveBeenCalledTimes(2);
      expect(host.querySelector('[data-forge-saved]')).not.toBeNull();
    });

    it.each(['missing identity', 'incomplete reply'])('returns repair to the user on %s', async reason => {
      const send = pendingSubmission();
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={send.submit} />));
      await act(async () => button(host, '保存为工作流')!.click());
      await act(async () => send.ack('accepted'));
      await act(async () => settledListener!({
        ...settled('completed', false, reason === 'missing identity' ? null : send.id()),
        error: { code: 'forge/turn-unconfirmed', detail: '' },
      }));
      expect(host.querySelector('[data-forge-repair-status]')?.textContent).toContain('请在对话中确认草稿后重新编译');
      expect(button(host, '保存为工作流')?.disabled).toBe(false);
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
    });

    it('does not let duplicate or unrelated terminals supersede the repair archive read', async () => {
      const send = pendingSubmission();
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValueOnce(repairable(1)).mockResolvedValueOnce(savedCreated);
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={send.submit} />));
      await act(async () => button(host, '保存为工作流')!.click());
      await act(async () => send.ack('accepted'));
      let complete!: (spec: ForgeSpecVm) => void;
      const archive = new Promise<ForgeSpecVm>(resolve => { complete = resolve; });
      forgeGetSpecMock.mockReturnValue(archive);
      const terminal = settled('completed', true, send.id());
      await act(async () => settledListener!(terminal));
      await act(async () => { settledListener!(terminal); settledListener!(settled('completed', true, 'previous')); });
      await act(async () => complete(converged));
      expect(forgeCompileMock).toHaveBeenCalledTimes(2);
      expect(forgeGetSpecMock).toHaveBeenCalledTimes(2); // initial read plus one post-repair read
      expect(host.querySelector('[data-forge-saved]')).not.toBeNull();
    });

    it('discards an aborted repair acknowledgment and a buffered terminal rejected as busy', async () => {
      const first = pendingSubmission(), second = pendingSubmission();
      const submit = vi.fn().mockImplementationOnce(first.submit).mockImplementationOnce(second.submit).mockResolvedValue('busy');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await act(async () => button(host, '保存为工作流')!.click());
      await act(async () => button(host, '停止修正')!.click());
      await act(async () => button(host, '保存为工作流')!.click());
      await act(async () => first.ack('accepted'));
      await act(async () => settledListener!({ ...settled('completed', true), promptId: first.id() }));
      expect(forgeCompileMock).toHaveBeenCalledTimes(2);
      await act(async () => settledListener!({ ...settled('completed', true), promptId: second.id() }));
      await act(async () => second.ack('busy'));
      expect(host.textContent).toContain('对话还在回复中');
      expect(forgeCompileMock).toHaveBeenCalledTimes(2);
    });
    it('sends the repair prompt through the conversation, waits for the turn, then recompiles', async () => {
      const submit = vi.fn().mockResolvedValue('accepted');
      const fixed: ForgeSpecVm = {
        ...converged,
        workflows: [{ ...converged.workflows![0], stages: [{ ...stage, goal: '<输入>需求\n<产出>代码\n<边界>不做部署\n<验收>测试通过' }, converged.workflows![0].stages[1]] }],
      };
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValueOnce(repairable(1)).mockResolvedValueOnce(savedCreated);
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);

      await act(async () => button(host, '保存为工作流')?.click());

      // 修正提示词原样交给对话栏发出(首行可见,其余在隐藏区段内)。
      await until(() => submit.mock.calls.length === 1);
      expect(submit).toHaveBeenCalledWith(repairable(1).repairPrompt, expect.stringMatching(/^acp-prompt-/));
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
      await until(() => host.textContent?.includes('正在自动修正（第 1/3 轮）') ?? false);
      // 未解决的问题按草稿序号本地化列出;进行中只能停止,不能再点编译。
      expect(host.textContent).toContain('阶段「第 1 个阶段」的目标缺少「边界」');
      expect(button(host, '保存为工作流')).toBeUndefined();
      expect(button(host, '停止修正')).toBeTruthy();

      // 这一轮收口且带来了增量:存档已在后端更新,重读后用新存档重新编译。
      forgeGetSpecMock.mockResolvedValue(fixed);
      await act(async () => settledListener?.(settled('completed', true, submit.mock.lastCall![1])));
      await until(() => host.textContent?.includes('已创建 1 个工作流模板') ?? false);
      expect(forgeCompileMock).toHaveBeenLastCalledWith('project-a', fixed, 'task-001', 2);
      expect(host.textContent).not.toContain('正在自动修正');
      expect(host.textContent).not.toContain('以下问题尚未解决');
      expect(button(host, '停止修正')).toBeUndefined();
    });

    it('gives up after three repair rounds and keeps the remaining issues visible', async () => {
      const submit = vi.fn().mockResolvedValue('accepted');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockReset();
      // 四次编译:前三次各触发一轮修正,第四次决定不再发第四轮;之后用户再编译回到第一轮。
      forgeCompileMock.mockResolvedValueOnce(repairable(1)).mockResolvedValueOnce(repairable(2))
        .mockResolvedValueOnce(repairable(3)).mockResolvedValueOnce(repairable(4))
        .mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => submit.mock.calls.length === 1);

      for (let round = 1; round <= 3; round += 1) {
        await act(async () => settledListener?.(settled('completed', true, submit.mock.lastCall![1])));
        await until(() => forgeCompileMock.mock.calls.length === round + 1);
      }
      await until(() => host.textContent?.includes('已自动修正 3 轮仍未通过') ?? false);

      // 三轮修正、四次编译(最后一次决定不再发第四轮)。
      expect(submit).toHaveBeenCalledTimes(3);
      expect(submit.mock.calls.map(([prompt]) => prompt.split('\n')[0])).toEqual([
        '编译未通过，正在修正 1/3', '编译未通过，正在修正 2/3', '编译未通过，正在修正 3/3',
      ]);
      expect(forgeCompileMock.mock.calls.map(([, , , attempt]) => attempt)).toEqual([1, 2, 3, 4]);
      expect(host.textContent).toContain('以下问题尚未解决');
      expect(button(host, '停止修正')).toBeUndefined();

      // 停止后用户再次编译:回路从起点重新开始(而不是卡在上一轮的标记上)。
      forgeCompileMock.mockResolvedValue(repairable(1));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => submit.mock.calls.length === 4);
      expect(host.textContent).not.toContain('已自动修正 3 轮仍未通过');
    });

    it('hands the conversation back when the model returns open questions', async () => {
      const submit = vi.fn().mockResolvedValue('accepted');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValueOnce(
        repairable(1, [{ code: 'forge/unresolved-questions', context: { items: '产物格式未定' } }]),
      );
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());

      await until(() => host.textContent?.includes('还有问题需要你来确认') ?? false);
      // 需要用户决定的问题不自动修:不发修正轮。
      expect(submit).not.toHaveBeenCalled();
      expect(host.textContent).toContain('还有问题待对话确认');
    });

    it('stops with a next step when the session is busy or the prompt is rejected', async () => {
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      const busy = vi.fn().mockResolvedValue('busy');
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={busy} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => host.textContent?.includes('对话还在回复中') ?? false);
      expect(busy).toHaveBeenCalledTimes(1);

      await act(async () => root.render(null));
      // 没有对话栏可发(宿主未提供出口)时同样不空转。
      forgeCompileMock.mockResolvedValueOnce(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => host.textContent?.includes('修正请求没有发出去') ?? false);
    });

    it('stops on request and ignores the turn that finishes afterwards', async () => {
      const submit = vi.fn().mockResolvedValue('accepted');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => button(host, '停止修正') !== undefined);

      await act(async () => button(host, '停止修正')?.click());
      expect(host.textContent).toContain('已停止自动修正');
      await act(async () => settledListener?.(settled('completed', true, submit.mock.lastCall![1])));
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
      expect(host.textContent).toContain('已停止自动修正');
    });

    it('does not recompile when the repair turn is cancelled or brings no delta', async () => {
      const submit = vi.fn().mockResolvedValue('accepted');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => button(host, '停止修正') !== undefined);
      await act(async () => settledListener?.(settled('cancelled', false, submit.mock.lastCall![1])));
      await until(() => host.textContent?.includes('自动修正被中断') ?? false);

      await act(async () => root.render(null));
      await act(async () => root.render(<ForgePreviewPanel key="second" resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => button(host, '停止修正') !== undefined);
      await act(async () => settledListener?.(settled('completed', false, submit.mock.lastCall![1])));
      await until(() => host.textContent?.includes('没有产生可用的调整') ?? false);
      expect(forgeCompileMock).toHaveBeenCalledTimes(2);
    });

    it('stops with a clear reason when the repair turn yields a delta that cannot be applied', async () => {
      // 真机回归：模型输出的增量 JSON 漏了右括号。后端带着原因收口，回路必须停下并
      // 说明“没有应用”——此前收口信号缺失，面板会永远停在“正在修正”。
      const submit = vi.fn().mockResolvedValue('accepted');
      forgeGetSpecMock.mockResolvedValue(converged);
      forgeCompileMock.mockResolvedValue(repairable(1));
      await act(async () => root.render(<ForgePreviewPanel resource={resource} onSubmitPrompt={submit} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      await act(async () => button(host, '保存为工作流')?.click());
      await until(() => button(host, '停止修正') !== undefined);

      await act(async () => settledListener?.({
        ...settled('completed', false, submit.mock.lastCall![1]),
        error: { code: 'forge/invalid-spec', detail: 'EOF while parsing an object at line 20 column 0' },
      }));

      await until(() => host.textContent?.includes('增量格式有误') ?? false);
      expect(host.textContent).not.toContain('正在自动修正');
      expect(host.textContent).not.toContain('没有产生可用的调整');
      // 没有可重新编译的东西：不再调用 forge_compile，也不再自动发一轮。
      expect(forgeCompileMock).toHaveBeenCalledTimes(1);
      expect(submit).toHaveBeenCalledTimes(1);
      // 用户可以重新发起。
      await until(() => button(host, '保存为工作流')?.disabled === false);
    });

    it('surfaces an unapplied delta from an ordinary conversation turn', async () => {
      forgeGetSpecMock.mockResolvedValue(converged);
      await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
      await until(() => button(host, '保存为工作流')?.disabled === false);
      expect(host.querySelector('[data-forge-delta-notice]')).toBeNull();

      // 回合正常结束但增量无法解析：预览不会更新，要说清楚原因。
      await act(async () => settledListener?.({
        ...settled('completed', false),
        error: { code: 'forge/invalid-spec', detail: 'EOF while parsing an object at line 20 column 0' },
      }));
      await until(() => host.querySelector('[data-forge-delta-notice]') !== null);
      expect(host.textContent).toContain('增量格式有误');

      // 之后一轮正常收口：提示消失。
      await act(async () => settledListener?.(settled('completed', true)));
      await until(() => host.querySelector('[data-forge-delta-notice]') === null);
    });
  });
});

describe('ConversationRunWorkspaceResourcePanel (forge-preview memo boundary)', () => {
  let host: HTMLDivElement;
  let root: Root;

  const run: ConversationRunVm = {
    projectId: 'project-a',
    taskId: 'task-001',
    runId: 'run-1',
    runMode: 'direct',
    sessionTree: { rounds: [], selectedSessionKey: null },
    activeSessions: [],
    inputAttachments: [],
    workflowStatus: 'valid',
    workflowValid: true,
    workflowGraph: { nodes: [], edges: [], groups: [] },
    resumable: false,
    scheduledTaskId: null,
  } as unknown as ConversationRunVm;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    graphViewRender.count = 0;
    forgeGetSpecMock.mockReset();
    forgeCompileMock.mockReset();
    forgeGetSpecMock.mockResolvedValue(converged);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  /**
   * 预览 Tab 挂在按 run 重渲染的宿主里:对话流式期间 run 快照不断更新,但预览只订阅
   * 轮次收口事件,不应跟着每条流式更新重渲染。宿主因 run 变化重渲染时,预览子树零提交。
   */
  it('does not re-render the preview subtree when the host re-renders on an unrelated run update', async () => {
    const onForgeTrialSessionStarted = vi.fn();
    const onForgePrompt = vi.fn();
    const props = { agentRegistry: null, onForgeTrialSessionStarted, onForgePrompt };
    await act(async () => root.render(<ConversationRunWorkspaceResourcePanel resource={resource} run={run} {...props} />));
    await until(() => graphViewRender.count > 0);
    const baseline = graphViewRender.count;
    expect(baseline).toBeGreaterThan(0);

    // 同一会话的流式更新:新的 run 快照,预览的输入没有变化。
    const streamed = { ...run, lastActivityAt: '2026-11-24T10:00:01Z' };
    await act(async () => root.render(<ConversationRunWorkspaceResourcePanel resource={resource} run={streamed} {...props} />));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(graphViewRender.count).toBe(baseline);

    // 对照:预览真正依赖的输入变化时照常重渲染(计数器有效)。
    const registry = { agents: [] } as unknown as AgentRegistryVm;
    await act(async () => root.render(<ConversationRunWorkspaceResourcePanel resource={resource} run={streamed} agentRegistry={registry} onForgeTrialSessionStarted={onForgeTrialSessionStarted} onForgePrompt={onForgePrompt} />));
    expect(graphViewRender.count).toBe(baseline + 1);
  });
});
