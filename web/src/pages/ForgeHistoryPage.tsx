import { useEffect, useRef, useState } from 'react';
import { Folder, History, Loader2, Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { getGenerationHistoryPage } from '@/api';
import { Page, PageHeader } from '@/components/PageScaffold';
import { useReadOnlyExperience } from '@/components/ReadOnlyExperience';
import { ConversationTaskDeleteDialog } from '@/components/conversation/ConversationTaskDeleteDialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { displayAppError } from '@/i18n';
import { formatLocalDateTime } from '@/lib/datetime';
import type { ConversationRecentTaskPageVm, ConversationRecentTaskRowVm, ConversationTaskRowVm, ConversationWorkspaceVm, GenerationHistoryQuery } from '@/types';

type Row = ConversationRecentTaskRowVm;
type Actions = {
  onOpen: (row: Row) => void;
  onRename: (row: Row, title: string) => Promise<ConversationTaskRowVm>;
  onDelete: (row: Row) => Promise<void>;
};
const pageSize = 30;
const rowKey = (row: ConversationTaskRowVm) => JSON.stringify([row.projectId, row.taskId, row.taskUuid]);

export function ForgeHistoryPage({ workspaces, ...actions }: Actions & { workspaces: ConversationWorkspaceVm[] }) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<GenerationHistoryQuery>({});
  const searchForm = useRef<HTMLFormElement>(null);
  const [cursor, setCursor] = useState<string>();
  const [revision, setRevision] = useState(0);
  function changeFilter(change: GenerationHistoryQuery) {
    setFilter(current => ({ ...current, ...change }));
    setCursor(undefined);
  }
  return <Page flush className="flex h-full min-w-0 flex-col" data-page="forge-history">
    <PageHeader variant="integrated" icon={<History />} title={t('forge.history.title')} />
    <div className="flex shrink-0 flex-wrap items-center gap-2 px-6 pb-3">
      <form ref={searchForm} role="search" className="flex min-w-0 max-w-md flex-1 basis-52 gap-1" onSubmit={event => {
        event.preventDefault();
        changeFilter({ query: String(new FormData(event.currentTarget).get('query') ?? '').trim() || undefined });
      }}>
        <Input name="query" type="search" maxLength={256} aria-label={t('forge.history.search')} placeholder={t('forge.history.search')} className="h-8 min-w-0" />
        <Button type="submit" variant="secondary" size="sm"><Search />{t('conversation.sidebar.search')}</Button>
      </form>
      <Select value={filter.projectId ? `project:${filter.projectId}` : 'all'} onValueChange={value => changeFilter({ projectId: value === 'all' ? undefined : value.slice('project:'.length) })}>
        <SelectTrigger size="sm" aria-label={t('common.workspace')} className="max-w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem value="all">{t('forge.history.allProjects')}</SelectItem>
          {workspaces.map(ws => <SelectItem key={ws.projectId} value={`project:${ws.projectId}`}>{ws.name}</SelectItem>)}
        </SelectContent>
      </Select>
      <Select value={filter.origin ?? 'all'} onValueChange={value => changeFilter({ origin: value === 'forge' || value === 'forge-trial' ? value : undefined })}>
        <SelectTrigger size="sm" aria-label={t('forge.history.kind')}><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{t('forge.history.allKinds')}</SelectItem>
          <SelectItem value="forge">{t('forge.session.badge')}</SelectItem>
          <SelectItem value="forge-trial">{t('forge.session.trialBadge')}</SelectItem>
        </SelectContent>
      </Select>
      {(filter.projectId || filter.origin || filter.query) && <Button type="button" variant="ghost" size="sm" onClick={() => {
        searchForm.current?.reset();
        setFilter({});
        setCursor(undefined);
      }}>{t('forge.history.clearFilters')}</Button>}
    </div>
    <HistoryResults key={JSON.stringify([filter.projectId, filter.origin, filter.query, cursor])}
      filter={filter} cursor={cursor} revision={revision} onPage={setCursor} {...actions}
      onChanged={() => setRevision(value => value + 1)} />
  </Page>;
}

