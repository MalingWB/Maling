/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api', () => ({
  getProfiles: vi.fn(), getConversationWorkspaces: vi.fn(), listSkills: vi.fn(),
  listInstalledPlugins: vi.fn(), listProjectSkills: vi.fn(), importPluginAsset: vi.fn(),
  importProfilesFromFiles: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('@/components/prompt-kit/markdown', () => ({
  Markdown: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

import { open } from '@tauri-apps/plugin-dialog';
import { getProfiles, importProfilesFromFiles, getConversationWorkspaces, listSkills, listProjectSkills, listInstalledPlugins, importPluginAsset } from '@/api';
import i18n from '@/i18n';
import { ContextManagementPage } from '@/pages/ContextManagementPage';
import { TooltipProvider } from '@/components/ui/tooltip';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.mocked(getProfiles).mockResolvedValue({ profiles: [] });
  vi.mocked(open).mockResolvedValue(null);
  await i18n.changeLanguage('zh-CN');
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root.render(
    <TooltipProvider>
      <ContextManagementPage agentRegistry={null} onAgentRegistryChange={() => {}} />
    </TooltipProvider>,
  ));
});

afterEach(async () => {
  await act(async () => root.unmount());
  document.body.replaceChildren();
  vi.resetAllMocks();
  vi.unstubAllGlobals();
});

async function activateTab(label: string) {
  const tab = [...host.querySelectorAll<HTMLElement>('[role="tab"]')].find(el => el.textContent === label)!;
  await act(async () => { tab.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 })); });
}

describe('plugin imports from standalone context management', () => {
  it.each(['skill', 'agent'] as const)('refreshes only the %s destination when the user returns', async (kind) => {
    vi.mocked(getConversationWorkspaces).mockResolvedValue([]);
    vi.mocked(listInstalledPlugins).mockResolvedValue([{ pluginId: 'tools', providerId: 'workbuddy', provenance: 'manual-directory', source: 'global', name: 'Tools', displayName: 'Tools', description: '', version: '', assets: [{ assetId: 'fixture', name: 'Fixture', description: '', kind, importable: true }] }]);
    vi.mocked(listSkills).mockResolvedValue({ global: [], project: [] });
    await activateTab(i18n.t('contextManagement.tabs.skills'));
    await activateTab('Plugin');
    const importButton = [...host.querySelectorAll('button')].find(b => b.textContent === i18n.t(kind === 'skill' ? 'plugin.importSkill' : 'plugin.importRole'))!;
    await act(async () => importButton.click());
    const confirm = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find(b => b.textContent === i18n.t('common.confirm'))!;
    await act(async () => { confirm.click(); confirm.click(); });
    expect(importPluginAsset).toHaveBeenCalledExactlyOnceWith({ projectId: null, assetId: 'fixture' });
    expect(listSkills).toHaveBeenCalledTimes(1); expect(getProfiles).toHaveBeenCalledTimes(1);
    await activateTab(i18n.t('contextManagement.skills.globalTab'));
    expect(listSkills).toHaveBeenCalledTimes(kind === 'skill' ? 2 : 1);
    await activateTab(i18n.t('contextManagement.profileManagement'));
    expect(getProfiles).toHaveBeenCalledTimes(kind === 'agent' ? 2 : 1);
    expect(listProjectSkills).not.toHaveBeenCalled(); expect(listInstalledPlugins).toHaveBeenCalledTimes(1);
  });
});

