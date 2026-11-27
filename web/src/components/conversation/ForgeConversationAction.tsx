import { Eye, Hammer } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';

export function ForgeConversationAction({ preview = false, disabled, onClick }: {
  preview?: boolean; disabled?: boolean; onClick: () => void;
}) {
  const { t } = useTranslation();
  const label = t(preview ? 'forge.preview.open' : 'forge.entry.action');
  const Icon = preview ? Eye : Hammer;
  return <Tooltip>
    <TooltipTrigger asChild>
      <Button type="button" variant="ghost" size="sm" className="h-6 min-w-0 gap-1 px-1.5 text-xs"
        aria-label={label} disabled={disabled} onClick={onClick}>
        <Icon className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="max-w-24 truncate">{t(preview ? 'forge.preview.title' : 'forge.entry.action')}</span>
      </Button>
    </TooltipTrigger>
    <TooltipContent>{label}</TooltipContent>
  </Tooltip>;
}
