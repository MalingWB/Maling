import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { decideUpstreamPull, subscribeUpstreamPullEvents } from '@/api';
import { useReadOnlyExperience } from '@/components/ReadOnlyExperience';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Textarea } from '@/components/ui/textarea';
import { displayAppError } from '@/i18n';
import type { MulticaUpstreamPullEventVm, UpstreamPullDecisionKind } from '@/types';

/// pending 槽唯一键（run + node）；settled 收口与本地移除都按此匹配（幂等）。
function pullKey(event: MulticaUpstreamPullEventVm) {
  return `${event.runId}/${event.nodeId}`;
}

/// 后端结构化错误形如 { code, params }（AppErrorVm）；迟到判定只看 code，不依赖文案。
function isDecisionNotPending(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as { code?: unknown }).code === 'multica.upstream-pull-not-pending'
  );
}

/**
 * multica 上游拉取决策弹窗（M5-ch 决策点，App 级全局挂载）。
 *
 * 拉取解析器首拉空值（父工作项尚未产出 completion-output）时经 pending 事件打开：用户三选一
 * ——继续等待拉取（恢复轮询）/ 手动提供交付说明 / 跳过拉取直接继续。60 秒未选择由后端自动
 * 回落轮询（无人值守不挂死），settled 事件统一收口关窗（resolved/manual/skipped/timeout）；
 * 多个在飞拉取按到达顺序逐个弹（同一时刻只展示队首）。用户直接关窗 = 不做决策（回落轮询
 * 自愈，settled 到达时幂等清理）。
 */
export function MulticaUpstreamPullDialog() {
  const { t } = useTranslation();
  const readOnly = useReadOnlyExperience();
  const [pendings, setPendings] = useState<MulticaUpstreamPullEventVm[]>([]);
  const [mode, setMode] = useState<'choice' | 'manual'>('choice');
  const [manualText, setManualText] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void subscribeUpstreamPullEvents((event) => {
      if (!active) return;
      const key = pullKey(event);
      if (event.outcome === null) {
        // pending：入队（幂等，重复事件不重复弹）。
        setPendings((current) =>
          current.some((item) => pullKey(item) === key) ? current : [...current, event],
        );
      } else {
        // settled：收口移除（用户已提交或直接关窗后的迟到事件是幂等 no-op）。
        setPendings((current) => current.filter((item) => pullKey(item) !== key));
      }
    }).then((unlisten) => {
      if (active) unsubscribe = unlisten;
      else unlisten();
    });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, []);

  const current = pendings[0] ?? null;
  // 队首切换（新拉取入队/收口）时重置表单态：不继承上一条的手动草稿与错误。
  useEffect(() => {
    setMode('choice');
    setManualText('');
    setError(null);
  }, [current && pullKey(current)]);

  const trimmedManual = manualText.trim();
  const canSubmitManual = mode === 'choice' || trimmedManual !== '';

  async function submit(decision: UpstreamPullDecisionKind) {
    if (!current || submitting) return;
    if (decision !== 'manual' && readOnly) return;
    setSubmitting(true);
    setError(null);
    try {
      await decideUpstreamPull(
        current.taskId,
        current.nodeId,
        decision,
        decision === 'manual' ? trimmedManual : null,
      );
      // 本地立即出队（settled 事件随 resolve 收口到达，移除幂等）。
      setPendings((items) => items.filter((item) => pullKey(item) !== pullKey(current)));
    } catch (err) {
      if (isDecisionNotPending(err)) {
        // 决策迟到：拉取已收口（弹窗通常已被 settled 关掉），幂等清理即可。
        setPendings((items) => items.filter((item) => pullKey(item) !== pullKey(current)));
        return;
      }
      // 其他失败保留弹窗让用户重选（后端槽未被写入，等待方仍在）。
      setError(displayAppError(t, err));
    } finally {
      setSubmitting(false);
    }
  }

  function dismissCurrent() {
    if (!current) return;
    // 直接关窗 = 不做决策：后端 60s 回落轮询自愈；settled 到达时幂等清理。
    setPendings((items) => items.filter((item) => pullKey(item) !== pullKey(current)));
  }

  if (!current) return null;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) dismissCurrent(); }}>
      <DialogContent className="max-w-md gap-0 p-0">
        <DialogHeader className="p-6 pb-0">
          <DialogTitle>{t('remote.upstreamPull.title')}</DialogTitle>
          <DialogDescription>
            {t('remote.upstreamPull.description', {
              task: current.taskTitle ?? current.taskId,
              node: current.nodeId,
              issue: current.parentIssueId,
            })}
          </DialogDescription>
        </DialogHeader>

        {mode === 'choice' ? (
          <div className="space-y-3 p-6">
            <p className="text-xs text-muted-foreground">{t('remote.upstreamPull.fallbackNote')}</p>
            {error && <p className="text-xs text-destructive">{error}</p>}
          </div>
        ) : (
          <div className="space-y-2 p-6">
            <Textarea
              value={manualText}
              onChange={(e) => setManualText(e.target.value)}
              disabled={submitting}
              placeholder={t('remote.upstreamPull.manualPlaceholder')}
              className="min-h-28 font-mono text-xs"
              aria-label={t('remote.upstreamPull.manualLabel')}
            />
            {error && <p className="text-xs text-destructive">{error}</p>}
          </div>
        )}

        <DialogFooter className="flex-wrap border-t border-border/60 p-6 pt-4">
          {mode === 'choice' ? (
            <>
              <Button
                variant="outline"
                disabled={readOnly || submitting}
                onClick={() => void submit('skip')}
              >
                {t('remote.upstreamPull.skip')}
              </Button>
              <Button
                variant="outline"
                disabled={readOnly || submitting}
                onClick={() => {
                  setMode('manual');
                  setError(null);
                }}
              >
                {t('remote.upstreamPull.manual')}
              </Button>
              <Button disabled={readOnly || submitting} onClick={() => void submit('retry')}>
                {t('remote.upstreamPull.retry')}
              </Button>
            </>
          ) : (
            <>
              <Button
                variant="ghost"
                disabled={submitting}
                onClick={() => {
                  setMode('choice');
                  setError(null);
                }}
              >
                {t('common.back')}
              </Button>
              <Button
                disabled={!canSubmitManual || submitting}
                onClick={() => void submit('manual')}
              >
                {t('remote.upstreamPull.manualSubmit')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
