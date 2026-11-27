/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '@/App';
import { browserApi } from '@/api/browser';
import { AppProviders } from '@/components/AppProviders';
import i18n from '@/i18n';
import { directFixture, forgeMaterialFixture, materialLocators } from './helpers/forge-material-fixture';
import { forgeHistoryFixture } from './helpers/forge-history-fixture';

// Real App, two workspaces, real ACP header/message actions and home composer. No native calls or provider.
vi.mock('@/components/GraphView', () => ({ GraphView: () => null }));
vi.mock('@/lib/use-intervention-notifications', () => ({ useInterventionNotifications: vi.fn() }));
vi.mock('@/lib/use-scheduled-notifications', () => ({ useScheduledNotifications: vi.fn() }));
vi.setConfig({ testTimeout: 20_000 }); // The full App and ACP mount can take >5s on a low-memory desktop.
const source = '/chat/projects/ledger/tasks/plain/runs/run-001';
const text = 'Fixture summary';
const action = (key: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${i18n.t(key)}"]`)!;
const quick = () => document.querySelector<HTMLElement>('[data-conversation-composer="quick"]')!;
const input = () => quick().querySelector('textarea')!;
const navigate = (path: string) => act(async () => {
  window.history.pushState({}, '', path);
  window.dispatchEvent(new PopStateEvent('popstate'));
});
const typeDraft = (value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input(), value);
  input().dispatchEvent(new Event('input', { bubbles: true }));
});
let dispose: (() => Promise<void>) | undefined;
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(async () => { await dispose?.(); dispose = undefined; vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function mount() {
  const run = await browserApi.getConversationRun('default', 'mock-task', 'run-052');
  const session = (await browserApi.getAcpSession('default', 'mock-task', 'run-052', 'round-001', 'dev', 'attempt-001'))!;
  const fixture = forgeHistoryFixture();
  for (const key of ['getAppBootstrap', 'getConversationWorkspaces', 'getConversationSidebarBootstrap'] as const) {
    vi.spyOn(browserApi, key).mockImplementation(fixture[key]);
  }
  vi.spyOn(browserApi, 'getConversationRunMode').mockResolvedValue({ mode: 'workflow' });
  vi.spyOn(browserApi, 'getConversationRun').mockResolvedValue({ ...run, projectId: 'ledger', taskId: 'plain', taskUuid: 'ledger-plain-uuid', runId: 'run-001' });
  vi.spyOn(browserApi, 'getAcpSession').mockResolvedValue({ ...session, events: [
    { id: 'reply', seq: 1, timestamp: '2026-11-27T00:00:00Z', kind: 'textDelta', content: 'Fixture reply', raw: {} },
  ], eventPage: { loadedCount: 1, total: 1, oldestSeq: 1, newestSeq: 1, hasNewer: false, hasOlder: false } });
  const create = vi.spyOn(browserApi, 'createConversationRun').mockRejectedValue(new Error('must not send'));
  const digest = vi.spyOn(browserApi, 'forgeSourceDigest');
  window.history.replaceState(null, '', source);
  const host = document.body.appendChild(document.createElement('div'));
  const root = createRoot(host);
  dispose = async () => { await act(async () => root.unmount()); host.remove(); };
  await act(async () => root.render(<AppProviders><App /></AppProviders>));
  expect(action('forge.entry.fromHere')).toBeTruthy();
  return { create, digest };
}

it.each(['draft', 'direct', 'unavailable'] as const)('hands off mixed Role/Skill/Plugin through real Context Management (%s)', async (branch) => {
  const fixture = forgeMaterialFixture(branch === 'direct');
  vi.spyOn(browserApi, 'getProfiles').mockImplementation(fixture.getProfiles);
  vi.spyOn(browserApi, 'listSkills').mockImplementation(fixture.listSkills);
  vi.spyOn(browserApi, 'listInstalledPlugins').mockImplementation(fixture.listInstalledPlugins);
  const mode = vi.spyOn(browserApi, 'getConversationRunMode').mockImplementation(fixture.getConversationRunMode);
  if (branch === 'unavailable') mode.mockRejectedValue({ code: 'app.unexpected', params: {} });
  const digest = vi.spyOn(browserApi, 'forgeItemsDigest').mockImplementation(fixture.forgeItemsDigest);
  const create = vi.spyOn(browserApi, 'createConversationRun'); // browser-only synthetic run, no Agent process
  const saveMode = vi.spyOn(browserApi, 'saveConversationRunMode');
  window.history.replaceState(null, '', '/chat/contexts');
  const host = document.body.appendChild(document.createElement('div'));
  const root = createRoot(host);
  dispose = async () => { await act(async () => root.unmount()); host.remove(); };
  await act(async () => root.render(<AppProviders><App /></AppProviders>));
  const select = (name: string) => act(async () => host.querySelector<HTMLButtonElement>(`[role="checkbox"][aria-label="${i18n.t('contextManagement.skills.forgeSelect',{name})}"]`)!.click());
  const tab = (label: string) => act(async () => { [...host.querySelectorAll('[role="tab"]')].find(el => el.textContent === label)!.dispatchEvent(new MouseEvent('mousedown', { bubbles:true, button:0 })); });
  await select('Reviewer');
  await tab(i18n.t('contextManagement.tabs.skills'));
  await select('Verify');
  await tab('Plugin');
  await select('Report');
  await act(async () => [...host.querySelectorAll('button')].find(b => b.textContent === i18n.t('contextManagement.skills.forgeAction',{ count:3 }))!.click());
  expect(digest).toHaveBeenCalledExactlyOnceWith('default', materialLocators);
  if (branch === 'unavailable') {
    expect(location.pathname).toBe('/chat/contexts');
    expect(document.body.textContent).toContain(i18n.t('contextManagement.skills.forgeModeUnavailable'));
    expect(host.querySelector('[role="checkbox"][aria-checked="true"]')).not.toBeNull();
    expect(create).not.toHaveBeenCalled();
  } else if (branch === 'direct') {
    expect(create).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ projectId: 'default', runMode: 'direct', directConfig: directFixture, forgeSourceItems: materialLocators }));
    expect(location.pathname).toMatch(/^\/chat\/projects\/default\/tasks\//);
    expect(saveMode).not.toHaveBeenCalled();
  } else {
    expect(location.pathname).toBe('/chat');
    expect(input().value).toContain('Reviewer');
    expect(input().value).toContain(i18n.t('forge.itemsPrefill.kind.role'));
    expect(input().value).not.toContain('private');
    expect(quick().querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe(i18n.t('conversation.home.direct'));
    expect(create).not.toHaveBeenCalled();
  }
});

it('prefills from the selected branch in its own workspace and Direct mode, without choosing an agent or sending', async () => {
  const { digest, create } = await mount();
  digest.mockResolvedValue({ title: 'Plain', markdown: text });
  await act(async () => action('forge.entry.fromHere').click());
  expect(digest).toHaveBeenCalledExactlyOnceWith('ledger', 'plain', expect.objectContaining({
    taskUuid: 'ledger-plain-uuid', runId: 'run-001', roundId: 'round-001', nodeId: 'dev', attemptId: 'attempt-001', branchId: 'root', after: { eventId: 'reply', eventSeq: 1 },
  }));
  expect(location.pathname).toBe('/chat');
  expect(input().value).toContain(text);
  expect.soft(quick().querySelector('[data-conversation-workspace-value]')?.textContent).toBe('账务中心');
  const selected = Array.from(quick().querySelectorAll('[role="tab"][aria-selected="true"]'), tab => tab.textContent);
  expect(selected).toEqual([i18n.t('conversation.home.direct')]);
  await act(async () => action('conversation.composer.removeForgeBinding').click());
  expect(quick().querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe(i18n.t('conversation.home.workflow'));
  expect(input().value).toContain(text);
  expect(create).not.toHaveBeenCalled();
});

it('does not overwrite an existing draft or carry its contents into a different workspace', async () => {
  const { digest, create } = await mount();
  await navigate('/chat');
  await typeDraft('Existing Atlas draft');
  await navigate(source);
  await act(async () => action('forge.entry.action').click());
  expect(document.body.textContent).toContain(i18n.t('acp.promptQueue.editDraftOccupied'));
  await navigate('/chat');
  expect(input().value).toBe('Existing Atlas draft');
  expect(digest).not.toHaveBeenCalled();
  expect(create).not.toHaveBeenCalled();
});

it('does not let a delayed digest hijack the draft edited after leaving the source run', async () => {
  const { digest, create } = await mount();
  let finish!: (value: { title: string; markdown: string }) => void;
  digest.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => { const button = action('forge.entry.action'); button.click(); button.click(); });
  await navigate('/chat');
  await typeDraft('Keep my current draft');
  await act(async () => finish({ title: 'Old source', markdown: text }));
  expect(input().value).toBe('Keep my current draft');
  expect(digest).toHaveBeenCalledTimes(1);
  expect(create).not.toHaveBeenCalled();
});
