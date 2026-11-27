import { forwardRef, useImperativeHandle, type ReactNode } from 'react';
import {
  ConversationComposerDraftProvider,
  createConversationComposerDraftBoundaryHandle,
  conversationComposerDraftOccupied,
  useConversationComposerDraftOwner,
  type ConversationComposerDraftBoundaryHandle,
} from '@/lib/conversation-composer-draft';

interface ConversationComposerDraftBoundaryProps {
  children: ReactNode;
}

export const ConversationComposerDraftBoundary = forwardRef<
  ConversationComposerDraftBoundaryHandle,
  ConversationComposerDraftBoundaryProps
>(function ConversationComposerDraftBoundary({ children }, ref) {
  const owner = useConversationComposerDraftOwner();
  const occupied = conversationComposerDraftOccupied(owner.draft);

  useImperativeHandle(
    ref,
    () => createConversationComposerDraftBoundaryHandle(owner),
    [occupied, owner.changeWorkspace, owner.prefillForge, owner.reset],
  );

  return (
    <ConversationComposerDraftProvider value={owner}>
      {children}
    </ConversationComposerDraftProvider>
  );
});

export type { ConversationComposerDraftBoundaryHandle };
