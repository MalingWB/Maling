import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown, ChevronUp, FolderOpen, Loader2, Package } from 'lucide-react';
import { forgetWorkBuddySource, importPluginAsset, listInstalledPlugins, selectWorkBuddySource } from '../api';
import { displayAppError } from '../i18n';
import { AgentIdentityLabel } from '@/components/AgentIdentityLabel';
import { AssetCardGrid } from '@/components/EntitySection';
import { EmptyState } from '@/components/PageScaffold';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from '@/components/ui/alert-dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import { forgeItemKey, pluginForgeItems } from '@/lib/forge-items';
import { RefreshSingleFlight } from '@/lib/refresh-single-flight';
import type { AgentCatalogEntryVm, InstalledPluginVm, ForgeSourceItemVm } from '../types';

type ImportSelection = { projectId: string | null; assetId: string; name: string; kind: 'skill' | 'agent' };
interface PluginForgeAssetsProps {
  projectId: string | null;
  projectName: string | null;
  catalog?: AgentCatalogEntryVm[];
  refreshToken: number;
  isItemSelected: (item: ForgeSourceItemVm) => boolean;
  onToggleItem: (item: ForgeSourceItemVm) => void;
  onItemsListed: (items: ForgeSourceItemVm[]) => void;
  onLoadingChange: (loading: boolean) => void;
  onImported: (result: ImportSelection) => void;
  canGenerate: boolean;
  readOnly: boolean;
}

export function PluginForgeAssets(props: PluginForgeAssetsProps) {
  // Empty/ready/error, disclosure, picker and confirmation all belong to this exact scope.
  return <ScopedInventory key={JSON.stringify([props.projectId, props.readOnly])} {...props} />;
}

