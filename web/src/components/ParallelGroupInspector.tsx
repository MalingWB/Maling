import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowUp, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { MAX_GROUP_STAGES, MIN_BRANCHES, PARALLEL_CAPS, type GroupEdit } from '@/lib/workflow-parallel';
import type { WorkflowEdgeDsl, WorkflowParallelGroupNodeDsl } from '@/types';

export function ParallelGroupInspector({ group, edges, onEdit, onSelect }: { group: WorkflowParallelGroupNodeDsl; edges: WorkflowEdgeDsl[]; onEdit: (action: GroupEdit) => void; onSelect: (id: string) => void }) {
  const { t } = useTranslation();
  const room = group.branches.reduce((n, b) => n + b.stages.length, 0) < MAX_GROUP_STAGES;
  const controls = (id: string, index: number, count: number, move: (delta: -1 | 1) => void, remove: () => void, canRemove: boolean, blockUp = false, blockDown = false) => (
    <div className="flex shrink-0 items-center gap-0.5">
      {([{ key: 'up', Icon: ArrowUp, disabled: index === 0 || blockUp, click: () => move(-1) },
        { key: 'down', Icon: ArrowDown, disabled: index === count - 1 || blockDown, click: () => move(1) },
        { key: 'remove', Icon: Trash2, disabled: !canRemove, click: remove }]).map(({key, Icon, disabled, click}) => {
        const label = t(`workflowEditor.parallel.${key}`, { id });
        return <Tooltip key={key}><TooltipTrigger asChild><Button type="button" variant="ghost" size="icon-sm" aria-label={label} disabled={disabled} onClick={click}><Icon className="size-3.5" /></Button></TooltipTrigger><TooltipContent>{label}</TooltipContent></Tooltip>;
      })}
    </div>
  );
  return <section className="min-w-0 space-y-3" aria-label={t('workflowEditor.parallel.fork')}>
    <h4 className="break-all text-sm font-semibold">{t('workflowEditor.parallel.fork')} · {group.id}</h4>
    <label className="grid gap-1 text-xs">{t('workflowEditor.parallel.cap')}
      <Select value={String(group.maxParallel)} onValueChange={v => onEdit({kind:'cap', value:Number(v)})}>
        <SelectTrigger aria-label={t('workflowEditor.parallel.cap')}><SelectValue /></SelectTrigger>
        <SelectContent>{PARALLEL_CAPS.map(n => <SelectItem key={n} value={String(n)}>{n}</SelectItem>)}</SelectContent>
      </Select>
    </label>
    {group.branches.map((branch, i) => <section key={branch.id} aria-label={branch.id} className="min-w-0 space-y-1 border-t pt-2">
      <div className="flex flex-wrap items-center justify-between gap-1"><h5 className="break-all text-xs font-semibold">{branch.id}</h5>
        {controls(branch.id, i, group.branches.length, delta => onEdit({kind:'move', branch:branch.id, delta}), () => onEdit({kind:'remove-branch', branch:branch.id}), group.branches.length > MIN_BRANCHES)}
      </div>
      {branch.stages.map((step, j) => <div key={step} className="flex min-w-0 flex-wrap items-center justify-between gap-1 pl-2">
        <Button type="button" variant="link" size="sm" className="h-auto min-w-0 max-w-full justify-start whitespace-normal break-all px-0 text-left" onClick={() => onSelect(step)}>{step}</Button>
        {controls(step, j, branch.stages.length, delta => onEdit({kind:'move', branch:branch.id, step, delta}), () => onEdit({kind:'remove-step', branch:branch.id, step}), branch.stages.length > 1,
          edges.some(e => e.from === step && e.to === branch.stages[j-1]), edges.some(e => e.from === branch.stages[j+1] && e.to === step))}
      </div>)}
      <Button type="button" variant="ghost" size="sm" disabled={!room} onClick={() => onEdit({kind:'add-step', branch:branch.id})}>{t('workflowEditor.parallel.addStep')}</Button>
    </section>)}
    <Button type="button" variant="outline" size="sm" disabled={!room} onClick={() => onEdit({kind:'add-branch'})}>{t('workflowEditor.parallel.addBranch')}</Button>
  </section>;
}
