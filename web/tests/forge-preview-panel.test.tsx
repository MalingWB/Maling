/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const forgeGetSpecMock = vi.fn();
const forgeCompileMock = vi.fn();
const forgeStartTrialMock = vi.fn();
const onTemplatesChangeMock = vi.fn();
const onTrialStartedMock = vi.fn();
vi.mock('../src/api', () => ({
  forgeGetSpec: (...args: unknown[]) => forgeGetSpecMock(...args),
  forgeCompile: (...args: unknown[]) => forgeCompileMock(...args),
  forgeStartTrial: (...args: unknown[]) => forgeStartTrialMock(...args),
  subscribeAcpSessionUpdates: () => Promise.resolve(() => {}),
}));

import { ForgePreviewPanel } from '@/components/workspace/ForgePreviewPanel';
import type { ForgePreviewWorkspaceResource } from '@/components/workspace/right-workspace-context';
import '@/i18n';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

/**
 * 铸造预览 Tab(方案 §5.2 第一阶段视图)契约:Spec 是会话唯一事实源,
 * Tab 是其只读投影——存档为空显示空态引导;有 Spec 时阶段四要素与
 * 未收敛项必须完整可见(角色/产物形态/成功条件/失败策略)。
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

const sampleSpec = {
  specVersion: '0.1',
  name: '登录功能流水线',
  intent: '实现登录并验证',
  sourceSkills: [],
  workflows: [
    {
      name: '登录功能流水线',
      stages: [
        {
          id: 's1',
          role: 'dev',
          goal: '<输入>需求\n<产出>代码\n<边界>不做部署\n<验收>测试通过',
          artifact: 'code',
          outputKind: 'json',
          schemaHint: { ok: 'boolean' },
          successCondition: '$.ok == true',
          onFailure: 'retry',
          manualCheck: false,
        },
        {
          role: { custom: { 'profile-name': '联调' } },
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

describe('ForgePreviewPanel', () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    forgeGetSpecMock.mockReset();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it('shows the empty guidance while the session has produced no spec archive', async () => {
    forgeGetSpecMock.mockResolvedValue(null);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    expect(host.textContent).toContain('尚未产出铸造草稿');
  });

  it('renders stage contracts and open questions from the archived spec', async () => {
    forgeGetSpecMock.mockResolvedValue(sampleSpec);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));

    expect(host.textContent).toContain('实现登录并验证');
    // goal 四要素解析为结构化行:要素标签与内容分行呈现(角标形式样例)。
    const rows = [...host.querySelectorAll('dl div')];
    expect(rows.length).toBeGreaterThanOrEqual(4);
    expect(rows[0].textContent).toContain('输入');
    expect(rows[0].textContent).toContain('需求');
    expect(rows[3].textContent).toContain('验收');
    expect(rows[3].textContent).toContain('测试通过');
    // 产物契约与失败策略投影。
    expect(host.textContent).toContain('JSON 产物');
    expect(host.textContent!.replace(/\s/g, '')).toContain('$.ok==true');
    expect(host.textContent).toContain('自动重试');
    expect(host.textContent).toContain('人工复核');
    // custom 角色显示 profile 名。
    expect(host.textContent).toContain('联调');
    // 未收敛项可见。
    expect(host.textContent).toContain('失败后是否自动开新轮?');
  });

  it('structures the natural colon form of goal elements into labeled rows', async () => {
    // 真机验收回归:模型常以「输入：…；产出：…」段落表达,同样结构化展示。
    const spec = {
      ...sampleSpec,
      workflows: [{
        name: '文章摘要事实核查',
        stages: [{
          id: 's1',
          role: 'dev',
          goal: '输入：用户提供的文章原文；产出：中文 markdown 要点总结；边界：只提炼要点，不做事实真伪判断，不改写原文观点；验收：要点覆盖文章全部核心信息，无遗漏。',
          artifact: '要点总结',
          outputKind: 'markdown',
          schemaHint: null,
          successCondition: null,
          onFailure: 'retry',
          manualCheck: false,
        }],
      }],
      openQuestions: [],
    };
    forgeGetSpecMock.mockResolvedValue(spec);
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));

    const rows = [...host.querySelectorAll('dl div')];
    expect(rows).toHaveLength(4);
    expect(rows[0].textContent).toContain('用户提供的文章原文');
    expect(rows[1].textContent).toContain('中文 markdown 要点总结');
    expect(rows[2].textContent).toContain('不做事实真伪判断');
    expect(rows[3].textContent).toContain('无遗漏');
    // markdown 产物行可见,无成功条件时不渲染该码片段。
    expect(host.textContent).toContain('Markdown 产物');
  });

  it('renders an error-state empty view when the spec archive cannot be read', async () => {
    forgeGetSpecMock.mockRejectedValue(new Error('io'));
    await act(async () => root.render(<ForgePreviewPanel resource={resource} />));
    expect(host.textContent).toContain('尚未产出铸造草稿');
  });

  it('blocks compile while open questions remain, then compiles and offers the trial entry', async () => {
    forgeGetSpecMock.mockResolvedValue(sampleSpec);
    const templates = { version: '0.1', lastUsedTemplateId: 'wf-1', lastCreatedWorkflow: null, templates: [] };
    forgeCompileMock.mockResolvedValue({ createdTemplateIds: ['wf-1'], templates });
    forgeStartTrialMock.mockResolvedValue({ projectId: 'project-a', taskId: 'task-trial', title: '试运行' });
    const props = {
      onWorkflowTemplatesChange: onTemplatesChangeMock,
      onTrialSessionStarted: onTrialStartedMock,
    };
    await act(async () => root.render(<ForgePreviewPanel resource={resource} {...props} />));

    // 未收敛(openQuestions 非空):编译禁用,试运行不出现。
    let compileButton = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('编译为模板'));
    expect(compileButton).toBeTruthy();
    expect(compileButton!.disabled).toBe(true);
    expect(host.textContent).not.toContain('试运行首节点');

    // 收敛后:卸载重挂载(新会话轮/重开 Tab 的等价形态),编译可用,调用
    // forgeCompile 并刷新模板库;试运行入口出现。
    const converged = { ...sampleSpec, openQuestions: [] };
    forgeGetSpecMock.mockResolvedValue(converged);
    await act(async () => root.render(null));
    await act(async () => root.render(<ForgePreviewPanel resource={resource} {...props} />));
    compileButton = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('编译为模板'));
    expect(compileButton!.disabled).toBe(false);
    await act(async () => compileButton?.click());
    expect(forgeCompileMock).toHaveBeenCalledWith(converged);
    expect(onTemplatesChangeMock).toHaveBeenCalledWith(templates);
    expect(host.textContent).toContain('已创建 1 个工作流模板');

    const trialButton = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('试运行首节点'));
    expect(trialButton).toBeTruthy();
    await act(async () => trialButton?.click());
    expect(forgeStartTrialMock).toHaveBeenCalledWith('project-a', 'wf-1', converged.intent);
    expect(onTrialStartedMock).toHaveBeenCalledWith('project-a', 'task-trial');
  });
});