function ScopedInventory(props: PluginForgeAssetsProps) {
  const { projectId, projectName, refreshToken, readOnly, onImported } = props;
  const { t } = useTranslation();
  const [plugins, setPlugins] = useState<InstalledPluginVm[] | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [selection, setSelection] = useState<ImportSelection | { kind: 'forget'; projectId: string | null; pluginId: string; name: string } | null>(null);
  const [operation, setOperation] = useState<'pick' | 'confirm' | null>(null);
  const [retry, setRetry] = useState(0);
  const [loading, setLoading] = useState(true);
  const flight = useRef(new RefreshSingleFlight());
  const mounted = useRef(false);
  const busy = useRef(false);
  const callbacks = useRef(props);
  callbacks.current = props;
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; callbacks.current.onLoadingChange(false); };
  }, []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    callbacks.current.onLoadingChange(true);
    void flight.current.run('inventory', () => listInstalledPlugins(projectId), refreshToken > 0 || retry > 0)
      .then((next) => {
        if (!active) return;
        setPlugins(next); setError('');
        const items = next.flatMap(pluginForgeItems);
        callbacks.current.onItemsListed(items);
        setSelection(old => {
          if (!old) return null;
          if (old.kind === 'forget') return next.some(p => p.pluginId === old.pluginId) ? old : null;
          return next.some(p => p.assets.some(a => a.assetId === old.assetId && a.importable)) ? old : null;
        });
      }).catch((err) => { if (active) setError(displayAppError(t, err)); })
      .finally(() => { if (active) { setLoading(false); callbacks.current.onLoadingChange(false); } });
    return () => { active = false; };
  }, [projectId, refreshToken, retry]);

  async function pickDirectory() {
    if (readOnly || busy.current || loading) return;
    busy.current = true; callbacks.current.onLoadingChange(true); setOperation('pick'); setNotice('');
    try {
      const picked = await selectWorkBuddySource(projectId);
      if (!picked || !mounted.current) return;
      // A pinned installation may resolve to multiple bundles/registry entries, not just the returned row.
      setRetry(n => n + 1);
    } catch (err) { if (mounted.current) setNotice(displayAppError(t, err)); }
    finally { busy.current = false; if (mounted.current) { setOperation(null); callbacks.current.onLoadingChange(false); } }
  }
  async function confirmSelection() {
    if (readOnly || !selection || busy.current) return;
    busy.current = true; callbacks.current.onLoadingChange(true); setOperation('confirm');
    try {
      if (selection.kind === 'forget') {
        const removed = await forgetWorkBuddySource(selection.projectId, selection.pluginId);
        if (!mounted.current) return;
        if (!removed) { setNotice(t('errors.plugin.asset-unavailable')); setRetry(n => n + 1); return; }
        setRetry(n => n + 1);
        return;
      }
      await importPluginAsset({ projectId: selection.projectId, assetId: selection.assetId });
      onImported(selection);
      if (mounted.current) setNotice(t(selection.kind === 'skill' ? 'plugin.skillImported' : 'plugin.roleImported'));
    } catch (err) { if (mounted.current) setNotice(displayAppError(t, err)); }
    finally { busy.current = false; if (mounted.current) { setSelection(null); setOperation(null); callbacks.current.onLoadingChange(false); } }
  }
  const hasManual = plugins?.some(p => (p.canForget || p.provenance === 'manual-directory') && p.source === (projectId ? 'project' : 'global'));
  const actionLabel = selection?.kind === 'forget' ? 'plugin.forget' : selection?.kind === 'skill' ? 'plugin.importSkill' : 'plugin.importRole';
  const directoryHintId = useId();
  return <div className="min-w-0" data-plugin-forge-assets>
    {!readOnly && <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 pt-3"><Button size="sm" variant="outline" className="max-w-full" aria-describedby={directoryHintId} disabled={!!operation || loading} onClick={() => void pickDirectory()}>
      {operation === 'pick' ? <Loader2 className="animate-spin" /> : <FolderOpen />}<span className="truncate">{t(hasManual ? 'plugin.replaceDirectory' : 'plugin.selectDirectory')}</span>
    </Button><p id={directoryHintId} className="text-xs text-muted-foreground">{t('plugin.directoryHint')}</p></div>}
    {notice && <p role="status" className="px-4 pt-3 text-xs text-muted-foreground">{notice}</p>}
    {error && <div role="alert" className="flex flex-wrap items-center gap-2 px-4 pt-3 text-sm text-destructive">{error}<Button variant="ghost" size="sm" disabled={loading} onClick={() => setRetry(n => n + 1)}>{t('common.retry')}</Button></div>}
    {!plugins && loading ? <div role="status" className="flex items-center justify-center gap-2 p-5 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />{t('contextManagement.skills.plugins.loading')}</div> : null}
    {plugins?.length === 0 && !error && <div className="p-5"><EmptyState>{t('contextManagement.skills.plugins.empty')}</EmptyState></div>}
    {plugins?.length ? <AssetCardGrid>{plugins.map(plugin => <PluginCard key={plugin.pluginId} plugin={plugin} projectName={projectName}
      host={props.catalog?.find(entry => entry.agentType === plugin.providerId)}
      canGenerate={!readOnly && props.canGenerate} canImport={!readOnly} disabled={loading || !!operation || (plugin.source === 'project' && !projectId)}
      onForget={() => { setNotice(''); setSelection({kind: 'forget', projectId: plugin.source === 'project' ? projectId : null, pluginId: plugin.pluginId, name: plugin.displayName || plugin.name}); }}
      isSelected={props.isItemSelected} toggle={props.onToggleItem} onImport={(asset) => {
        setNotice(''); setSelection({ ...asset, projectId: plugin.source === 'project' ? projectId : null });
      }} />)}</AssetCardGrid> : null}
    <AlertDialog open={!!selection} onOpenChange={open => { if (!open && !busy.current) setSelection(null); }}>
      <AlertDialogContent><AlertDialogHeader>
        <AlertDialogTitle className="break-words">{t(actionLabel)} · {selection?.name}</AlertDialogTitle>
        <AlertDialogDescription>{t(selection?.kind === 'forget' ? 'plugin.forgetConfirm' : selection?.kind === 'skill' ? 'plugin.skillConfirm' : 'plugin.roleConfirm')}</AlertDialogDescription>
      </AlertDialogHeader><AlertDialogFooter><AlertDialogCancel disabled={!!operation}>{t('common.cancel')}</AlertDialogCancel>
        <AlertDialogAction disabled={!!operation} onClick={event => { event.preventDefault(); void confirmSelection(); }}>
          {operation === 'confirm' && <Loader2 className="size-3 animate-spin" />}{t('common.confirm')}
        </AlertDialogAction></AlertDialogFooter></AlertDialogContent>
    </AlertDialog>
  </div>;
}

