import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FlaskConical, Loader2, Play, TriangleAlert } from 'lucide-react';
import { forgeCompile, forgeGetSpec, forgeStartTrial, subscribeAcpSessionUpdates } from '../../api';
import { useEventDrivenRefresh } from '@/lib/use-event-driven-refresh';
import type { ForgePreviewWorkspaceResource } from './right-workspace-context';
import type { ForgeCompileResultVm, ForgeSpecVm, ForgeStageVm, WorkflowTemplateStore } from '../../types';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';

interface ForgePreviewPanelProps {
  resource: ForgePreviewWorkspaceResource;
  /// 编译落库后刷新持有方的模板库(工作流模板管理/选择器共用)。
  onWorkflowTemplatesChange?: (store: WorkflowTemplateStore) => void;
  /// 试运行任务创建并启动后,由持有方导航到该会话(方案 §5.6)。
  onTrialSessionStarted?: (projectId: string, taskId: string) => void;
  /// G1 绑定检查:无任何可用 agent 时编译成功后提醒(不阻塞)。
  agentRegistry?: { agents: { diagnostic?: { available: boolean } | null }[] } | null;
}

/**
 * 铸造预览（方案 §5.2,第一阶段视图）：Forge 会话的 Spec 摘要 + 编译/试运行动作。
 *
 * Spec 是会话唯一事实源,Tab 只是其投影——打开/重开时从 task 级存档
 * 全量读取(≤10 节点,成本可忽略)。收敛(openQuestions 清空)后可编译
 * 为模板(source=Forged),并可发起首节点试运行(第三阶段在本 Tab 原位
 * 切换为流程图与编辑器预览,本组件届时退役为摘要区块)。
 */
/// invoke 错误(CommandErrorVm 或 Error)转可读文本。
function forgeActionErrorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'object' && err !== null) {
    const record = err as { message?: unknown; code?: unknown; params?: { message?: unknown } };
    if (typeof record.message === 'string' && record.message) return record.message;
    if (typeof record.params?.message === 'string' && record.params.message) {
      return `${String(record.code ?? '')} ${record.params.message}`.trim();
    }
    if (typeof record.code === 'string') return record.code;
    try {
      return JSON.stringify(err);
    } catch {
      return String(err);
    }
  }
  return String(err);
}