function HistoryResults({ filter, cursor, revision, onPage, onChanged, ...actions }: Actions & {
  filter: GenerationHistoryQuery; cursor?: string; revision: number; onPage: (cursor?: string) => void; onChanged: () => void;
}) {
  const { t } = useTranslation();
  const [page, setPage] = useState<ConversationRecentTaskPageVm>();
  const [request, setRequest] = useState<{ state: 'loading' | 'ready' } | { state: 'error'; error: unknown }>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const flight = useRef<{ key: string; promise: ReturnType<typeof getGenerationHistoryPage> } | null>(null);
  const { projectId, origin, query } = filter;
  useEffect(() => {
    let current = true;
    const key = JSON.stringify([revision, attempt]); // A new filter/cursor mounts a new result window.
    if (flight.current?.key !== key) flight.current = { key, promise: getGenerationHistoryPage({ projectId, origin, query, cursor, limit: pageSize }) };
    setRequest({ state: 'loading' });
    flight.current.promise.then(result => {
      if (current) { setPage(result); setRequest({ state: 'ready' }); }
    }, error => { if (current) setRequest({ state: 'error', error }); });
    return () => { current = false; };
  }, [projectId, origin, query, cursor, revision, attempt]);

  // Apply the acknowledged write to the current window immediately; revalidate membership,
  // ordering and pagination on the server, including when the user switched filters mid-write.
  function updateRow(row: Row, updated?: ConversationTaskRowVm) {
    if (!updated && mounted.current && cursor && page?.tasks.length === 1 && !page.nextCursor) onPage();
    setPage(prev => prev && ({ ...prev, tasks: prev.tasks.flatMap(item => rowKey(item) !== rowKey(row) ? [item] : updated ? [{ ...updated, workspaceName: item.workspaceName }] : []) }));
    onChanged();
  }
  const failed = request.state === 'error';
  const partial = (page?.errors.length ?? 0) > 0;
  return <div className="min-h-0 min-w-0 flex-1 overflow-y-auto px-6 pb-5">
    {request.state === 'loading' && <p role="status" className="flex items-center gap-2 py-3 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />{t('forge.history.loading')}</p>}
    {(failed || partial) && <div role="alert" className="flex flex-wrap items-center gap-2 py-2 text-sm text-destructive">
      <span>{failed ? t('forge.history.failed') : t('forge.history.partial')}</span>
      {failed && <span>{displayAppError(t, request.error)}</span>}
      <Button size="sm" variant="outline" disabled={request.state === 'loading'} onClick={() => setAttempt(n => n + 1)}>{t('common.retry')}</Button>
    </div>}
    {request.state === 'ready' && !partial && !page?.tasks.length && <p className="py-8 text-sm text-muted-foreground">{t('forge.history.empty')}</p>}
    <ul className="divide-y divide-border/60">{page?.tasks.map(row => <HistoryRow key={rowKey(row)} row={row} {...actions} onChanged={updateRow} />)}</ul>
    {(cursor || page?.nextCursor) && <div className="flex flex-wrap gap-2 pt-4">
      {cursor && <Button size="sm" variant="outline" onClick={() => onPage()}>{t('forge.history.newest')}</Button>}
      {page?.nextCursor && <Button size="sm" variant="outline" disabled={request.state === 'loading'} onClick={() => onPage(page.nextCursor!)}>{t('common.nextPage')}</Button>}
    </div>}
  </div>;
}

function HistoryRow({ row, onOpen, onRename, onDelete, onChanged }: Actions & { row: Row; onChanged: (row: Row, updated?: ConversationTaskRowVm) => void }) {
  const { t } = useTranslation();
  const readOnly = useReadOnlyExperience();
  const [action, setAction] = useState<'rename' | 'delete' | null>(null);
  const [draft, setDraft] = useState(row.title);
  const [saving, setSaving] = useState(false);
  const writing = useRef(false);
  const [error, setError] = useState('');
  return <li className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2 py-3" data-project-id={row.projectId} data-task-id={row.taskId}>
    <div className="min-w-0 flex-1 basis-56">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {row.latestRun && row.taskUuid ? <Button variant="link" className="h-auto min-w-0 max-w-full shrink whitespace-normal break-all p-0 text-left text-foreground" onClick={() => onOpen(row)}>{row.title}</Button>
          : <span className="min-w-0 break-all text-sm font-medium">{row.title}</span>}
        <Badge variant={row.origin === 'forge-trial' ? 'outline' : 'secondary'}>{t(row.origin === 'forge-trial' ? 'forge.session.trialBadge' : 'forge.session.badge')}</Badge>
        {!row.latestRun && <span className="text-xs text-muted-foreground">{t('forge.history.noRun')}</span>}
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span className="flex min-w-0 items-center gap-1"><Folder aria-hidden="true" className="size-3.5 shrink-0" /><span className="break-all">{row.workspaceName}</span></span>
        <span>{t('forge.history.lastActivity', { time: formatLocalDateTime(row.lastActivityAt) })}</span>
      </div>
    </div>
    {!readOnly && !action && <div className="flex items-center gap-1">
      <Button size="sm" variant="ghost" disabled={!row.taskUuid} onClick={() => { setDraft(row.title); setError(''); setAction('rename'); }}>{t('conversation.sidebar.rename')}</Button>
      <Button size="sm" variant="ghost" disabled={!row.taskUuid} onClick={() => setAction('delete')}>{t('common.delete')}</Button>
    </div>}
    {action === 'rename' && <form className="flex min-w-0 flex-wrap items-center gap-1" onSubmit={async event => {
      event.preventDefault();
      if (writing.current || !draft.trim() || !row.taskUuid) return;
      writing.current = true; setSaving(true); setError('');
      try { const updated = await onRename(row, draft.trim()); onChanged(row, updated); setAction(null); }
      catch (err) { setError(err instanceof Error ? err.message : displayAppError(t, err)); }
      finally { writing.current = false; setSaving(false); }
    }}>
      <Input autoFocus disabled={saving} aria-label={t('conversation.sidebar.rename')} value={draft} className="h-8 w-40 max-w-full" onChange={event => setDraft(event.target.value)} />
      <Button size="sm" disabled={saving || !draft.trim() || draft.trim() === row.title}>{saving && <Loader2 className="size-3 animate-spin" />}{t('common.save')}</Button>
      <Button size="sm" type="button" variant="ghost" disabled={saving} onClick={() => setAction(null)}>{t('common.cancel')}</Button>
      {error && <span role="alert" className="text-xs text-destructive">{error}</span>}
    </form>}
    {action === 'delete' && <ConversationTaskDeleteDialog title={row.title} workspaceName={row.workspaceName} onClose={() => setAction(null)} onConfirm={async () => { await onDelete(row); onChanged(row); }} />}
  </li>;
}
