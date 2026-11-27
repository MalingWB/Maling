import { useRef, useState } from 'react';
import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog';

export function ConversationTaskDeleteDialog({ title, workspaceName, onConfirm, onClose }: {
  title: string; workspaceName?: string; onConfirm: () => void | Promise<void>; onClose: () => void;
}) {
  const { t } = useTranslation();
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  return <AlertDialog open onOpenChange={open => { if (!open && !inFlight.current) onClose(); }}>
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{t('conversation.sidebar.deleteConfirmTitle')}</AlertDialogTitle>
        <AlertDialogDescription className="break-words">
          {workspaceName && <span className="mb-1 block">{t('common.workspace')}: {workspaceName}</span>}
          {t('conversation.sidebar.deleteConfirmDescription', { title })}
        </AlertDialogDescription>
      </AlertDialogHeader>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={pending}>{t('common.close')}</AlertDialogCancel>
        <AlertDialogAction disabled={pending} className="bg-destructive text-destructive-foreground hover:bg-destructive/90" onClick={async event => {
          event.preventDefault();
          if (inFlight.current) return;
          inFlight.current = true;
          setPending(true);
          setError('');
          try { await onConfirm(); onClose(); }
          catch (err) { setError(err instanceof Error ? err.message : (await import('@/i18n')).displayAppError(t, err)); }
          finally { inFlight.current = false; setPending(false); }
        }}>
          {pending && <Loader2 className="size-4 animate-spin" />}{t('conversation.sidebar.deleteConfirmAction')}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
}