export function ForgePreviewPanel({ resource, onWorkflowTemplatesChange, onTrialSessionStarted, agentRegistry }: ForgePreviewPanelProps) {
  const { t } = useTranslation();
  const [spec, setSpec] = useState<ForgeSpecVm | null | undefined>(undefined);
  const [compileResult, setCompileResult] = useState<ForgeCompileResultVm | null>(null);
  const [busy, setBusy] = useState<'compile' | 'trial' | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const load = useCallback(() => {
    let cancelled = false;
    forgeGetSpec(resource.projectId, resource.taskId)
      .then((next) => {
        if (!cancelled) setSpec(next);
      })
      .catch(() => {
        if (!cancelled) setSpec(null);
      });
    return () => {
      cancelled = true;
    };
  }, [resource.projectId, resource.taskId]);

  // 事件驱动刷新:铸造对话每轮完成后端提取 Spec 增量并随会话更新事件
  // 通知;本 Tab 是 Spec 的只读投影,按 task 过滤事件去重刷新(挂载时
  // 先取一次;编译结果不因刷新复述——模板库才是权威)。
  useEventDrivenRefresh(
    () => { load(); },
    [
      useCallback((listener: () => void) => subscribeAcpSessionUpdates((event) => {
        if (event.taskId === resource.taskId) listener();
      }), [resource.taskId]),
    ],
    { refreshOnMount: true },
  );
  // 会话恢复或重开时,上一次编译结果不复述(模板库已是权威)。
  useEffect(() => {
    setCompileResult(null);
    setActionError(null);
  }, [resource.key]);

  const openQuestions = spec?.openQuestions ?? [];
  const converged = spec !== null && spec !== undefined && openQuestions.length === 0;

  const handleCompile = async () => {
    if (!spec || busy) return;
    setBusy('compile');
    setActionError(null);
    try {
      const result = await forgeCompile(spec);
      setCompileResult(result);
      onWorkflowTemplatesChange?.(result.templates);
    } catch (err) {
      setActionError(forgeActionErrorText(err));
    } finally {
      setBusy(null);
    }
  };

  const handleStartTrial = async () => {
    if (!spec || !compileResult || busy) return;
    const templateId = compileResult.createdTemplateIds[0];
    if (!templateId) return;
    setBusy('trial');
    setActionError(null);
    try {
      // 样例需求 = 锚定轮确认的 intent(方案 §5.6);任务创建后由持有方
      // 启动并导航——首节点即停已由试运行任务的截断工作流保证。
      const trial = await forgeStartTrial(resource.projectId, templateId, spec.intent);
      onTrialSessionStarted?.(trial.projectId, trial.taskId);
    } catch (err) {
      setActionError(forgeActionErrorText(err));
    } finally {
      setBusy(null);
    }
  };

  if (spec === undefined) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        {t('forge.preview.loading')}
      </div>
    );
  }
  if (spec === null) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
        {t('forge.preview.empty')}
      </div>
    );
  }

  return (
    <TooltipProvider>
    <div data-right-workspace-resource="forge-preview" className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/60 px-3 py-2">
        <p className="min-w-0 truncate text-xs text-muted-foreground">{spec.intent}</p>
        <div className="flex shrink-0 items-center gap-1.5">
          {compileResult ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-6 gap-1 px-2 text-xs"
                  disabled={busy !== null}
                  onClick={() => void handleStartTrial()}
                >
                  {busy === 'trial' ? <Loader2 className="size-3 animate-spin" /> : <FlaskConical className="size-3" />}
                  {t('forge.preview.startTrial')}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t('forge.preview.startTrialHint')}</TooltipContent>
            </Tooltip>
          ) : null}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className="h-6 gap-1 px-2 text-xs"
                disabled={!converged || busy !== null}
                onClick={() => void handleCompile()}
              >
                {busy === 'compile' ? <Loader2 className="size-3 animate-spin" /> : <Play className="size-3" />}
                {t('forge.preview.compile')}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{converged ? t('forge.preview.compileHint') : t('forge.preview.compileBlocked')}</TooltipContent>
          </Tooltip>
        </div>
      </div>
      {compileResult ? (
        <div className="shrink-0 px-3 py-1.5 text-xs text-muted-foreground">
          {t('forge.preview.compiledCount', { count: compileResult.createdTemplateIds.length })}
          {agentRegistry && agentRegistry.agents.length > 0
          && agentRegistry.agents.every((agent) => agent.diagnostic && !agent.diagnostic.available) ? (
            <span className="ml-2 text-foreground/70">{t('forge.preview.bindingMissing')}</span>
          ) : null}
        </div>
      ) : null}
      {actionError ? (
        <div className="shrink-0 px-3 py-1.5 text-xs text-destructive">
          <span className="inline-flex min-w-0 items-start gap-1.5">
            <TriangleAlert className="mt-0.5 size-3 shrink-0" />
            <span className="min-w-0 break-all">{actionError}</span>
          </span>
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {(spec.workflows ?? []).map((workflow, index) => (
          <section key={`${workflow.name}-${index}`} className="mt-3">
            <h3 className="text-sm font-medium">{workflow.name}</h3>
            <ol className="mt-1.5 space-y-2">
              {workflow.stages.map((stage, stageIndex) => (
                <ForgeStageRow key={stage.id ?? stageIndex} stage={stage} index={stageIndex + 1} />
              ))}
            </ol>
          </section>
        ))}
        {openQuestions.length > 0 ? (
          <section className="mt-3">
            <h3 className="text-sm font-medium">{t('forge.preview.openQuestions')}</h3>
            <ul className="mt-1.5 list-disc space-y-1 pl-5 text-xs text-muted-foreground">
              {openQuestions.map((question, index) => (
                <li key={index}>{question}</li>
              ))}
            </ul>
          </section>
        ) : null}
      </div>
    </div>
    </TooltipProvider>
  );
}

/// goal 四要素的结构化解析:接受协议示例的 `<输入>` 标记与模型常见的
/// 「输入：…；」前缀两种形式;命中不足两段视为不可结构化,调用方回退整段展示。
type GoalElements = { key: 'input' | 'output' | 'boundary' | 'acceptance'; text: string }[];

