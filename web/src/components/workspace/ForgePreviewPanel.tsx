import { memo, useCallback, useEffect, useId, useMemo, useReducer, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Check, ChevronDown, FlaskConical, Loader2, Pencil, Square, TriangleAlert } from 'lucide-react';
import { forgeCompile, forgeGetSpec, forgeSetStageFailure, forgeStartTrial, subscribeForgeTurnSettled } from '../../api';
import type { ForgeTurnSettledEventVm } from '../../api/client';
import { GraphView } from '../GraphView';
import type { GraphVm, WorkflowGraphDirectionPreference } from '../../types';
import { WorkflowGraphDirectionProvider } from '@/components/workflow-direction-preference';
import {
  END_NODE,
  LAYER_SPACING,
  LAYOUT_PADDING,
  NEW_ROUND_NODE,
  NODE_HEIGHT,
  TERMINAL_NODE_HEIGHT,
  WORKFLOW_FIT_MAX_ZOOM,
} from '@/components/workflowGraph';
import type { ForgePreviewWorkspaceResource } from './right-workspace-context';
import type {
  ForgeCompileOutcomeVm,
  ForgeErrorVm,
  ForgeOnFailureVm,
  ForgeQuestionVm,
  ForgeSpecVm,
  ForgeStageVm,
  ForgeTrialInfo,
  WorkflowTemplateStore,
} from '../../types';
import type { AcpPromptSubmitResult } from '@/components/acp/ACPChatDialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Checkbox } from '@/components/ui/checkbox';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { displayAppError } from '@/i18n';
import { forgeCommandErrorText, forgeErrorText, forgeRepairText } from '@/lib/forge-errors';
import { createAcpPromptId } from '@/lib/acp-event-normalization';
import { forgeRepairActive, forgeRepairReducer, initialForgeRepairState } from '@/lib/forge-repair-loop';
import {
  changedForgeStageIds,
  forgeFailureTargets,
  forgeIndexedStages,
  forgeOnFailureContinuesSession,
  forgeOnFailureKind,
  forgeOnFailureTarget,
  forgeOnFailureValue,
  forgeQuestion,
  forgeSpecContentKey,
  type ForgeIndexedStage,
  type ForgeOnFailureKind,
} from '@/lib/forge-spec';
import { cn } from '@/lib/utils';

type ForgeCompileSaved = Extract<ForgeCompileOutcomeVm, { status: 'saved' }>;

interface ForgePreviewPanelProps {
  resource: ForgePreviewWorkspaceResource;
  /// 编译落库后刷新持有方的模板库(工作流模板管理/选择器共用)。
  onWorkflowTemplatesChange?: (store: WorkflowTemplateStore) => void;
  /// 试运行任务创建并启动后,由持有方导航到该会话(方案 §5.6)。
  onTrialSessionStarted?: (projectId: string, taskId: string) => void;
  /// 经当前会话的对话栏发出一条消息：自动修正轮与「采用推荐」共用这条提交路径。缺省时只展示、不发送。
  onSubmitPrompt?: (prompt: string, promptId?: string) => Promise<AcpPromptSubmitResult>;
  /// 把一段文字带进对话输入框（不发送）。缺省时阶段卡片上没有「改这一步」。
  onPrefillPrompt?: (text: string) => void;
  /// 打开已保存的工作流查看。缺省时完成态里没有「查看工作流」。
  onViewWorkflow?: (templateId: string) => void;
  /// 对话正在回复：会写草稿的操作（改失败策略、采用推荐）等回复结束再做，避免被这一轮的增量覆盖。
  conversationActive?: boolean;
  /// G1 绑定检查:无任何可用 agent 时保存成功后提醒(不阻塞)。
  agentRegistry?: { agents: { diagnostic?: { available: boolean } | null }[] } | null;
}

type AdoptionState = { pending: ReadonlySet<string>; notice: 'busy' | 'unavailable' | null };
const NO_ADOPTION: AdoptionState = { pending: new Set(), notice: null };

/** 失败策略的可选项，按常见程度排列。 */
const FAILURE_KINDS: ForgeOnFailureKind[] = ['retry', 'manual-check', 'jump', 'new-round', 'new-round-from', 'end'];

/**
 * 工作流预览（方案 §5.2）：生成会话的草稿与操作入口。
 *
 * Spec 是会话唯一事实源,Tab 只是其投影——打开/重开时从 task 级存档全量读取
 * (≤10 节点,成本可忽略),之后只在后端发出"轮次收口"事件时重读(该事件在增量
 * 提取并落档之后发出,读到的就是这一轮之后的存档)。
 *
 * 自上而下：标题与主操作 → 待澄清（每题可「采用推荐」）→ 保存后的去向 → 蓝图 →
 * 阶段卡片。待澄清是保存的前提,所以排在最前面;蓝图紧跟其后,位置不随上面的区块
 * 出现而移动太多。阶段卡片上可以直接改失败策略（经后端同一个合并入口落档）。
 *
 * 保存未通过时由这里驱动自动修正回路(`lib/forge-repair-loop`):修正提示词经对话
 * 栏发出,回路按轮次收口事件与重新保存的结果推进,最多三轮。宿主以 resource.key
 * 作为本组件的 key,切换会话时状态整体重建。
 */