describe('shared Forge materials', () => {
  const role = { id: 'stored.role-id', name: 'Reviewer', scope: 'user' as const, isBuiltIn: false,
    summary: '', content: 'private definition', path: '/private/role', dynamicTemplate: false, createdAt: '', updatedAt: '' };
  const ws = { projectId: 'p1', workspacePath: '/fixture/one', name: 'Fixture' };
  const box = (name: string) => host.querySelector<HTMLButtonElement>(`[role="checkbox"][aria-label="${i18n.t('contextManagement.skills.forgeSelect', { name })}"]`);
  const generate = (count: number) => [...host.querySelectorAll('button')].find(b => b.textContent === i18n.t('contextManagement.skills.forgeAction', { count }))!;
  async function mountForge(extraProfiles: Awaited<ReturnType<typeof getProfiles>>['profiles'] = []) {
    vi.mocked(getProfiles).mockResolvedValue({ profiles: [role, ...extraProfiles] });
    const handoff = vi.fn().mockResolvedValue(true);
    await act(async () => root.render(<TooltipProvider><ContextManagementPage key="forge" agentRegistry={null} onAgentRegistryChange={() => {}} onForgeItems={handoff} /></TooltipProvider>));
    expect(box(role.name)).not.toBeNull();
    await act(async () => box(role.name)!.click());
    return handoff;
  }
  it('loads anchor projects only on Role action; a failed handoff keeps selection and a successful retry clears it', async () => {
    vi.mocked(getConversationWorkspaces).mockResolvedValue([ws]);
    const handoff = await mountForge();
    expect(getConversationWorkspaces).not.toHaveBeenCalled();
    expect(listSkills).not.toHaveBeenCalled();
    handoff.mockResolvedValueOnce(false);
    await act(async () => generate(1).click());
    expect(handoff).toHaveBeenCalledExactlyOnceWith([{ catalog: 'profile', source: 'global', assetId: role.id }], ws.projectId);
    expect(box(role.name)?.getAttribute('aria-checked')).toBe('true');
    await act(async () => generate(1).click());
    expect(generate(0).disabled).toBe(true);
  });
  it('sends a built-in Role by its stored ID, never its display name, body or permissions', async () => {
    vi.mocked(getConversationWorkspaces).mockResolvedValue([ws]);
    const handoff = await mountForge([{ ...role, scope: 'built-in', isBuiltIn: true, id: 'sys.reviewer' }]);
    await act(async () => box(role.name)!.click());
    await activateTab(i18n.t('contextManagement.builtInSectionTitle'));
    await act(async () => box(role.name)!.click());
    await act(async () => generate(1).click());
    expect(handoff).toHaveBeenCalledExactlyOnceWith([{ catalog: 'profile', source: 'built-in', assetId: 'sys.reviewer' }], ws.projectId);
  });
  it('enforces the combined material cap without dropping any previous choices', async () => {
    const extra = Array.from({ length: 32 }, (_, i) => ({ ...role, id: `extra-${i}`, name: `Extra ${i}` }));
    const handoff = await mountForge(extra);
    await act(async () => { for (const r of extra.slice(0, 31)) box(r.name)!.click(); });
    await act(async () => box(extra[31].name)!.click());
    expect(generate(32).disabled).toBe(false);
    expect(host.textContent).toContain(i18n.t('contextManagement.skills.forgeLimit', {count:32}));
    expect(box(extra[31].name)?.getAttribute('aria-checked')).toBe('false');
    expect(handoff).not.toHaveBeenCalled();
  });
  it('does not substitute a different project or clear roles when project lookup fails or returns no projects', async () => {
    vi.mocked(getConversationWorkspaces).mockRejectedValueOnce({ code: 'app.unexpected', params: {} }).mockResolvedValueOnce([]);
    const handoff = await mountForge();
    await act(async () => generate(1).click());
    expect(generate(1).disabled).toBe(false);
    await act(async () => generate(1).click());
    expect(host.textContent).toContain(i18n.t('contextManagement.skills.forgeProjectUnavailable'));
    expect(generate(1).disabled).toBe(false);
    expect(handoff).not.toHaveBeenCalled();
  });
  it('asks for an anchor for global material and rejects a remembered choice that disappears before confirmation', async () => {
    const two = { projectId: 'p2', workspacePath: '/fixture/two', name: 'Two' };
    localStorage.setItem('gold-band:context-management:skill-project-workspace', two.workspacePath);
    vi.mocked(getConversationWorkspaces).mockResolvedValueOnce([ws, two]).mockResolvedValueOnce([ws]);
    const handoff = await mountForge();
    await act(async () => generate(1).click());
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(dialog.textContent).toContain(two.name);
    await act(async () => [...dialog.querySelectorAll('button')].find(b => b.textContent === i18n.t('contextManagement.skills.forgeAnchorConfirm'))!.click());
    expect(handoff).not.toHaveBeenCalled();
    expect(host.textContent).toContain(i18n.t('contextManagement.skills.forgeProjectUnavailable'));
    expect(generate(1).disabled).toBe(false);
    localStorage.clear();
  });
  it('reconciles against complete successful profile reads, never against search or failed refresh', async () => {
    await mountForge();
    const search = host.querySelector<HTMLInputElement>('input')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'not a match');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(box(role.name)).toBeNull();
    expect(generate(1).disabled).toBe(false);
    const refresh = () => host.querySelector<HTMLButtonElement>(`button[aria-label="${i18n.t('common.refresh')}"]`)!.click();
    vi.mocked(getProfiles).mockRejectedValueOnce({ code:'app.unexpected', params:{} }).mockResolvedValueOnce({ profiles:[] });
    await act(async () => refresh());
    expect(generate(1).disabled).toBe(false);
    await act(async () => refresh());
    expect(generate(0).disabled).toBe(true);
  });
  it('keeps global Role/Skill/Plugin choices across tab changes and a remembered project', async () => {
    localStorage.setItem('gold-band:context-management:skill-project-workspace', ws.workspacePath);
    const two = { projectId: 'p2', workspacePath: '/fixture/two', name: 'Two' };
    vi.mocked(getConversationWorkspaces).mockResolvedValue([ws, two]);
    vi.mocked(listProjectSkills).mockImplementation(async path => [{ name: path === ws.workspacePath ? 'Project A' : 'Project B', assetId: 'local-id', source: 'project', description: '', directoryPath: path, agentSource: '', syncedAgentTypes: [], loadWarnings: [] }]);
    vi.mocked(listSkills).mockResolvedValue({ global: [{ name: 'Audit', assetId: 'skill-a', source: 'global', description: '', directoryPath: '/fixture', agentSource: '', syncedAgentTypes: [], loadWarnings: [] }], project: [] });
    vi.mocked(listInstalledPlugins).mockResolvedValue([{ pluginId: 'tools', providerId: 'workbuddy', provenance: 'manual-directory', source: 'global', name: 'Tools', displayName: '', description: '', version: '', assets: [{ assetId: 'command-a', name: 'Publish', description: '', kind: 'command', importable: false }] }]);
    const handoff = await mountForge();
    await activateTab(i18n.t('contextManagement.tabs.skills'));
    await act(async () => box('Audit')!.click());
    await activateTab('Plugin');
    await act(async () => box('Publish')!.click());
    await activateTab(i18n.t('contextManagement.skills.projectTab'));
    await act(async () => box('Project A')!.click());
    expect(generate(4).disabled).toBe(false);
    HTMLElement.prototype.scrollIntoView ??= () => {};
    await act(async () => { host.querySelector(`[role="combobox"][aria-label="${i18n.t('contextManagement.scope')}"]`)!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); });
    await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(el => el.textContent === two.name)!.click());
    expect(box('Project A')).toBeNull();
    expect(box('Project B')?.getAttribute('aria-checked')).toBe('false');
    expect(generate(3).disabled).toBe(false);
    await activateTab(i18n.t('contextManagement.profileManagement'));
    await act(async () => generate(3).click());
    const dialog = document.querySelector('[role="dialog"]')!;
    await act(async () => [...dialog.querySelectorAll('button')].find(b => b.textContent === i18n.t('contextManagement.skills.forgeAnchorConfirm'))!.click());
    expect(handoff).toHaveBeenCalledExactlyOnceWith([{catalog:'profile',source:'global',assetId:role.id}, {catalog:'skill',source:'global',assetId:'skill-a'}, {catalog:'plugin',source:'global',assetId:'command-a'}], two.projectId);
    localStorage.clear();
  });
});

