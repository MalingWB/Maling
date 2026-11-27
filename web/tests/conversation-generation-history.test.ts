import { expect, it } from 'vitest';
import { applyConversationTaskTitle, applyConversationSidebarBootstrap, createEmptyConversationSidebar } from '@/lib/conversation-sidebar-loading';
import { forgeHistoryFixture } from './helpers/forge-history-fixture';

it('a historical rename updates only materialized copies of that owner; it cannot make an older task recent', async () => {
  const fixture = forgeHistoryFixture();
  const base = applyConversationSidebarBootstrap(createEmptyConversationSidebar(), await fixture.getConversationSidebarBootstrap());
  const [a, b] = (await fixture.getGenerationHistoryPage({})).tasks;
  const sidebar = { ...base, recentTasks: [a], pinnedTasks: [b], tasksByWorkspace: { atlas: [a], ledger: [] } };
  const next = applyConversationTaskTitle(sidebar, { ...b, title: 'Revised' });
  expect(next.pinnedTasks.map(t => t.title)).toEqual(['Revised']);
  expect(next.recentTasks).toEqual([a]);
  expect(next.tasksByWorkspace).toEqual({ atlas: [a], ledger: [] });
});
