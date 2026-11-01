import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';

/**
 * Fullscreen overlay for workflow graph inspection. The embedded graph keeps
 * its own zoom/direction controls; node interactions close the overlay before
 * navigating so the target view is not hidden behind the dialog.
 */
export function GraphFullscreenDialog({
  open,
  onOpenChange,
  title,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex h-screen w-screen max-w-none flex-col gap-0 overflow-hidden rounded-none border-0 p-0 sm:max-w-none [&>button]:z-10"
        aria-describedby={undefined}
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{title}</DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1" data-workflow-graph-fullscreen="true">
          {open ? children : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
