import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Hammer, Loader2, Trash2, TriangleAlert } from 'lucide-react';
import { forgeCleanupSessions, forgeListSessions } from '../api';
import type { ForgeSessionEntryVm } from '../types';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { Badge } from '@/components/ui/badge';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';

/**
 * 铸造历史（方案 G2/D3）：Forge 铸造与试运行会话不进默认列表，统一在
 * 此入口管理与清理。列表按项目作用域读取（会话与试运行挂靠项目）；
 * 清理是删除任务目录的不可逆操作，逐项/批量均需 AlertDialog 确认。
 */
export function ForgeSessionHistoryPage({ projectId }: { projectId: string }) {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<ForgeSessionEntryVm[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [cleanupTarget, setCleanupTarget] = useState<'selected' | 'unfinished' | null>(null);
  const [cleanupBusy, setCleanupBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const listRequestIdRef = useRef(0);

  const loadSessions = useCallback(() => {
    const requestId = ++listRequestIdRef.current;
    setLoadFailed(false);
    forgeListSessions(projectId)
      .then((next) => {
        if (requestId !== listRequestIdRef.current) return;
        setSessions(next);
        setSelected(new Set());
      })
      .catch(() => {
        if (requestId === listRequestIdRef.current) setLoadFailed(true);
      });
  }, [projectId]);

  useEffect(() => {
    setSessions(null);
    loadSessions();
  }, [loadSessions]);

  const toggleSelected = (taskId: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  };

  const unfinishedIds = (sessions ?? [])
    .filter((session) => session.origin === 'forge' && (session.specSummary?.workflowCount ?? 0) === 0)
    .map((session) => session.taskId);

  const cleanupIds = cleanupTarget === 'selected'
    ? [...selected]
    : cleanupTarget === 'unfinished'
      ? unfinishedIds
      : [];

  const runCleanup = async () => {
    if (cleanupIds.length === 0 || !projectId) return;
    setCleanupBusy(true);
    setActionError(null);
    try {
      await forgeCleanupSessions(projectId, cleanupIds);
      setCleanupTarget(null);
      loadSessions();
    } catch (err) {
      setActionError(String((err as Error)?.message ?? err));
    } finally {
      setCleanupBusy(false);
    }
  };

  return (
    <TooltipProvider>
    <div data-theme-wallpaper-slot="conversation" className="h-full min-h-0 overflow-y-auto bg-background">
      <div className="mx-auto w-full max-w-3xl px-6 py-8">
        <header className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <Hammer className="size-4 text-foreground" />
            <h1 className="text-base font-medium">{t('forge.history.title')}</h1>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 px-2.5 text-xs"
              disabled={unfinishedIds.length === 0}
              onClick={() => setCleanupTarget('unfinished')}
            >
              <Trash2 className="size-3.5" />
              {t('forge.history.cleanupUnfinished')}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 px-2.5 text-xs"
              disabled={selected.size === 0}
              onClick={() => setCleanupTarget('selected')}
            >
              <Trash2 className="size-3.5" />
              {t('forge.history.cleanupSelected', { count: selected.size })}
            </Button>
          </div>
        </header>
        <p className="mt-1.5 text-xs text-muted-foreground">{t('forge.history.subtitle')}</p>

        {actionError ? (
          <div className="mt-4 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-all">{actionError}</span>
          </div>
        ) : null}

        <div className="mt-5 space-y-2 pb-10">
          {sessions === null && !loadFailed ? (
            <div className="flex items-center justify-center gap-2 py-14 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              {t('forge.history.loading')}
            </div>
          ) : null}
          {loadFailed ? (
            <div className="flex flex-col items-center gap-3 py-14 text-sm text-muted-foreground">
              <span>{t('forge.history.loadFailed')}</span>
              <Button variant="outline" size="sm" className="h-7 text-xs" onClick={loadSessions}>
                {t('common.retry')}
              </Button>
            </div>
          ) : null}
          {sessions !== null && sessions.length === 0 ? (
            <div className="py-14 text-center text-sm text-muted-foreground">{t('forge.history.empty')}</div>
          ) : null}
          {(sessions ?? []).map((session) => (
            <ForgeSessionRow
              key={session.taskId}
              session={session}
              checked={selected.has(session.taskId)}
              onToggle={() => toggleSelected(session.taskId)}
            />
          ))}
        </div>
      </div>

      <AlertDialog open={cleanupTarget !== null} onOpenChange={(open) => { if (!open && !cleanupBusy) setCleanupTarget(null); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('forge.history.cleanupConfirmTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('forge.history.cleanupConfirmDescription', { count: cleanupIds.length })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={cleanupBusy}>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction disabled={cleanupBusy || cleanupIds.length === 0} onClick={(e) => { e.preventDefault(); void runCleanup(); }}>
              {cleanupBusy ? <Loader2 className="size-3.5 animate-spin" /> : null}
              {t('forge.history.cleanupConfirmAction')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
    </TooltipProvider>
  );
}

function ForgeSessionRow({
  session,
  checked,
  onToggle,
}: {
  session: ForgeSessionEntryVm;
  checked: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const summary = session.specSummary;
  return (
    <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border/60 bg-card px-4 py-3 transition-colors hover:bg-sidebar-accent/40">
      <Checkbox
        checked={checked}
        onCheckedChange={() => onToggle()}
        className="mt-1"
        aria-label={t('forge.history.selectSession', { title: session.title ?? session.taskId })}
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="min-w-0 truncate text-sm font-medium">{session.title ?? session.taskId}</span>
          <Badge variant="secondary" className="h-4.5 shrink-0 rounded px-1.5 text-[0.7rem] font-normal">
            {t(session.origin === 'forge-trial' ? 'forge.history.trialBadge' : 'forge.history.sessionBadge')}
          </Badge>
        </div>
        {summary ? (
          <p className="mt-0.5 truncate text-xs text-muted-foreground">
            {summary.name || summary.intent}
            <span className="ml-2">{t('forge.history.stageCount', { count: summary.stageCount })}</span>
            {summary.openQuestionCount > 0 ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="ml-2 text-muted-foreground underline decoration-dotted underline-offset-2">
                    {t('forge.history.openQuestionCount', { count: summary.openQuestionCount })}
                  </span>
                </TooltipTrigger>
                <TooltipContent>{t('forge.history.openQuestionHint')}</TooltipContent>
              </Tooltip>
            ) : null}
          </p>
        ) : (
          <p className="mt-0.5 text-xs text-muted-foreground/70">{t('forge.history.noSpec')}</p>
        )}
      </div>
    </label>
  );
}