async function pickImportFiles() {
  const importButton = Array.from(host.querySelectorAll('button')).find(
    (button) => button.textContent === i18n.t('contextManagement.importProfile'),
  );
  expect(importButton).toBeDefined();
  await act(async () => importButton!.click());

  const trigger = document.querySelector<HTMLButtonElement>(
    '[data-slot="profile-import-settings-sheet"] button[aria-haspopup="menu"]',
  );
  expect(trigger).not.toBeNull();
  await act(async () => trigger!.dispatchEvent(
    new MouseEvent('pointerdown', { bubbles: true, button: 0 }),
  ));

  const menuItem = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]')).find(
    (item) => item.textContent === i18n.t('contextManagement.importPickFiles'),
  );
  expect(menuItem).toBeDefined();
  await act(async () => menuItem!.click());
}

describe('profile file import', () => {
  it('offers only .md files and keeps settings open when selection is cancelled', async () => {
    await pickImportFiles();

    expect(open).toHaveBeenCalledExactlyOnceWith({
      multiple: true,
      filters: [{ name: 'Markdown', extensions: ['md'] }],
      title: i18n.t('contextManagement.importPickFilesTitle'),
    });
    expect(importProfilesFromFiles).not.toHaveBeenCalled();
    expect(document.querySelector('[data-slot="profile-import-settings-sheet"]')).not.toBeNull();
  });

  it.each([
    { label: 'a single file', selected: 'C:/roles/developer.md', filePaths: ['C:/roles/developer.md'] },
    {
      label: 'multiple files',
      selected: ['C:/roles/developer.md', 'C:/roles/reviewer.MD'],
      filePaths: ['C:/roles/developer.md', 'C:/roles/reviewer.MD'],
    },
  ])('passes $label to the import API and shows the result', async ({ selected, filePaths }) => {
    vi.mocked(open).mockResolvedValue(selected);
    vi.mocked(importProfilesFromFiles).mockResolvedValue({
      totalScanned: filePaths.length,
      imported: filePaths.map((sourcePath, index) => ({
        sourcePath,
        status: 'imported',
        name: `Imported role ${index + 1}`,
        fallbacks: [],
        importedId: `imported-${index + 1}`,
        error: null,
      })),
      failed: [],
      truncated: false,
    });

    await pickImportFiles();

    expect(importProfilesFromFiles).toHaveBeenCalledExactlyOnceWith(filePaths, false);
    const result = document.querySelector('[data-slot="profile-import-result-sheet"]');
    expect(result).not.toBeNull();
    for (const sourcePath of filePaths) expect(result!.textContent).toContain(sourcePath);
  });
});