function parseGoalElements(goal: string): GoalElements | null {
  const elements: Array<{ key: 'input' | 'output' | 'boundary' | 'acceptance'; name: string }> = [
    { key: 'input', name: '输入' },
    { key: 'output', name: '产出' },
    { key: 'boundary', name: '边界' },
    { key: 'acceptance', name: '验收' },
  ];
  const hits: Array<{ key: 'input' | 'output' | 'boundary' | 'acceptance'; contentStart: number; start: number }> = [];
  for (const element of elements) {
    const angle = goal.indexOf(`<${element.name}>`);
    if (angle >= 0) {
      hits.push({ key: element.key, start: angle, contentStart: angle + element.name.length + 2 });
      continue;
    }
    const colon = goal.indexOf(`${element.name}：`) >= 0
      ? goal.indexOf(`${element.name}：`)
      : goal.indexOf(`${element.name}:`);
    if (colon >= 0) {
      hits.push({ key: element.key, start: colon, contentStart: colon + element.name.length + 1 });
    }
  }
  if (hits.length < 2) return null;
  hits.sort((left, right) => left.start - right.start);
  return hits.map((hit, position) => {
    const end = position + 1 < hits.length ? hits[position + 1].start : goal.length;
    return {
      key: hit.key,
      text: goal.slice(hit.contentStart, end).replace(/^[\s:：]+/, '').replace(/[；;\s]+$/, '').trim(),
    };
  }).filter((item) => item.text.length > 0);
}

function ForgeStageRow({ stage, index }: { stage: ForgeStageVm; index: number }) {
  const { t } = useTranslation();
  const role = typeof stage.role === 'string' ? stage.role : stage.role.custom['profile-name'];
  const roleLabel = ['dev', 'test', 'review', 'plan', 'accept'].includes(role)
    ? t(`forge.preview.role.${role}`)
    : role;
  const elements = parseGoalElements(stage.goal ?? '');
  return (
    <li className="rounded-lg border border-border/60 bg-card px-3.5 py-2.5">
      <div className="flex min-w-0 items-center gap-2">
        <span className="inline-flex size-4.5 shrink-0 items-center justify-center rounded-full bg-foreground/10 text-[0.65rem] font-medium tabular-nums text-foreground">
          {index}
        </span>
        <span className="min-w-0 truncate text-xs font-medium text-foreground">
          {stage.artifact || roleLabel}
        </span>
        <span className="ml-auto inline-flex shrink-0 items-center rounded border border-border/60 px-1.5 py-px text-[0.65rem] text-muted-foreground">
          {roleLabel}
        </span>
        <span className="inline-flex shrink-0 items-center rounded border border-border/60 px-1.5 py-px text-[0.65rem] text-muted-foreground">
          {t(`forge.preview.onFailure.${stage.onFailure}`)}
        </span>
      </div>
      {elements ? (
        <dl className="mt-2 space-y-1">
          {elements.map((element) => (
            <div key={element.key} className="flex gap-2 text-xs leading-relaxed">
              <dt className="w-7 shrink-0 text-right text-muted-foreground/80">
                {t(`forge.preview.element.${element.key}`)}
              </dt>
              <dd className="min-w-0 flex-1 break-words text-foreground/85">{element.text}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="mt-2 whitespace-pre-wrap break-words text-xs leading-relaxed text-foreground/85">{stage.goal}</p>
      )}
      {(stage.outputKind !== 'none' || stage.successCondition) ? (
        <div className="mt-2 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border/40 pt-1.5 text-[0.7rem] text-muted-foreground">
          <span>{t('forge.preview.outputKindLabel')}: {t(`forge.preview.outputKind.${stage.outputKind}`)}</span>
          {stage.successCondition ? (
            <span className="inline-flex min-w-0 items-center gap-1">
              <span className="shrink-0">{t('forge.preview.successCondition')}:</span>
              <code className="min-w-0 break-all rounded bg-muted/40 px-1 py-px font-mono text-[0.65rem]">{stage.successCondition}</code>
            </span>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}
