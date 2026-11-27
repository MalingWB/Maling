/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '@/App';
import { browserApi } from '@/api/browser';
import { AppProviders } from '@/components/AppProviders';
import { fileContentStore } from '@/components/workspace/files/file-content-store';
import i18n from '@/i18n';
import { forgeHistoryFixture } from './helpers/forge-history-fixture';

// Keep the real App/Shell/WorkspaceShell/sidebar/history page. Only unrelated heavy destinations and notifications are replaced.
vi.mock('@/pages/ConversationHomePage', () => ({ ConversationHomePage: () => null }));
vi.mock('@/pages/ConversationRunPage', () => ({ ConversationRunPage: () => <div data-run-page /> }));
vi.mock('@/components/acp/ACPChatDialog', () => ({ configureAcpResourceCacheSessionCount: vi.fn() }));
vi.mock('@/components/GraphView', () => ({ GraphView: () => null }));
vi.mock('@/lib/use-intervention-notifications', () => ({ useInterventionNotifications: vi.fn() }));
vi.mock('@/lib/use-scheduled-notifications', () => ({ useScheduledNotifications: vi.fn() }));
vi.mock('@/components/scheduled-tasks/useScheduledRuntimeSettings', () => ({ prefetchScheduledRuntimeSettings: vi.fn() }));
vi.mock('@/components/settings/useImSettings', () => ({ prefetchImSettings: vi.fn() }));

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener() {}, removeEventListener() {} })));
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

async function mount(registered: boolean) {
  const fixture = forgeHistoryFixture(registered);
  for (const key of Object.keys(fixture) as (keyof typeof fixture)[]) vi.spyOn(browserApi, key).mockImplementation(fixture[key]);
  window.history.replaceState(null, '', '/chat');
  const host = document.createElement('div'); document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<AppProviders><App /></AppProviders>));
  return async () => { await act(async () => root.unmount()); host.remove(); };
}
function button(key: string, within: ParentNode = document) {
  const button = [...within.querySelectorAll('button')].find(el => el.textContent === i18n.t(key));
  expect(button, key).toBeDefined();
  return button!;
}
const fixedNavigation = () => document.querySelector('[data-conversation-sidebar-region="fixed-navigation"]')!;
const click = (button: HTMLElement) => act(async () => button.click());

it('opens global history from the left even when no workspace exists, without preloading the history', async () => {
  const dispose = await mount(false);
  try {
    expect(browserApi.getGenerationHistoryPage).not.toHaveBeenCalled();
    await click(button('forge.history.title', fixedNavigation()));
    expect(location.pathname).toBe('/chat/forge-history');
    expect(button('forge.history.title', fixedNavigation()).getAttribute('aria-current')).toBe('page');
    expect(document.querySelector('[data-page="forge-history"]')?.textContent).toContain(i18n.t('forge.history.empty'));
    expect(browserApi.getGenerationHistoryPage).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ projectId: undefined }));
  } finally { await dispose(); }
});

it('renames and confirms deletion of a runless owner once, reflects sidebar copies, and opens the surviving same-ID run', async () => {
  const dispose = await mount(true);
  const flush = vi.spyOn(fileContentStore, 'flushAll').mockResolvedValueOnce(false).mockResolvedValue(true);
  try {
    await click(button('forge.history.title', fixedNavigation()));
    const row = (projectId: string) => document.querySelector<HTMLElement>(`[data-page="forge-history"] li[data-project-id="${projectId}"][data-task-id="task-001"]`)!;
    expect(row('atlas') && row('ledger')).toBeTruthy();
    await click(button('conversation.sidebar.rename', row('ledger')));
    const input = row('ledger').querySelector('input')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, '新对账流程');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await click(button('common.save', row('ledger')));
    expect(browserApi.updateTaskMetadata).toHaveBeenCalledExactlyOnceWith('ledger', 'task-001', 'ledger-instance', '新对账流程', undefined);
    const recent = (id: string) => document.querySelector(`[data-conversation-task-row="recent"][data-project-id="${id}"]`);
    expect(recent('ledger')?.textContent).toContain('新对账流程');
    await click(button('common.delete', row('ledger')));
    expect(document.querySelectorAll('[role="alertdialog"]')).toHaveLength(1);
    expect(browserApi.deleteConversationTask).not.toHaveBeenCalled();
    const confirm = () => button('conversation.sidebar.deleteConfirmAction', document.querySelector('[role="alertdialog"]')!);
    await click(confirm());
    expect(document.querySelector('[role="alertdialog"] [role="alert"]')).toBeTruthy();
    expect(browserApi.deleteConversationTask).not.toHaveBeenCalled();
    await act(async () => { const submit = confirm(); submit.click(); submit.click(); });
    expect(flush.mock.calls).toEqual([['ledger'], ['ledger']]);
    expect(browserApi.deleteConversationTask).toHaveBeenCalledExactlyOnceWith('ledger', 'task-001', 'ledger-instance');
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(row('ledger')).toBeNull();
    expect(recent('ledger')).toBeNull();
    expect(recent('atlas')).toBeTruthy();
    expect(document.querySelector('[data-conversation-task-row="pinned"][data-project-id="ledger"]')).toBeNull();
    await click(row('atlas').querySelector('button')!);
    expect(location.pathname).toContain('/chat/projects/atlas/tasks/task-001/runs/run-001');
    expect(browserApi.getConversationRun).toHaveBeenCalledExactlyOnceWith('atlas', 'task-001', 'run-001', null);
  } finally { await dispose(); }
});
