import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CloudDownload } from 'lucide-react';
import { decideUpstreamPull, subscribeUpstreamPullEvents } from '@/api';
import { useReadOnlyExperience } from '@/components/ReadOnlyExperience';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { displayAppError } from '@/i18n';
import type { MulticaUpstreamPullEventVm, UpstreamPullDecisionKind } from '@/types';

/**
 * multica 上游拉取决策卡片（M5-ch 决策点，会话内 InterventionLayer 挂载）。
 *
 * 拉取解析器「确认父未产出」（拉取成功但 completion-output 为空）时经 pending 事件打开：
 * 用户三选一——继续等待拉取（恢复轮询）/ 手动提供交付说明 / 跳过拉取直接继续。60 秒未
 * 选择由后端自动回落轮询（无人值守不挂死），settled 事件统一收口收起（resolved /
 * manual / skipped / timeout）；多个在飞拉取按到达顺序各渲染一张卡片。卡片不阻塞会话，
 * 无关闭按钮——忽略它等价于不决策（60s 回落自愈，settled 到达时幂等收起）。
 */
export function MulticaUpstreamPullIntervention({
  taskId,
  readOnly = false,
}: {
  taskId: string;
  readOnly?: boolean;
}) {
  const [pendings, setPendings] = useState<MulticaUpstreamPullEventVm[]>([]);

  useEffect(() => {
    // 实例按会话挂载，taskId 正常不变；防御性清空以防复用（后端决策槽不受影响）。
    setPendings([]);
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void subscribeUpstreamPullEvents((event) => {
      if (!active || event.taskId !== taskId) return;
      const key = multicaPullKey(event);
      if (event.outcome === null) {
        // pending：入队（幂等，重复事件不重复入队）。
        setPendings((current) =>
          current.some((item) => multicaPullKey(item) === key)
            ? current
            : [...current, event],
        );
      } else {
        // settled：收口移除（用户已提交后的迟到事件是幂等 no-op）。
        setPendings((current) =>
          current.filter((item) => multicaPullKey(item) !== key),
        );
      }
    }).then((unlisten) => {
      if (active) unsubscribe = unlisten;
      else unlisten();
    });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [taskId]);

  if (pendings.length === 0) return null;

  return (
    <>
      {pendings.map((pending) => (
        <MulticaUpstreamPullCard
          key={multicaPullKey(pending)}
          pending={pending}
          readOnly={readOnly}
          onDone={() => {
            // 提交成功/决策迟到：本地立即出队（settled 事件随后到达，移除幂等）。
            // 出队在父级队列而非卡片内部 state——同槽位 pending 再次入队时按 key 重建卡片。
            setPendings((current) =>
              current.filter((item) => multicaPullKey(item) !== multicaPullKey(pending)),
            );
          }}
        />
      ))}
    </>
  );
}

/** pending 槽唯一键（run + node）；settled 收口与本地收起都按此匹配（幂等）。 */
export function multicaPullKey(event: MulticaUpstreamPullEventVm) {
  return `${event.runId}/${event.nodeId}`;
}

/** 后端结构化错误形如 { code, params }（AppErrorVm）；迟到判定只看 code，不依赖文案。 */
function isDecisionNotPending(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as { code?: unknown }).code === 'multica.upstream-pull-not-pending'
  );
}

function MulticaUpstreamPullCard({
  pending,
  readOnly: readOnlyProp = false,
  onDone,
}: {
  pending: MulticaUpstreamPullEventVm;
  readOnly?: boolean;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const readOnly = readOnlyProp || useReadOnlyExperience();
  const [mode, setMode] = useState<'choice' | 'manual'>('choice');
  const [manualText, setManualText] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const trimmedManual = manualText.trim();

  async function submit(decision: UpstreamPullDecisionKind) {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await decideUpstreamPull(
        pending.taskId,
        pending.nodeId,
        decision,
        decision === 'manual' ? trimmedManual : null,
      );
      onDone();
    } catch (err) {
      if (isDecisionNotPending(err)) {
        // 决策迟到：拉取已收口，幂等收起即可。
        onDone();
        return;
      }
      // 其他失败保留卡片让用户重选（后端槽未被写入，等待方仍在）。
      setError(displayAppError(t, err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      data-theme-role="permission-card"
      data-multica-upstream-pull-card="true"
      className="w-full max-w-2xl overflow-hidden px-4 py-3.5"
    >
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-transparent text-muted-foreground">
            <CloudDownload className="size-4" />
          </span>
          <div className="min-w-0">
            <div className="text-ui-compact font-semibold tracking-[-0.01em] text-foreground">
              {t('remote.upstreamPull.title')}
            </div>
            <div className="mt-0.5 break-words text-ui-caption leading-4 text-muted-foreground">
              {t('remote.upstreamPull.description', {
                task: pending.taskTitle ?? pending.taskId,
                node: pending.nodeId,
                issue: pending.parentIssueId,
              })}
            </div>
          </div>
        </div>

        {mode === 'choice' ? (
          <>
            <div className="text-ui-caption text-muted-foreground">
              {t('remote.upstreamPull.fallbackNote')}
            </div>
            {error ? (
              <div className="text-ui-caption text-destructive">{error}</div>
            ) : null}
            <div className="flex min-w-0 flex-wrap gap-2 pl-11">
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-lg border-border/65 bg-transparent px-3 text-xs font-medium shadow-none text-muted-foreground hover:bg-muted/45 hover:text-foreground"
                disabled={readOnly || submitting}
                onClick={() => void submit('skip')}
              >
                {t('remote.upstreamPull.skip')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-lg border-border/65 bg-transparent px-3 text-xs font-medium shadow-none text-muted-foreground hover:bg-muted/45 hover:text-foreground"
                disabled={readOnly || submitting}
                onClick={() => {
                  setMode('manual');
                  setError(null);
                }}
              >
                {t('remote.upstreamPull.manual')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-lg border-border/65 bg-transparent px-3 text-xs font-medium shadow-none text-accent-foreground hover:border-accent-foreground/35 hover:bg-accent/60 hover:text-accent-foreground"
                disabled={readOnly || submitting}
                onClick={() => void submit('retry')}
              >
                {t('remote.upstreamPull.retry')}
              </Button>
            </div>
          </>
        ) : (
          <>
            <Textarea
              value={manualText}
              onChange={(e) => setManualText(e.target.value)}
              disabled={submitting}
              placeholder={t('remote.upstreamPull.manualPlaceholder')}
              className="min-h-28 font-mono text-xs"
              aria-label={t('remote.upstreamPull.manualLabel')}
            />
            {error ? (
              <div className="text-ui-caption text-destructive">{error}</div>
            ) : null}
            <div className="flex min-w-0 flex-wrap justify-end gap-2">
              <Button
                size="sm"
                variant="ghost"
                className="h-8 px-3 text-xs"
                disabled={submitting}
                onClick={() => {
                  setMode('choice');
                  setError(null);
                }}
              >
                {t('common.back')}
              </Button>
              <Button
                size="sm"
                className="h-8 px-3 text-xs"
                disabled={trimmedManual === '' || submitting}
                onClick={() => void submit('manual')}
              >
                {t('remote.upstreamPull.manualSubmit')}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
