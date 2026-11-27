import { useTranslation } from 'react-i18next';
import type { ConversationTaskRowVm } from '@/types';
import { Badge } from '@/components/ui/badge';

export function ConversationOriginBadge({ origin }: Pick<ConversationTaskRowVm, 'origin'>) {
  const { t } = useTranslation();
  if (origin !== 'forge' && origin !== 'forge-trial') return null;
  return <Badge variant="outline" className="rounded px-1 py-0 text-[0.65rem] leading-4 text-muted-foreground">
    {t(origin === 'forge-trial' ? 'forge.session.trialBadge' : 'forge.session.badge')}
  </Badge>;
}