const PREVIEW_ROWS = 3;
function PluginCard({ plugin, projectName, host, canGenerate, canImport, disabled, isSelected, toggle, onImport, onForget }: {
  plugin: InstalledPluginVm; projectName: string | null; host?: AgentCatalogEntryVm;
  canGenerate: boolean; canImport: boolean; disabled: boolean;
  isSelected: PluginForgeAssetsProps['isItemSelected']; toggle: PluginForgeAssetsProps['onToggleItem'];
  onForget: () => void;
  onImport: (asset: Omit<ImportSelection, 'projectId'>) => void;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const manual = plugin.provenance === 'manual-directory';
  const sourceName = manual ? t('plugin.manualSource')
    : plugin.provenance === 'desktop-bundled' ? t('plugin.desktopBundled')
    : plugin.provenance === 'desktop-registry' ? t('plugin.desktopRegistry')
    : plugin.providerId === 'codebuddy-code' ? t('plugin.codeBuddyCli') : host?.label ?? t('plugin.hostSource');
  const iconKey = !manual && host?.iconKey;
  return <Card data-plugin-name={plugin.name} className="min-w-0 gap-0 overflow-hidden border-border/50 py-0">
    <CardHeader className="min-w-0 gap-1.5 px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1"><CardTitle className="break-all text-sm leading-snug">{plugin.displayName || plugin.name}</CardTitle>
        {plugin.version && <span className="break-all text-xs text-muted-foreground">{plugin.version}</span>}</div>
      <div className="flex min-w-0 flex-wrap items-center gap-2 text-xs" data-plugin-provider={plugin.providerId}>
        {iconKey ? <AgentIdentityLabel name={sourceName} iconKey={iconKey} /> : <span className="inline-flex min-w-0 items-center gap-2">{manual ? <FolderOpen className="size-4 shrink-0" /> : <Package className="size-4 shrink-0" />}{sourceName}</span>}
        <Badge variant="secondary" className="max-w-full whitespace-normal break-all text-ui-micro">{plugin.source === 'global' ? t('forge.itemsPrefill.scope.global') : [t('forge.itemsPrefill.scope.project'), projectName].filter(Boolean).join(' · ')}</Badge>
      </div>
      {manual && <p className="text-xs text-muted-foreground">{t('plugin.manualEngine')}</p>}
      {plugin.description && <CardDescription className="line-clamp-2 break-words text-xs">{plugin.description}</CardDescription>}
    </CardHeader>
    <CardContent className="border-t border-border/30 px-4 py-3">
      <p className="text-xs text-muted-foreground">{plugin.unavailable ? t('plugin.unavailable') : plugin.assets.length ? t('plugin.assetCount', { count: plugin.assets.length }) : t('contextManagement.skills.plugins.noAssets')}</p>
      <ul id={listId} className="divide-y divide-border/30">{(expanded ? plugin.assets : plugin.assets.slice(0, PREVIEW_ROWS)).map(({assetId, name, kind, description, importable}) => {
        const item: ForgeSourceItemVm = { catalog: 'plugin', source: plugin.source, assetId };
        return <li key={forgeItemKey(item)} className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 py-2">
          {canGenerate && <Checkbox disabled={disabled} checked={isSelected(item)} onCheckedChange={() => toggle(item)} aria-label={t('contextManagement.skills.forgeSelect', { name })} />}
          <div className="min-w-[min(100%,6rem)] flex-1"><span className="break-all text-xs font-medium">{name}</span>
            <span className="ml-1.5 text-ui-micro text-muted-foreground">{t(`forge.itemsPrefill.kind.${kind}`)}</span>
            {description && <p className="line-clamp-2 break-words text-xs text-muted-foreground">{description}</p>}
            {kind === 'skill' && !importable && <p className="text-xs text-muted-foreground">{t('plugin.unsupportedBundle')}</p>}</div>
          {canImport && importable && (kind === 'skill' || kind === 'agent') && <Button size="sm" variant="outline" className="ml-auto h-7 px-2 text-xs" disabled={disabled} onClick={() => onImport({ assetId, name, kind })}>{t(kind === 'skill' ? 'plugin.importSkill' : 'plugin.importRole')}</Button>}
        </li>;
      })}</ul>
      {plugin.assets.length > PREVIEW_ROWS && <Button size="sm" variant="ghost" className="mt-1 h-7 px-2 text-xs" aria-expanded={expanded} aria-controls={listId} onClick={() => setExpanded(!expanded)}>
        {expanded ? <ChevronUp /> : <ChevronDown />}{t(expanded ? 'plugin.collapse' : 'plugin.showRemaining', { count: plugin.assets.length - PREVIEW_ROWS })}</Button>}
      {canImport && (manual || plugin.canForget) && <Button size="sm" variant="ghost" className="mt-1 h-7 px-2 text-xs text-muted-foreground" disabled={disabled} onClick={onForget}>{t('plugin.forget')}</Button>}
    </CardContent>
  </Card>;
}