function ForgePreviewPanelComponent({
  resource,
  onWorkflowTemplatesChange,
  onTrialSessionStarted,
  onSubmitPrompt,
  onPrefillPrompt,
  onViewWorkflow,
  conversationActive = false,
  agentRegistry,
}: ForgePreviewPanelProps) {
  const { t } = useTranslation();
  const cardIdPrefix = useId();
  /** undefined = 尚未读到;null = 会话还没有存档。 */
  const [spec, setSpec] = useState<ForgeSpecVm | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [compileResult, setCompileResult] = useState<ForgeCompileSaved | null>(null);
  /** 上一次保存成功时草稿的内容指纹：草稿之后变过，上一次保存就不再代表当前草稿。 */
  const [compiledSpecKey, setCompiledSpecKey] = useState<string | null>(null);
  const [busy, setBusy] = useState<'compile' | 'trial' | null>(null);
  const [copyTarget, setCopyTarget] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** 最近一轮回复里有 Spec 增量但无法应用（与修正回路无关的普通对话轮）。 */
  const [deltaNotice, setDeltaNotice] = useState<'unconfirmed' | 'invalid-delta' | null>(null);
  const [repair, dispatchRepair] = useReducer(forgeRepairReducer, initialForgeRepairState);
  /** 上一轮对话改动的阶段：蓝图与卡片上标出，直到下一轮。 */
  const [changedStageIds, setChangedStageIds] = useState<ReadonlySet<string>>(() => new Set());
  /** 蓝图与卡片之间联动的选中阶段。 */
  const [focusedStageId, setFocusedStageId] = useState<string | null>(null);
  /** 已经发给对话的「采用推荐」：在草稿更新前保持等待态，避免重复发送。 */
  const [adoption, setAdoption] = useState<AdoptionState>(NO_ADOPTION);

  const specRef = useRef(spec);
  const acceptSpec = useCallback((next: ForgeSpecVm | null) => {
    if ((next?.revision ?? 0) < (specRef.current?.revision ?? 0)) return false;
    specRef.current = next;
    setSpec(next);
    return true;
  }, []);
  const loadGenerationRef = useRef(0);
  /** 读取存档;失败时保留已有内容并返回 undefined(由调用方决定如何收口)。 */
  const loadSpec = useCallback(async (): Promise<ForgeSpecVm | null | undefined> => {
    const generation = ++loadGenerationRef.current;
    try {
      const next = await forgeGetSpec(resource.projectId, resource.taskId);
      // 迟到的响应不覆盖更新的读取结果。
      if (generation !== loadGenerationRef.current) return undefined;
      if (!acceptSpec(next)) return undefined;
      setLoadError(null);
      return next;
    } catch (error) {
      if (generation !== loadGenerationRef.current) return undefined;
      setLoadError(displayAppError(t, error));
      return undefined;
    }
  }, [resource.projectId, resource.taskId, t, acceptSpec]);

  const compileSpec = useCallback(
    (target: ForgeSpecVm, repairAttempt: number) =>
      forgeCompile(resource.projectId, target, resource.taskId, repairAttempt),
    [resource.projectId, resource.taskId],
  );

  // 订阅与回路步骤在挂载后只读最新值,不因回调身份变化重新订阅或重复执行。
  const onWorkflowTemplatesChangeRef = useRef(onWorkflowTemplatesChange);
  const onSubmitPromptRef = useRef(onSubmitPrompt);
  const loadSpecRef = useRef(loadSpec);
  const compileSpecRef = useRef(compileSpec);
  onWorkflowTemplatesChangeRef.current = onWorkflowTemplatesChange;
  onSubmitPromptRef.current = onSubmitPrompt;
  loadSpecRef.current = loadSpec;
  compileSpecRef.current = compileSpec;

  const showSaved = useCallback((saved: ForgeCompileSaved, specKey: string) => {
    if (saved.spec && !acceptSpec(saved.spec)) return;
    setCopyTarget(null);
    setCompileResult(saved);
    setCompiledSpecKey(saved.spec ? forgeSpecContentKey(saved.spec) : specKey);
    setActionError(null);
    onWorkflowTemplatesChangeRef.current?.(saved.templates);
  }, [acceptSpec]);
  const showSavedRef = useRef(showSaved);
  showSavedRef.current = showSaved;

  useEffect(() => {
    void loadSpec();
  }, [loadSpec]);

  // 轮次收口:重读存档刷新投影,标出这一轮改动的阶段,并把结果交给修正回路(只在回路
  // 等待这一轮时生效)。
  const handleTurnSettledRef = useRef((_event: ForgeTurnSettledEventVm) => {});
  handleTurnSettledRef.current = (event) => {
    // Active repair owns its one post-settlement archive read. Ordinary refreshes must
    // not invalidate that read when a previous turn or duplicate notification arrives.
    if (!forgeRepairActive(repair)) {
      const before = specRef.current;
      void loadSpecRef.current().then((next) => {
        if (next) setChangedStageIds(changedForgeStageIds(before, next));
      });
      setAdoption(NO_ADOPTION);
      setDeltaNotice(event.error?.code === 'forge/turn-unconfirmed' ? 'unconfirmed' : event.error ? 'invalid-delta' : null);
      return;
    }
    dispatchRepair({
      type: 'turn-settled',
      promptId: event.promptId,
      unconfirmed: event.error?.code === 'forge/turn-unconfirmed',
      outcome: event.outcome,
      specUpdated: event.specUpdated,
      invalidDelta: Boolean(event.error),
    });
  };
  useEffect(() => {
    let active = true;
    let dispose: (() => void) | null = null;
    void subscribeForgeTurnSettled((event) => {
      if (!active || event.projectId !== resource.projectId || event.taskId !== resource.taskId) return;
      handleTurnSettledRef.current(event);
    }).then(
      (unlisten) => {
        if (active) dispose = unlisten;
        else unlisten();
      },
      () => {},
    );
    return () => {
      active = false;
      dispose?.();
    };
  }, [resource.projectId, resource.taskId]);

  // 回路的两个异步环节(发出修正轮、重新保存)由状态推进触发,结果再交回状态机。
  const repairStepRef = useRef<string | null>(null);
  useEffect(() => {
    if (repair.status !== 'submitting' && repair.status !== 'recompiling') {
      // 离开异步环节后清掉标记:下一次回路可以复用同样的轮次编号。
      repairStepRef.current = null;
      return;
    }
    const step = `${repair.status}:${repair.promptId}`;
    // 同一步只执行一次(开发态 StrictMode 会把 effect 跑两遍)。
    if (repairStepRef.current === step) return;
    repairStepRef.current = step;
    if (repair.status === 'submitting') {
      const submit = onSubmitPromptRef.current;
      if (!submit) {
        dispatchRepair({ type: 'submit-settled', promptId: repair.promptId, result: 'unavailable' });
        return;
      }
      submit(repair.prompt, repair.promptId).then(
        (result) => dispatchRepair({ type: 'submit-settled', promptId: repair.promptId, result }),
        () => dispatchRepair({ type: 'submit-settled', promptId: repair.promptId, result: 'unavailable' }),
      );
      return;
    }
    void (async () => {
      // 修正轮的增量已落档:重新保存前重读,用这一轮之后的存档。
      const before = specRef.current;
      const latest = await loadSpecRef.current();
      if (repairStepRef.current !== step) return;
      if (!latest) {
        dispatchRepair({ type: 'recompiled', promptId: repair.promptId, outcome: 'failed' });
        return;
      }
      try {
        setChangedStageIds(changedForgeStageIds(before, latest));
        const outcome = await compileSpecRef.current(latest, repair.nextAttempt);
        if (repairStepRef.current !== step) return;
        if (outcome.status === 'saved') {
          showSavedRef.current(outcome, forgeSpecContentKey(latest));
          dispatchRepair({ type: 'recompiled', promptId: repair.promptId, outcome: 'saved' });
        } else {
          setCompileResult(null);
          setCompiledSpecKey(null);
          dispatchRepair({
            type: 'recompiled',
            promptId: repair.promptId,
            nextPromptId: createAcpPromptId(),
            outcome: 'repairable',
            errors: outcome.errors,
            prompt: outcome.repairPrompt,
          });
        }
      } catch {
        dispatchRepair({ type: 'recompiled', promptId: repair.promptId, outcome: 'failed' });
      }
    })();
  }, [repair]);

  const questions = useMemo(() => (spec?.openQuestions ?? []).map(forgeQuestion), [spec]);
  const converged = spec !== null && spec !== undefined && questions.length === 0;
  /** 草稿已收敛，但与上一次保存的内容不同（或还没保存过）。 */
  const needsSave = converged && compiledSpecKey !== forgeSpecContentKey(spec);
  const repairActive = forgeRepairActive(repair);
  const repairErrors: ForgeErrorVm[] = 'errors' in repair ? repair.errors : [];
  const repairText = forgeRepairText(t, repair);

  const handleCompile = async (copy?: { workflowId: string; name: string }) => {
    const current = specRef.current;
    if (!current || busy) return;
    setBusy('compile');
    setActionError(null);
    if (!copy) {
      setCompileResult(null);
      setCompiledSpecKey(null);
    }
    setDeltaNotice(null);
    // 重新发起保存即回到回路起点(上一轮的停止状态不再保留)。
    dispatchRepair({ type: 'reset' });
    try {
      const outcome = copy
        ? await forgeCompile(resource.projectId, current, resource.taskId, 1, copy)
        : await compileSpec(current, 1);
      if (outcome.status === 'saved') {
        if (copy && (current.workflows?.length ?? 0) > 1) {
          // Only the selected workflow was saved. The batch still needs an explicit save.
          if (outcome.spec) acceptSpec(outcome.spec);
          setCompileResult(null);
          setCompiledSpecKey(null);
          onWorkflowTemplatesChangeRef.current?.(outcome.templates);
          setCopyTarget(null);
        } else showSaved(outcome, forgeSpecContentKey(current));
      } else if (copy) {
        setActionError(outcome.errors.map(error => forgeErrorText(t, error, current)).join(' '));
      } else {
        dispatchRepair({ type: 'repair-needed', promptId: createAcpPromptId(), prompt: outcome.repairPrompt, errors: outcome.errors });
      }
    } catch (error) {
      const failure = error as { code?: string; params?: { workflow?: string } } | null;
      if (failure?.code === 'forge/target-unavailable' && failure.params?.workflow) setCopyTarget(failure.params.workflow);
      setActionError(forgeCommandErrorText(t, error) ?? displayAppError(t, error));
      // A batch may have checkpointed earlier members before the failure.
      await loadSpec();
    } finally {
      setBusy(null);
    }
  };

  const handleStartTrial = async () => {
    if (!spec || !compileResult || busy) return;
    const review = compileResult.trial;
    if (!review) return;
    setBusy('trial');
    setActionError(null);
    try {
      // 样例需求 = 草稿里的 intent;任务创建后由持有方启动并导航——首节点即停已由
      // 试运行任务的截断工作流保证。
      const trial = await forgeStartTrial(resource.projectId, review.templateId, review.revision, spec.intent);
      onTrialSessionStarted?.(trial.projectId, trial.taskId);
    } catch (error) {
      const stale = error as { code?: string; params?: { current?: ForgeTrialInfo } } | null;
      const current = stale?.params?.current;
      if (stale?.code === 'forge/stale-trial' && current?.templateId === review.templateId) {
        setCompileResult(saved => saved?.trial === review ? { ...saved, trial: current } : saved);
        setActionError(t('forge.preview.trialChanged'));
      } else setActionError(displayAppError(t, error));
    } finally {
      setBusy(null);
    }
  };

  /** 把待澄清项的推荐答案交给对话落实：一条消息说清采用哪几项。 */
  const adoptRecommendations = async (items: ForgeQuestionVm[]) => {
    const submit = onSubmitPromptRef.current;
    const pending = new Set(items.map((item) => item.question));
    if (!submit) {
      setAdoption({ pending: new Set(), notice: 'unavailable' });
      return;
    }
    const prompt = items.length === 1
      ? t('forge.preview.adoptMessage', { question: items[0].question, recommendation: items[0].recommendation })
      : t('forge.preview.adoptAllMessage', {
        items: items
          .map((item, index) => t('forge.preview.adoptAllItem', { index: index + 1, question: item.question, recommendation: item.recommendation }))
          .join('\n'),
      });
    setAdoption({ pending, notice: null });
    try {
      const result = await submit(prompt);
      if (result !== 'accepted') setAdoption({ pending: new Set(), notice: result === 'busy' ? 'busy' : 'unavailable' });
    } catch {
      setAdoption({ pending: new Set(), notice: 'unavailable' });
    }
  };

  // 从蓝图上点了某个阶段：滚到对应的卡片（用户的显式定位）。
  useEffect(() => {
    if (!focusedStageId) return;
    document.getElementById(`${cardIdPrefix}-${focusedStageId}`)?.scrollIntoView({ block: 'nearest' });
  }, [cardIdPrefix, focusedStageId]);

  if (spec === undefined) {
    if (loadError) {
      return (
        <PanelMessage
          icon={<TriangleAlert className="size-4" />}
          text={t('forge.preview.loadFailed')}
          action={<Button variant="outline" size="sm" onClick={() => void loadSpec()}>{t('common.retry')}</Button>}
        />
      );
    }
    return <PanelMessage icon={<Loader2 className="size-4 animate-spin" />} text={t('forge.preview.loading')} />;
  }
  if (spec === null) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
        {t('forge.preview.empty')}
      </div>
    );
  }

  const workflowName = spec.name?.trim() || spec.workflows?.[0]?.name?.trim() || t('forge.preview.untitled');
  const savedTemplateId = compileResult?.templateIds[0];
  const savedTemplate = compileResult?.templates.templates.find((template) => template.id === savedTemplateId);
  const trialGroup = compileResult?.trial?.group;
  const savedMessage = compileResult
    ? compileResult.createdCount > 0 && compileResult.updatedCount > 0
      ? t('forge.preview.compiledMixed', { created: compileResult.createdCount, updated: compileResult.updatedCount })
      : compileResult.createdCount === 1 && savedTemplate
        ? t('forge.preview.savedCreated', { name: savedTemplate.name })
        : compileResult.updatedCount === 1 && savedTemplate
          ? t('forge.preview.savedUpdated', { name: savedTemplate.name })
          : compileResult.updatedCount > 0
            ? t('forge.preview.compiledUpdated', { count: compileResult.updatedCount })
            : t('forge.preview.compiledCount', { count: compileResult.createdCount })
    : null;
  const bindingMissing = Boolean(
    agentRegistry
    && agentRegistry.agents.length > 0
    && agentRegistry.agents.every((agent) => agent.diagnostic && !agent.diagnostic.available),
  );
  const adoptable = questions.filter((item) => item.recommendation);

  return (
    <TooltipProvider>
      <div data-right-workspace-resource="forge-preview" className="flex min-h-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/60 px-3 py-2">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium text-foreground" data-forge-workflow-name>{workflowName}</p>
            {spec.intent ? <p className="mt-0.5 truncate text-xs text-muted-foreground">{spec.intent}</p> : null}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {repairActive ? (
              <Button
                variant="outline"
                size="sm"
                className="h-6 gap-1 px-2 text-xs"
                onClick={() => dispatchRepair({ type: 'abort' })}
              >
                <Square className="size-3" />
                {t('forge.repair.stop')}
              </Button>
            ) : needsSave ? (
              <Button
                data-forge-save
                size="sm"
                className="h-6 gap-1 px-2 text-xs"
                disabled={busy !== null || conversationActive}
                onClick={() => void handleCompile()}
              >
                {busy === 'compile' ? <Loader2 className="size-3 animate-spin" /> : null}
                {t('forge.preview.compile')}
              </Button>
            ) : null}
          </div>
        </div>
        {repairText ? (
          <div data-forge-repair-status className="flex shrink-0 items-start gap-1.5 border-b border-border/40 px-3 py-1.5 text-xs text-muted-foreground">
            {repairActive ? <Loader2 className="mt-0.5 size-3 shrink-0 animate-spin" /> : <TriangleAlert className="mt-0.5 size-3 shrink-0" />}
            <span className="min-w-0 break-words">{repairText}</span>
          </div>
        ) : null}
        {deltaNotice && !repairText ? (
          <div data-forge-delta-notice className="flex shrink-0 items-start gap-1.5 border-b border-border/40 px-3 py-1.5 text-xs text-muted-foreground">
            <TriangleAlert className="mt-0.5 size-3 shrink-0" />
            <span className="min-w-0 break-words">{t(deltaNotice === 'unconfirmed' ? 'forge.repair.unconfirmed' : 'forge.preview.deltaInvalid')}</span>
          </div>
        ) : null}
        {repairErrors.length > 0 ? (
          <section data-forge-compile-errors className="shrink-0 border-b border-border/40 px-3 py-2">
            <h3 className="text-xs font-medium text-foreground">{t('forge.repair.errorsTitle')}</h3>
            <ul className="mt-1 list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
              {repairErrors.map((error, index) => (
                <li key={`${error.code}:${index}`} className="break-words">
                  {forgeErrorText(t, error, spec)}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {compileResult && needsSave ? (
          <div data-forge-stale-save className="shrink-0 px-3 py-1.5 text-xs text-muted-foreground">
            {t('forge.preview.staleAfterEdit')}
          </div>
        ) : null}
        {actionError ? (
          <div role="alert" className="shrink-0 px-3 py-1.5 text-xs text-destructive">
            <span className="inline-flex min-w-0 items-start gap-1.5">
              <TriangleAlert className="mt-0.5 size-3 shrink-0" />
              <span className="min-w-0 break-all">{actionError}</span>
            </span>
          </div>
        ) : null}
        {loadError ? (
          <div role="alert" className="flex shrink-0 items-center gap-2 border-b border-border/40 px-3 py-1.5 text-xs text-muted-foreground">
            <TriangleAlert className="size-3 shrink-0" />
            <span className="min-w-0 flex-1 break-words">{loadError}</span>
            <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => void loadSpec()}>{t('common.retry')}</Button>
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-y-auto p-3">
          {questions.length > 0 ? (
            <ForgeOpenQuestions
              questions={questions}
              pending={adoption.pending}
              notice={adoption.notice}
              disabled={conversationActive || repairActive || adoption.pending.size > 0}
              onAdopt={(item) => void adoptRecommendations([item])}
              onAdoptAll={() => void adoptRecommendations(adoptable)}
            />
          ) : null}
          {compileResult && converged && !needsSave ? (
            <section data-forge-saved className="mb-3 rounded-lg border border-border/60 bg-card px-3 py-2.5">
              <div className="flex min-w-0 items-start gap-2">
                <Check className="mt-0.5 size-4 shrink-0 text-accent-foreground/80" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <p className="break-words text-sm font-medium text-foreground">{savedMessage}</p>
                  {bindingMissing ? <p className="mt-1 text-xs text-foreground/70">{t('forge.preview.bindingMissing')}</p> : null}
                </div>
              </div>
              {trialGroup && <p data-forge-group-trial className="mt-2 break-words text-xs text-muted-foreground">
                {t('forge.preview.groupTrialScope', { branches: trialGroup.branches, cap: trialGroup.cap })}
              </p>}
              <div className="mt-2 flex flex-wrap gap-1.5">
                <Button size="sm" className="h-7 gap-1 px-2.5 text-xs" disabled={!compileResult.trial || busy !== null} onClick={() => void handleStartTrial()}>
                  {busy === 'trial' ? <Loader2 className="size-3 animate-spin" /> : <FlaskConical className="size-3" />}
                  {t(trialGroup ? 'forge.preview.startGroupTrial' : 'forge.preview.startTrial')}
                </Button>
                {onViewWorkflow && savedTemplateId ? (
                  <Button size="sm" variant="outline" className="h-7 gap-1 px-2.5 text-xs" onClick={() => onViewWorkflow(savedTemplateId)}>
                    {t('forge.preview.viewWorkflow')}
                  </Button>
                ) : null}
                {onPrefillPrompt ? (
                  <Button size="sm" variant="ghost" className="h-7 px-2.5 text-xs" onClick={() => onPrefillPrompt(t('forge.preview.keepAdjustingPrompt'))}>
                    {t('forge.preview.keepAdjusting')}
                  </Button>
                ) : null}
              </div>
            </section>
          ) : null}
          <ForgeSpecGraph
            spec={spec}
            changedStageIds={changedStageIds}
            focusedStageId={focusedStageId}
            onFocusStage={setFocusedStageId}
          />
          {(spec.workflows ?? []).map((workflow, index) => (
            <section key={workflow.id} className="mt-3">
              <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
                <h3 className="min-w-0 break-words text-sm font-medium">{workflow.name}</h3>
                {workflow.id && !copyTarget && converged && compileResult?.spec?.workflows?.some(saved => saved.id === workflow.id) && (
                  <Button type="button" variant="outline" size="sm" className="h-7 px-2 text-xs"
                    disabled={busy !== null || repairActive || conversationActive}
                    onClick={() => { setActionError(null); setCopyTarget(workflow.id!); }}>
                    {t('forge.preview.saveCopy')}
                  </Button>
                )}
              </div>
              {copyTarget && copyTarget === workflow.id && <ForgeCopyForm key={copyTarget} workflowName={workflow.name}
                disabled={busy !== null || repairActive || conversationActive}
                onCancel={() => { setCopyTarget(null); setActionError(null); }}
                onSave={name => void handleCompile({ workflowId: copyTarget, name })} />}
              <ol className="mt-1.5 space-y-2">
                {forgeIndexedStages(spec).filter((entry) => entry.workflowIndex === index).map((entry) => (
                  <ForgeStageCard
                    key={entry.id}
                    domId={`${cardIdPrefix}-${entry.id}`}
                    resource={resource}
                    spec={spec}
                    entry={entry}
                    changed={changedStageIds.has(entry.id)}
                    focused={focusedStageId === entry.id}
                    editable={Boolean(entry.stage.id) && !conversationActive && !repairActive}
                    conversationActive={conversationActive}
                    canPrefill={Boolean(onPrefillPrompt)}
                    onPrefill={() => {
                      setFocusedStageId(entry.id);
                      onPrefillPrompt?.(t('forge.preview.modifyStagePrompt', { index: entry.ordinal, name: stageTitle(t, entry.stage) }));
                    }}
                    onSpecChange={acceptSpec}
                  />
                ))}
              </ol>
            </section>
          ))}
        </div>
      </div>
    </TooltipProvider>
  );
}

function ForgeCopyForm({ workflowName, disabled, onSave, onCancel }: { workflowName: string; disabled: boolean; onSave: (name: string) => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  return <form data-forge-copy className="flex flex-wrap items-center gap-2 px-3 pb-2" onSubmit={event => {
    event.preventDefault();
    if (!disabled && name.trim()) onSave(name.trim());
  }}>
    <Input autoFocus value={name} onChange={event => setName(event.target.value)} className="min-w-28 flex-1" disabled={disabled}
      aria-label={t('forge.preview.copyName', { name: workflowName })} placeholder={t('forge.preview.copyName', { name: workflowName })} />
    <Button type="submit" size="sm" disabled={disabled || !name.trim()}>{t('forge.preview.saveCopy')}</Button>
    <Button type="button" variant="ghost" size="sm" disabled={disabled} onClick={onCancel}>{t('common.cancel')}</Button>
  </form>;
}

function PanelMessage({ icon, text, action }: { icon: React.ReactNode; text: string; action?: React.ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-sm text-muted-foreground">
      <span className="inline-flex items-center gap-2">
        {icon}
        {text}
      </span>
      {action}
    </div>
  );
}

export const ForgePreviewPanel = memo(ForgePreviewPanelComponent);

/** 待澄清：真正影响流程走向、需要用户拍板的事项。有推荐答案的可以一键采用。 */
function ForgeOpenQuestions({
  questions,
  pending,
  notice,
  disabled,
  onAdopt,
  onAdoptAll,
}: {
  questions: ForgeQuestionVm[];
  pending: ReadonlySet<string>;
  notice: AdoptionState['notice'];
  disabled: boolean;
  onAdopt: (item: ForgeQuestionVm) => void;
  onAdoptAll: () => void;
}) {
  const { t } = useTranslation();
  const adoptable = questions.filter((item) => item.recommendation);
  return (
    <section data-forge-open-questions className="mb-3 rounded-lg border border-border/60 bg-card px-3 py-2.5">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-medium text-foreground">
          {t('forge.preview.openQuestions')}
          {questions.length > 1 ? <span className="ml-1.5 font-normal text-muted-foreground">{questions.length}</span> : null}
        </h3>
        {adoptable.length > 1 ? (
          <Button
            data-forge-adopt-all
            size="sm"
            variant="outline"
            className="h-6 px-2 text-xs"
            disabled={disabled}
            onClick={onAdoptAll}
          >
            {t('forge.preview.adoptAll')}
          </Button>
        ) : null}
      </div>
      <ol className="mt-1.5 space-y-2">
        {questions.map((item, index) => (
          <li key={`${index}:${item.question}`} data-forge-question className="flex items-start gap-2 text-xs">
            <span className="mt-px inline-flex size-4.5 shrink-0 items-center justify-center rounded-full bg-foreground/10 text-[0.65rem] font-medium tabular-nums text-foreground">
              {index + 1}
            </span>
            <div className="min-w-0 flex-1">
              <p className="break-words text-foreground">{item.question}</p>
              {item.recommendation ? (
                <p className="mt-0.5 break-words text-muted-foreground" data-forge-recommendation>
                  {t('forge.preview.recommendationLabel')}: {item.recommendation}
                </p>
              ) : null}
            </div>
            {item.recommendation ? (
              pending.has(item.question) ? (
                <span className="inline-flex shrink-0 items-center gap-1 text-muted-foreground" data-forge-adopt-pending>
                  <Loader2 className="size-3 animate-spin" />
                  {t('forge.preview.adoptSent')}
                </span>
              ) : (
                <Button
                  size="sm"
                  variant="outline"
                  className="h-6 shrink-0 px-2 text-xs"
                  disabled={disabled}
                  onClick={() => onAdopt(item)}
                >
                  {t('forge.preview.adoptRecommendation')}
                </Button>
              )
            ) : null}
          </li>
        ))}
      </ol>
      {notice ? (
        <p role="alert" className="mt-1.5 text-xs text-muted-foreground">
          {t(notice === 'busy' ? 'forge.repair.busy' : 'forge.preview.adoptUnavailable')}
        </p>
      ) : null}
      <p className="mt-1.5 text-xs text-muted-foreground">{t('forge.preview.compileBlocked')}</p>
    </section>
  );
}

/// goal 四要素的结构化解析:接受协议示例的 `<输入>` 标记与模型常见的
/// 「输入：…；」前缀两种形式;命中不足两段视为不可结构化,调用方回退整段展示。
type GoalElements = { key: 'input' | 'output' | 'boundary' | 'acceptance'; text: string }[];

function parseGoalElements(goal: ForgeStageVm['goal']): GoalElements | null {
  if (typeof goal !== 'string') {
    if ('original' in goal) return null;
    return (['input', 'output', 'boundary', 'acceptance'] as const).map(key => ({ key, text: goal[key] }));
  }
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

/// stage 的角色名:内置角色是其种类,custom 角色是生成时起的 profile 名。
function stageRoleName(stage: ForgeStageVm): string {
  return typeof stage.role === 'string' ? stage.role : stage.role.custom.profileName;
}

/// 角色的展示文案(内置角色按界面语言本地化)。
function stageRoleLabel(t: TFunction, stage: ForgeStageVm): string {
  const role = stageRoleName(stage);
  return typeof stage.role === 'string' && ['dev', 'test', 'review', 'plan', 'accept', 'unassigned'].includes(role) ? t(`forge.preview.role.${role}`) : role;
}

/// 与编辑器保持一致：节点名标识步骤；不同步骤可以使用相同的产物名。
function stageTitle(t: TFunction, stage: ForgeStageVm): string {
  return stage.id?.trim() || stageRoleLabel(t, stage);
}

function stageOptionLabel(t: TFunction, entry: ForgeIndexedStage): string {
  return `${entry.ordinal}. ${stageTitle(t, entry.stage)}`;
}

/// 失败去向的白话说明；跳转与“从指定阶段重开”带上目标阶段。
function failureLabel(t: TFunction, stage: ForgeStageVm, stages: ForgeIndexedStage[]): string {
  const kind = forgeOnFailureKind(stage.onFailure);
  const target = forgeOnFailureTarget(stage.onFailure);
  if (target) {
    const targetStage = stages.find((entry) => entry.stage.id === target)?.stage;
    const name = targetStage ? stageTitle(t, targetStage) : target;
    return kind === 'jump'
      ? t(forgeOnFailureContinuesSession(stage.onFailure) ? 'forge.preview.onFailure.jump-continue-detail' : 'forge.preview.onFailure.jump-detail', { stage: name })
      : t('forge.preview.onFailure.new-round-from-detail', { stage: name });
  }
  return t(`forge.preview.onFailure.${kind}`);
}

/// 阶段卡片：目标四要素 + 可直接修改的失败策略；技术字段默认折叠。
function ForgeStageCard({
  domId,
  resource,
  spec,
  entry,
  changed,
  focused,
  editable,
  conversationActive,
  canPrefill,
  onPrefill,
  onSpecChange,
}: {
  domId: string;
  resource: ForgePreviewWorkspaceResource;
  spec: ForgeSpecVm;
  entry: ForgeIndexedStage;
  changed: boolean;
  focused: boolean;
  /** 没有稳定 id 的阶段、对话回复中、自动修正进行中都不能直接改。 */
  editable: boolean;
  conversationActive: boolean;
  canPrefill: boolean;
  onPrefill: () => void;
  onSpecChange: (spec: ForgeSpecVm) => void;
}) {
  const { t } = useTranslation();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const { stage } = entry;
  const roleLabel = stageRoleLabel(t, stage);
  const title = stageTitle(t, stage);
  const elements = parseGoalElements(stage.goal ?? '');
  const branch = spec.workflows?.[entry.workflowIndex]?.parallelGroups?.flatMap(g =>
    g.branches.filter(b => b.stages.includes(entry.id)).map(b => ({group: g.id, branch: b.id})),
  )[0];
  return (
    <li
      id={domId}
      data-forge-stage-card={entry.id}
      className={cn(
        'rounded-lg border bg-card px-3.5 py-2.5 transition-colors',
        focused ? 'border-primary/60' : 'border-border/60',
        changed && 'border-primary/40',
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          className="group flex min-w-0 flex-1 items-center gap-2 text-left"
          disabled={!canPrefill}
          onClick={onPrefill}
          aria-label={canPrefill ? t('forge.preview.modifyStagePrompt', { index: entry.ordinal, name: title }) : undefined}
        >
          <span className="inline-flex size-4.5 shrink-0 items-center justify-center rounded-full bg-foreground/10 text-[0.65rem] font-medium tabular-nums text-foreground">
            {entry.ordinal}
          </span>
          <span className="min-w-0 truncate text-xs font-medium text-foreground">{title}</span>
        </button>
        {changed ? (
          <span className="shrink-0 rounded border border-primary/35 bg-primary/10 px-1.5 py-px text-[0.65rem] text-primary">
            {t('forge.preview.changedBadge')}
          </span>
        ) : null}
        <span className="shrink-0 rounded border border-border/60 px-1.5 py-px text-[0.65rem] text-muted-foreground">{roleLabel}</span>
        {canPrefill ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex shrink-0">
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-5"
                  aria-label={t('forge.preview.modifyStage')}
                  onClick={(event) => { event.stopPropagation(); onPrefill(); }}
                >
                  <Pencil className="size-3" />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{t('forge.preview.modifyStage')}</TooltipContent>
          </Tooltip>
        ) : null}
      </div>
      {branch && <p className="mt-1 break-all text-xs text-muted-foreground">{t('forge.preview.parallelBranch', branch)}</p>}
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
        <p className="mt-2 whitespace-pre-wrap break-words text-xs leading-relaxed text-foreground/85">{typeof stage.goal === 'string' ? stage.goal : 'original' in stage.goal ? stage.goal.original : null}</p>
      )}
      <div className="mt-2 border-t border-border/40 pt-2">
        <FailureStrategyControl
          resource={resource}
          spec={spec}
          entry={entry}
          editable={editable}
          conversationActive={conversationActive}
          onSpecChange={onSpecChange}
        />
      </div>
      <Collapsible open={detailsOpen} onOpenChange={setDetailsOpen}>
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="mt-1.5 inline-flex items-center gap-1 text-[0.7rem] text-muted-foreground hover:text-foreground"
            aria-expanded={detailsOpen}
          >
            {t('forge.preview.technicalDetails')}
            <ChevronDown className={cn('size-3 transition-transform', detailsOpen && 'rotate-180')} />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-1.5 text-[0.7rem] leading-relaxed text-muted-foreground">
          <dl className="space-y-1">
            <div className="flex gap-2">
              <dt className="w-14 shrink-0">{t('forge.preview.outputKindLabel')}</dt>
              <dd>{t(`forge.preview.outputKind.${stage.outputKind}`)}{stage.artifact ? ` · ${stage.artifact}` : ''}</dd>
            </div>
            {stage.successCondition ? (
              <div className="flex gap-2">
                <dt className="w-14 shrink-0">{t('forge.preview.successCondition')}</dt>
                <dd><code className="break-all rounded bg-muted/40 px-1 py-px font-mono text-[0.65rem]">{stage.successCondition}</code></dd>
              </div>
            ) : null}
            {stage.roleDefinition ? (
              <div className="flex gap-2">
                <dt className="w-14 shrink-0">{t('forge.preview.roleDefinition')}</dt>
                <dd className="min-w-0 flex-1 whitespace-pre-wrap break-words">{stage.roleDefinition}</dd>
              </div>
            ) : null}
          </dl>
        </CollapsibleContent>
      </Collapsible>
    </li>
  );
}

/// 失败策略：下拉选择；跳转与“从指定阶段重开”再选一个目标阶段。改动经后端落档后
/// 以返回的草稿为准更新界面。
function FailureStrategyControl({
  resource,
  spec,
  entry,
  editable,
  conversationActive,
  onSpecChange,
}: {
  resource: ForgePreviewWorkspaceResource;
  spec: ForgeSpecVm;
  entry: ForgeIndexedStage;
  editable: boolean;
  conversationActive: boolean;
  onSpecChange: (spec: ForgeSpecVm) => void;
}) {
  const { t } = useTranslation();
  const kind = forgeOnFailureKind(entry.stage.onFailure);
  const currentTarget = forgeOnFailureTarget(entry.stage.onFailure);
  const continuesSession = forgeOnFailureContinuesSession(entry.stage.onFailure);
  const targets = useMemo(() => forgeFailureTargets(spec, entry.id), [spec, entry.id]);
  const groups = spec.workflows?.[entry.workflowIndex]?.parallelGroups ?? [];
  const member = groups.some(g => g.branches.some(b => b.stages.includes(entry.id)));
  const groupTargets = member ? [] : groups;
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const apply = async (nextKind: ForgeOnFailureKind, options: { target: string | null; continueSession: boolean }) => {
    const value = forgeOnFailureValue(nextKind, { ...options, continueSession: options.continueSession && !groupTargets.some(g => g.id === options.target) });
    if (!value || !entry.stage.id) return;
    setSaving(true);
    setError(null);
    try {
      onSpecChange(await forgeSetStageFailure(resource.projectId, resource.taskId, entry.stage.id, value));
    } catch (failure) {
      setError(forgeCommandErrorText(t, failure) ?? displayAppError(t, failure));
    } finally {
      setSaving(false);
    }
  };

  // 默认目标：紧邻的上一个阶段；已经是第一阶段时取下一个阶段。
  const defaultTarget = [...targets].reverse().find((candidate) => candidate.ordinal < entry.ordinal)?.id
    ?? targets[0]?.id ?? groupTargets[0]?.id
    ?? null;
  const needsTarget = kind === 'jump' || kind === 'new-round-from';
  const disabled = !editable || saving;

  return (
    <div data-forge-stage-failure={entry.id}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-xs">
        <span className="shrink-0 text-muted-foreground">{t('forge.preview.onFailureLabel')}</span>
        <Select
          value={kind}
          disabled={disabled}
          onValueChange={(value) => {
            const next = value as ForgeOnFailureKind;
            void apply(next, {
              target: currentTarget ?? defaultTarget,
              // 新改成跳转时默认续接目标阶段的会话；已经是跳转的保持原设置。
              continueSession: next === 'jump' ? (kind === 'jump' ? continuesSession : true) : true,
            });
          }}
        >
          <SelectTrigger size="sm" className="h-6 w-auto max-w-[11rem] text-xs" aria-label={t('forge.preview.onFailureLabel')}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {FAILURE_KINDS.filter(k => !member || ['end', 'retry', 'jump'].includes(k)).map((candidate) => (
              <SelectItem
                key={candidate}
                value={candidate}
                disabled={(candidate === 'jump' || candidate === 'new-round-from') && targets.length + groupTargets.length === 0}
              >
                {t(`forge.preview.onFailure.${candidate}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {needsTarget ? (
          <Select
            value={currentTarget ?? ''}
            disabled={disabled}
            onValueChange={(target) => void apply(kind, { target, continueSession: kind === 'jump' ? continuesSession : true })}
          >
            <SelectTrigger
              size="sm"
              className="h-6 w-auto max-w-[11rem] text-xs"
              aria-label={t(kind === 'jump' ? 'forge.preview.failureTargetLabel' : 'forge.preview.restartTargetLabel')}
            >
              <SelectValue placeholder={t('forge.preview.failureTargetPlaceholder')} />
            </SelectTrigger>
            <SelectContent>
              {targets.map((candidate) => (
                <SelectItem key={candidate.id} value={candidate.id}>{stageOptionLabel(t, candidate)}</SelectItem>
              ))}
              {groupTargets.map(g => <SelectItem key={g.id} value={g.id}>{g.id}</SelectItem>)}
            </SelectContent>
          </Select>
        ) : null}
        {kind === 'jump' && !groupTargets.some(g => g.id === currentTarget) ? (
          <label className="inline-flex items-center gap-1.5 text-muted-foreground">
            <Checkbox
              className="size-3.5"
              checked={continuesSession}
              disabled={disabled}
              onCheckedChange={(checked) => void apply('jump', { target: currentTarget, continueSession: checked === true })}
            />
            {t('forge.preview.continueSessionLabel')}
          </label>
        ) : null}
        {saving ? <Loader2 className="size-3 animate-spin text-muted-foreground" aria-label={t('common.loading')} /> : null}
      </div>
      {error ? <p role="alert" className="mt-1 text-xs text-destructive">{error}</p> : null}
      {!editable && entry.stage.id && conversationActive ? (
        <p className="mt-1 text-[0.7rem] text-muted-foreground">{t('forge.preview.conversationActiveHint')}</p>
      ) : null}
    </div>
  );
}

/// Spec → 蓝图投影(§6.3/§7.2:对话与图同屏生长)。纯前端投影,Spec
/// 结构事件(轮完成存档刷新)驱动整图重建——≤10 节点,重建成本可忽略,
/// 与 token 流完全解耦。
///
/// 阶段名是卡片标题，「角色 · 失败去向」是第二行；链尾接「结束」端点。失败即终止的
/// 阶段有一条失败边指向「结束」，开启新一轮的指向「新一轮」端点，跳回前面阶段的连回
/// 那个阶段。原阶段重试与人工复核没有边，只在第二行与阶段卡片上体现。
function specToGraph(spec: ForgeSpecVm, t: TFunction): GraphVm {
  const nodes: GraphVm['nodes'] = [];
  const edges: GraphVm['edges'] = [];
  const idle = { code: 'draft', tone: 'neutral', icon: 'dot', resumable: false, blockingError: false } as const;
  const entries = forgeIndexedStages(spec);
  const addWorker = (entry: ForgeIndexedStage) => {
    const { stage } = entry;
    const role = stageRoleLabel(t, stage);
    nodes.push({
      id: entry.id,
      nodeId: entry.id,
      label: stageTitle(t, stage),
      nodeType: 'worker',
      runtimeDisplay: { ...idle, terminal: false },
      artifactCount: stage.outputKind === 'json' ? 1 : 0,
      attachmentCount: 0,
      current: false,
      retryable: false,
      attemptCount: 0,
      attempts: [],
      detail: `${role} · ${failureLabel(t, stage, entries)}`,
    });
  };
  // Marker appearance does not imply a sink: start, fork and join have outgoing links.
  const terminal = (id: string, label: string): GraphVm['nodes'][number] => ({
    id,
    label,
    nodeType: 'terminal',
    runtimeDisplay: { ...idle, terminal: true },
    artifactCount: 0,
    attachmentCount: 0,
    current: false,
    retryable: false,
    attemptCount: 0,
    attempts: [],
  });
  for (const [index, workflow] of (spec.workflows ?? []).entries()) {
    const steps = entries.filter(e => e.workflowIndex === index);
    if (!steps.length) continue;
    const prefix = `$forge:${workflow.id || index}:`;
    const groups = new Map((workflow.parallelGroups ?? []).map(g => [g.id, g]));
    const inlet = (id: string) => groups.has(id) ? `${prefix}fork:${id}` : id;
    const outlet = (id: string) => groups.has(id) ? `${prefix}join:${id}` : id;
    const owner = new Map<string, string>();
    const connect = (from: string, to: string, label = 'success') => edges.push({from, to, label});
    const failure = (from: string, strategy: ForgeOnFailureVm, end: string) => {
      const kind = forgeOnFailureKind(strategy);
      if (kind === 'end') connect(from, end, 'failure');
      if (kind === 'new-round' || kind === 'new-round-from') connect(from, NEW_ROUND_NODE, 'failure');
      if (kind === 'jump') {
        const target = forgeOnFailureTarget(strategy);
        if (target && inlet(target) !== from) connect(from, inlet(target), 'failure');
      }
    };
    for (const group of groups.values()) {
      for (const branch of group.branches) for (const member of branch.stages) owner.set(member, group.id);
    }
    const outer = [...new Set(steps.map(s => owner.get(s.id) ?? s.id))];
    const byId = new Map(steps.map(step => [step.id, step]));
    const start = `${prefix}start`;
    nodes.push(terminal(start, t('workflowEditor.nodeLabels.start')));
    connect(start, inlet(outer[0]));
    // ELK breaks cycles by model order. Emit the success path, with feedback
    // links pointing backwards, just as the authoring graph does.
    for (const id of outer) {
      const group = groups.get(id);
      if (!group) {
        addWorker(byId.get(id)!);
        continue;
      }
      const fork = inlet(id), join = outlet(id);
      nodes.push(terminal(fork, t('forge.preview.parallelFork', {id, cap: group.maxParallel})));
      for (const branch of group.branches) {
        let previous = fork;
        for (const member of branch.stages) {
          const step = byId.get(member);
          if (!step) continue;
          addWorker(step);
          connect(previous, member);
          previous = member;
        }
        connect(previous, join);
      }
      nodes.push(terminal(join, t('forge.preview.parallelJoin', {id})));
      if (group.onFailure === 'retry') connect(join, fork, 'failure');
      else failure(join, group.onFailure, END_NODE);
    }
    outer.forEach((id, i) => connect(outlet(id), outer[i + 1] ? inlet(outer[i + 1]) : END_NODE));
    for (const step of steps) {
      const group = owner.get(step.id);
      // A failed leaf settles its own branch, never ends the entire run directly.
      if (!group || step.stage.onFailure !== 'end') failure(step.id, step.stage.onFailure, END_NODE);
    }
  }
  if (edges.some((edge) => edge.to === END_NODE)) nodes.push(terminal(END_NODE, t('workflowEditor.nodeLabels.end')));
  if (edges.some((edge) => edge.to === NEW_ROUND_NODE)) nodes.push(terminal(NEW_ROUND_NODE, t('status.new-round')));
  return { nodes, edges, groups: [] };
}

/** 蓝图画布的适配边距（每侧）：画布外框已经留白，不再让适配视图重复留白。 */
const BLUEPRINT_FIT_PADDING = 0.05;
/** 蓝图画布高度的下限与上限（像素）：上限之内卡片按接近原始大小显示，超出后缩小。 */
const BLUEPRINT_MIN_HEIGHT = 230;
const BLUEPRINT_MAX_HEIGHT = 560;

/// 蓝图画布的高度：按「阶段卡片纵向堆叠 + 端点 + 布局留白」的内容高度，反推出让
/// 适配缩放落在上限附近所需的画布高度，并封顶。
///
/// `GraphView` 跟随父容器高度（其它页面把它放在撑满高度的容器里）；这里的父容器在
/// 滚动区里，必须自己给出确定高度，否则画布塌缩成一条线——图渲染了却看不见。
function forgeGraphHeight(graph: GraphVm): number {
  // The projection is already in success-topology order. Budget the longest
  // path, not the sum of parallel siblings or a failure/retry detour.
  const predecessors = new Map<string, string[]>();
  for (const edge of graph.edges) {
    if (edge.label !== 'success') continue;
    const incoming = predecessors.get(edge.to);
    if (incoming) incoming.push(edge.from);
    else predecessors.set(edge.to, [edge.from]);
  }
  const bottom = new Map<string, number>();
  for (const node of graph.nodes) {
    const top = Math.max(0, ...(predecessors.get(node.id) ?? []).map(id => (bottom.get(id) ?? 0) + LAYER_SPACING));
    bottom.set(node.id, top + (node.nodeType === 'worker' ? NODE_HEIGHT : TERMINAL_NODE_HEIGHT));
  }
  const content = Math.max(0, ...bottom.values()) + LAYOUT_PADDING * 2;
  const height = Math.ceil(content * WORKFLOW_FIT_MAX_ZOOM / (1 - BLUEPRINT_FIT_PADDING * 2));
  return Math.min(BLUEPRINT_MAX_HEIGHT, Math.max(BLUEPRINT_MIN_HEIGHT, height));
}

function ForgeSpecGraph({
  spec,
  changedStageIds,
  focusedStageId,
  onFocusStage,
}: {
  spec: ForgeSpecVm;
  changedStageIds: ReadonlySet<string>;
  focusedStageId: string | null;
  onFocusStage: (stageId: string) => void;
}) {
  const { t } = useTranslation();
  // 侧栏又窄又高：蓝图默认纵向排布（自动方向会选横向，把卡片缩到看不清）；
  // 画布控件里仍然可以切换，且只影响这一张图。
  const [direction, setDirection] = useState<WorkflowGraphDirectionPreference>('vertical');
  const graph = useMemo(() => specToGraph(spec, t), [spec, t]);
  const height = useMemo(() => forgeGraphHeight(graph), [graph]);
  const handleNodeSelect = useCallback((node: GraphVm['nodes'][number]) => {
    if (node.nodeType === 'worker') onFocusStage(node.id);
  }, [onFocusStage]);
  if (graph.nodes.length === 0) return null;
  return (
    <div data-forge-spec-graph className="mb-3" style={{ height }}>
      <WorkflowGraphDirectionProvider value={{ preference: direction, setPreference: setDirection }}>
        <GraphView
          graph={graph}
          variant="workflow"
          allowFullscreen={false}
          density="compact"
          fitPadding={BLUEPRINT_FIT_PADDING}
          highlightedNodeIds={changedStageIds}
          selectedNodeId={focusedStageId}
          onNodeSelect={handleNodeSelect}
        />
      </WorkflowGraphDirectionProvider>
    </div>
  );
}
