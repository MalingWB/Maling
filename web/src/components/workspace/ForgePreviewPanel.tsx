import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2 } from 'lucide-react';
import { forgeGetSpec } from '../../api';
import type { ForgePreviewWorkspaceResource } from './right-workspace-context';
import type { ForgeSpecVm, ForgeStageVm } from '../../types';

/**
 * 铸造预览（方案 §5.2,第一阶段视图）：Forge 会话的 Spec 只读摘要。
 *
 * Spec 是会话唯一事实源,Tab 只是其投影——打开/重开时从 task 级存档
 * 全量读取(≤10 节点,成本可忽略);对话轮次推进后由持有方刷新重开。
 * 第三阶段在同 Tab 原位切换为流程图与编辑器预览,本组件届时退役为
 * 摘要区块。
 */
export function ForgePreviewPanel({ resource }: { resource: ForgePreviewWorkspaceResource }) {
  const { t } = useTranslation();
  const [spec, setSpec] = useState<ForgeSpecVm | null | undefined>(undefined);

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

  useEffect(() => load(), [load]);

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
    <div data-right-workspace-resource="forge-preview" className="min-h-0 flex-1 overflow-y-auto p-3">
      <p className="text-xs text-muted-foreground">{spec.intent}</p>
      {spec.workflows.map((workflow, index) => (
        <section key={`${workflow.name}-${index}`} className="mt-3">
          <h3 className="text-sm font-medium">{workflow.name}</h3>
          <ol className="mt-1.5 space-y-2">
            {workflow.stages.map((stage, stageIndex) => (
              <ForgeStageRow key={stage.id ?? stageIndex} stage={stage} index={stageIndex + 1} />
            ))}
          </ol>
        </section>
      ))}
      {spec.openQuestions.length > 0 ? (
        <section className="mt-3">
          <h3 className="text-sm font-medium">{t('forge.preview.openQuestions')}</h3>
          <ul className="mt-1.5 list-disc space-y-1 pl-5 text-xs text-muted-foreground">
            {spec.openQuestions.map((question, index) => (
              <li key={index}>{question}</li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

function ForgeStageRow({ stage, index }: { stage: ForgeStageVm; index: number }) {
  const { t } = useTranslation();
  const role = typeof stage.role === 'string' ? stage.role : stage.role.custom['profile-name'];
  return (
    <li className="rounded-md border border-border/60 bg-card px-3 py-2">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium text-foreground">{index}. {stage.artifact ?? t(`forge.preview.role.${role}`)}</span>
      </div>
      <p className="mt-1 whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground">{stage.goal}</p>
      <div className="mt-1.5 flex flex-wrap gap-x-4 gap-y-1 text-[0.7rem] text-muted-foreground">
        <span>{t('forge.preview.outputKindLabel')}: {t(`forge.preview.outputKind.${stage.outputKind}`)}</span>
        {stage.successCondition ? <span>{t('forge.preview.successCondition')}: {stage.successCondition}</span> : null}
        <span>{t('forge.preview.onFailureLabel')}: {t(`forge.preview.onFailure.${stage.onFailure}`)}</span>
      </div>
    </li>
  );
}
