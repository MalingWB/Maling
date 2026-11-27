/** @vitest-environment jsdom */
import React, { act, StrictMode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ForgeHistoryPage } from '@/pages/ForgeHistoryPage';
import { getGenerationHistoryPage } from '@/api';
import { loadI18nLanguage } from '@/i18n';
import type { ConversationRecentTaskRowVm, ConversationRecentTaskPageVm } from '@/types';

vi.mock('@/api', () => ({ getGenerationHistoryPage: vi.fn() }));
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const workspaces = ['Atlas', 'Birch'].map(name => ({ name, projectId: name.toLowerCase(), workspacePath: `/${name}` }));
const atlas: ConversationRecentTaskRowVm = { projectId: 'atlas', taskId: 'task-1', taskUuid: 'a-uuid', workspaceName: 'Atlas', title: 'Release draft', origin: 'forge', runMode: 'direct', autoTitle: false, lastActivityAt: '2026-03-01T12:00:00Z', runs: [], runHistoryStatus: 'not-loaded', pinned: false };
const birch: ConversationRecentTaskRowVm = { ...atlas, projectId: 'birch', taskUuid: 'b-uuid', title: 'Release trial', workspaceName: 'Birch', origin: 'forge-trial', latestRun: { runId: 'r-1', status: 'completed', startedAt: '', updatedAt: '', resumable: false } };
const page = (tasks: ConversationRecentTaskRowVm[], nextCursor: string | null = null): ConversationRecentTaskPageVm => ({ tasks, nextCursor, errors: [] });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const read = vi.mocked(getGenerationHistoryPage);
let root: Root;
let host: HTMLDivElement;
const onOpen = vi.fn();
const onRename = vi.fn();
const onDelete = vi.fn();
const mount = () => act(async () => root.render(<StrictMode><ForgeHistoryPage workspaces={workspaces} onOpen={onOpen} onRename={onRename} onDelete={onDelete} /></StrictMode>));
function button(name: string, within: ParentNode = document) {
  const found = [...within.querySelectorAll('button')].find(el => el.textContent === name);
  if (!found) throw Error(`Missing button ${name}`);
  return found;
}
const click = (element: HTMLElement) => act(async () => element.click());
function setInput(input: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}
async function choose(label: string, value: string) {
  await act(async () => host.querySelector(`[role="combobox"][aria-label="${label}"]`)!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(el => el.textContent === value)!;
  await click(option);
}
beforeAll(async () => { await loadI18nLanguage('en'); HTMLElement.prototype.scrollIntoView ??= () => {}; });
beforeEach(() => { vi.resetAllMocks(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

describe('generation history management', () => {
  it('isolates equal task IDs, keeps runless drafts manageable, and confirms deletion once', async () => {
    read.mockResolvedValue(page([atlas, birch]));
    await mount();
    expect(read).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledWith({ projectId: undefined, origin: undefined, query: undefined, cursor: undefined, limit: 30 });
    const a = host.querySelector('li[data-project-id="atlas"]')!;
    const b = host.querySelector('li[data-project-id="birch"]')!;
    expect(a.textContent).toContain('Not run yet');
    expect(a.textContent).toContain('Generation');
    expect(a.textContent).toContain('Atlas');
    expect(a.textContent).toContain('Last activity:');
    expect(b.textContent).toContain('Trial');
    expect([...a.querySelectorAll('button')].map(el => el.textContent)).toEqual(['Rename', 'Delete']);
    await click(button(birch.title, b));
    expect(onOpen).toHaveBeenCalledWith(birch);
    await click(button('Rename', a));
    await act(async () => setInput(a.querySelector('input')!, 'Revised'));
    const updated = { ...atlas, title: 'Revised (saved)' };
    onRename.mockResolvedValue(updated); read.mockResolvedValue(page([updated, birch]));
    await click(button('Save', a));
    expect(onRename).toHaveBeenCalledWith(atlas, 'Revised');
    expect(a.textContent).toContain(updated.title);
    await click(button('Delete', a));
    expect(document.querySelectorAll('[role="alertdialog"]')).toHaveLength(1);
    expect(onDelete).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alertdialog"]')!.textContent).toContain(updated.title);
    expect(document.querySelector('[role="alertdialog"]')!.textContent).toContain(atlas.workspaceName);
    const pending = deferred<void>();
    onDelete.mockReturnValue(pending.promise);
    read.mockResolvedValue(page([birch]));
    const confirm = document.querySelector<HTMLButtonElement>('[role="alertdialog"] button:last-of-type')!;
    await act(async () => { confirm.click(); confirm.click(); });
    expect(onDelete).toHaveBeenCalledTimes(1); expect(onDelete).toHaveBeenCalledWith(updated);
    await act(async () => pending.resolve());
    expect(host.querySelector('li[data-project-id="atlas"]')).toBeNull();
    expect(host.querySelector('li[data-project-id="birch"]')).toBe(b);
  });

  it('does not open or mutate a row without a verified UUID and returns from a deleted final page', async () => {
    const unknown = { ...birch, taskUuid: null };
    read.mockResolvedValueOnce(page([unknown], 'older')).mockResolvedValueOnce(page([atlas])).mockResolvedValue(page([unknown]));
    await mount();
    const row = host.querySelector('li')!;
    expect(row.textContent).toContain(unknown.title);
    expect([...row.querySelectorAll('button')].every(button => button.disabled)).toBe(true);
    await click(button('Next'));
    await click(button('Delete'));
    const dialog = document.querySelector('[role="alertdialog"]')!;
    await click(dialog.querySelector<HTMLButtonElement>('button:last-of-type')!);
    expect(read.mock.lastCall![0].cursor).toBeUndefined();
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(host.querySelector('li')!.textContent).toContain(unknown.title);
    expect(onDelete).toHaveBeenCalledWith(atlas);
  });

  it('drops obsolete pages on filter changes, including late successes and failures', async () => {
    const old = deferred<ConversationRecentTaskPageVm>();
    const middle = deferred<ConversationRecentTaskPageVm>();
    read.mockReturnValueOnce(old.promise).mockReturnValueOnce(middle.promise).mockResolvedValue(page([birch]));
    await mount();
    expect(host.querySelector('[role="status"]')?.textContent).toContain('Loading');
    expect(host.textContent).not.toContain('No generations');
    await choose('Workspace', 'Birch');
    await choose('Record type', 'Trial');
    await act(async () => setInput(host.querySelector('input[name="query"]')!, ' release '));
    await click(button('Search'));
    expect(read).toHaveBeenLastCalledWith({ projectId: 'birch', origin: 'forge-trial', query: 'release', cursor: undefined, limit: 30 });
    await act(async () => { old.resolve(page([atlas])); middle.reject(Error('stale read')); });
    expect(host.querySelectorAll('li')).toHaveLength(1);
    expect(host.textContent).toContain(birch.title);
    expect(host.textContent).not.toContain(atlas.title);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it('clears an empty search and shows all workspaces again without typed text left behind', async () => {
    read.mockResolvedValue(page([]));
    await mount();
    await choose('Workspace', 'Atlas');
    const input = host.querySelector<HTMLInputElement>('input[name="query"]')!;
    await act(async () => setInput(input, 'missing'));
    await click(button('Search'));
    read.mockResolvedValue(page([atlas, birch]));
    await click(button('Clear filters'));
    expect(input.value).toBe('');
    expect(read.mock.lastCall![0]).toMatchObject({ projectId: undefined, origin: undefined, query: undefined, cursor: undefined });
    expect(host.querySelectorAll('li[data-project-id]')).toHaveLength(2);
  });

  it('continues past an empty damaged page and retries an opaque cursor locally', async () => {
    read.mockResolvedValueOnce({ tasks: [], errors: [{ code: 'task.not-found', params: {} }], nextCursor: 'opaque' })
      .mockRejectedValueOnce(Error('unavailable')).mockResolvedValueOnce(page([atlas]));
    await mount();
    expect(host.querySelector('[role="alert"]')!.textContent).toContain('Some records');
    await click(button('Next'));
    expect(read.mock.lastCall![0].cursor).toBe('opaque');
    expect(host.querySelector('[role="alert"]')!.textContent).toContain('Could not load');
    await click(button('Retry'));
    expect(read.mock.lastCall![0].cursor).toBe('opaque');
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.textContent).toContain(atlas.title);
    read.mockResolvedValue(page([]));
    await click(button('Back to latest'));
    expect(read.mock.lastCall![0].cursor).toBeUndefined();
    expect(host.textContent).toContain('No generations or trial runs match.');
  });
});
