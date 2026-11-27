/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const listInstalledPluginsMock = vi.fn();
const importPluginAssetMock = vi.fn();
const selectWorkBuddySourceMock = vi.fn();
const forgetWorkBuddySourceMock = vi.fn();
vi.mock('@/api', () => ({
  selectWorkBuddySource: (...args: unknown[]) => selectWorkBuddySourceMock(...args),
  forgetWorkBuddySource: (...args: unknown[]) => forgetWorkBuddySourceMock(...args),
  importPluginAsset: (...args: unknown[]) => importPluginAssetMock(...args),
  listInstalledPlugins: (...args: unknown[]) => listInstalledPluginsMock(...args),
}));

import { PluginForgeAssets } from '@/components/PluginForgeAssets';
import type { InstalledPluginVm, ForgeSourceItemVm } from '@/types';
import '@/i18n';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Plugin 页签(工作流铸造入口 C):只在页签激活时扫描已安装的 Claude Code plugin,
 * 把 skill / command / agent 作为可勾选的铸造素材。加载、空、失败三种状态可区分;
 * 同一时刻只有一个扫描,迟到响应不覆盖新结果。
 */
const deployTools: InstalledPluginVm = {
  providerId: 'claude-acp', provenance: 'host-inventory',
  name: 'deploy-tools',
  displayName: 'Deploy Tools',
  description: 'Deployment tooling',
  version: '1.2.0',
  pluginId: 'fixture-install',
  source: 'global',
  assets: [
    { importable: true, kind: 'skill', name: 'deploy', description: 'Run a deployment', assetId: 'fixture-deploy' },
    { importable: false, kind: 'command', name: 'release', description: 'Cut a release', assetId: 'fixture-release.md' },
    { importable: true, kind: 'agent', name: 'reviewer', description: '', assetId: 'fixture-reviewer.md' },
  ],
};
const solo: InstalledPluginVm = {
  providerId: 'claude-acp', provenance: 'host-inventory',
  name: 'solo',
  displayName: 'Solo',
  description: '',
  version: '',
  pluginId: 'fixture-solo',
  source: 'project',
  assets: [],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function until(condition: () => boolean, rounds = 20) {
  for (let round = 0; round < rounds; round += 1) {
    if (condition()) return;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  if (!condition()) throw new Error('condition not reached');
}

const checkboxes = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>('[role="checkbox"]')];

describe('PluginForgeAssets', () => {
  let host: HTMLDivElement;
  let root: Root;
  let selected: ForgeSourceItemVm[];
  let listed: ForgeSourceItemVm[] | null;
  let loadingFlags: boolean[];

  const render = async (props: Partial<React.ComponentProps<typeof PluginForgeAssets>> = {}) => {
    const onToggleItem = vi.fn((item: ForgeSourceItemVm) => {
      selected = selected.some((candidate) => candidate.assetId === item.assetId) ? selected.filter((it) => it.assetId !== item.assetId) : [...selected, item];
    });
    await act(async () => root.render(
      <PluginForgeAssets
        canGenerate readOnly={false} onImported={() => {}}
        projectId={null}
        projectName={null}
        refreshToken={0}
        isItemSelected={(item) => selected.some((candidate) => candidate.assetId === item.assetId)}
        onToggleItem={onToggleItem}
        onItemsListed={(items) => { listed = items; }}
        onLoadingChange={(flag) => { loadingFlags.push(flag); }}
        {...props}
      />,
    ));
    return onToggleItem;
  };

  it('imports only after confirmation with the selected provider and install identity', async () => {
    listInstalledPluginsMock.mockResolvedValue([deployTools]);
    importPluginAssetMock.mockReset().mockResolvedValue(undefined);
    await render();
    await until(() => !!host.querySelector('[data-plugin-provider]'));
    const importButton = [...host.querySelectorAll('button')].find(b => b.textContent === '导入 Skill')!;
    await act(async () => importButton.click());
    expect(importPluginAssetMock).not.toHaveBeenCalled();
    const confirm = [...document.querySelectorAll('[role="alertdialog"] button')].find(b => b.textContent === '确认') as HTMLButtonElement;
    await act(async () => confirm.click());
    expect(importPluginAssetMock).toHaveBeenCalledWith({ assetId: 'fixture-deploy', projectId: null });
    expect(host.textContent).toContain('已导入');
  });

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    listInstalledPluginsMock.mockReset();
    selected = [];
    listed = null;
    loadingFlags = [];
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it('scans on mount, shows a loading state, then lists plugins and their forgeable assets', async () => {
    const scan = deferred<InstalledPluginVm[]>();
    listInstalledPluginsMock.mockReturnValue(scan.promise);
    await render();
    // 扫描未返回前是加载态,不是“没有 plugin”。
    expect(listInstalledPluginsMock).toHaveBeenCalledTimes(1);
    expect(listInstalledPluginsMock).toHaveBeenCalledWith(null);
    expect(host.textContent).toContain('正在读取插件来源');
    expect(host.textContent).not.toContain('未发现已安装');
    expect(loadingFlags).toEqual([true]);

    await act(async () => scan.resolve([deployTools, solo]));
    expect(host.textContent).not.toContain('正在读取插件来源');
    // 有素材的 plugin 逐条列出;没有可读载体的条目不出现,并说明该 plugin 没有可用素材。
    expect(host.querySelector('[data-plugin-name="deploy-tools"]')?.textContent).toContain('Deploy Tools');
    expect(checkboxes(host).map((box) => box.getAttribute('aria-label'))).toEqual(['选择 deploy', '选择 release', '选择 reviewer']);
    expect(host.textContent).not.toContain('ghost');
    expect(host.querySelector('[data-plugin-name="solo"]')?.textContent).toContain('没有可作为素材');
    expect(loadingFlags).toEqual([true, false]);
    expect([...host.querySelectorAll('button')].some(b => b.textContent === '从目录导入')).toBe(true);
    expect(selectWorkBuddySourceMock).not.toHaveBeenCalled();
    // 清单交给父级:父级据此维护勾选与“生成工作流”的素材列表。
    expect(listed).toEqual(deployTools.assets.map(({assetId}) => ({assetId, catalog: 'plugin', source: 'global'})));
  });

  it('leaves selection state with the parent and reports toggles', async () => {
    listInstalledPluginsMock.mockResolvedValue([deployTools]);
    const onToggleItem = await render();
    await until(() => checkboxes(host).length === 3);

    expect(checkboxes(host)[0].getAttribute('aria-checked')).toBe('false');
    await act(async () => checkboxes(host)[0].click());
    expect(onToggleItem).toHaveBeenCalledTimes(1);
    expect(onToggleItem.mock.calls[0][0]).toMatchObject({ assetId: 'fixture-deploy', catalog: 'plugin', source: 'global' });
    // 组件不持有勾选:父级更新选择并重渲染后 checked 才变化。
    expect(checkboxes(host)[0].getAttribute('aria-checked')).toBe('false');
    await render();
    expect(checkboxes(host)[0].getAttribute('aria-checked')).toBe('true');
  });

  it('shows the empty state only after a successful scan found nothing', async () => {
    listInstalledPluginsMock.mockResolvedValue([]);
    await render();
    await until(() => host.textContent?.includes('此范围没有插件或手动来源。') ?? false);
    expect(checkboxes(host)).toHaveLength(0);
  });

  it('keeps the last good list when a refresh fails, and retries through the same scan', async () => {
    listInstalledPluginsMock.mockResolvedValueOnce([deployTools]).mockRejectedValueOnce({ code: 'app.unexpected', params: {} });
    await render();
    await until(() => checkboxes(host).length === 3);

    // 父级刷新按钮触发重新扫描。
    await render({ refreshToken: 1 });
    await until(() => host.querySelector('[role="alert"]') !== null);
    // 局部失败不清空已展示的内容。
    expect(checkboxes(host).length).toBe(3);
    expect(loadingFlags).toEqual([true, false, true, false]);

    listInstalledPluginsMock.mockResolvedValue([deployTools, solo]);
    await act(async () => [...host.querySelectorAll('button')].find((button) => button.textContent?.includes('重试'))?.click());
    await until(() => host.querySelector('[role="alert"]') === null);
    expect(listInstalledPluginsMock).toHaveBeenCalledTimes(3);
    expect(host.querySelectorAll('[data-plugin-name]')).toHaveLength(2);
  });

  it('drops a late response from a superseded scan', async () => {
    const first = deferred<InstalledPluginVm[]>();
    const second = deferred<InstalledPluginVm[]>();
    listInstalledPluginsMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    await render();
    // 切换了项目:新扫描取代旧扫描。
    await render({ projectId: 'project-fixture' });
    expect(listInstalledPluginsMock).toHaveBeenLastCalledWith('project-fixture');

    await act(async () => second.resolve([solo]));
    await act(async () => first.resolve([deployTools]));
    // 旧请求晚返回,不覆盖新项目的结果。
    expect(host.querySelectorAll('[data-plugin-name]')).toHaveLength(1);
    expect(host.querySelector('[data-plugin-name="solo"]')).not.toBeNull();
    expect(loadingFlags.at(-1)).toBe(false);
  });

  it('resets the loading flag when unmounted mid-scan', async () => {
    const scan = deferred<InstalledPluginVm[]>();
    listInstalledPluginsMock.mockReturnValue(scan.promise);
    await render();
    expect(loadingFlags).toEqual([true]);
    await act(async () => root.render(null));
    // 切走页签后父级的刷新按钮不留转圈。
    expect(loadingFlags).toEqual([true, false]);
    await act(async () => scan.resolve([deployTools]));
    expect(listed).toBeNull();
  });

  it('withdraws the previous inventory and its confirmation before a new scope resolves or fails', async () => {
    const scan = deferred<InstalledPluginVm[]>();
    listInstalledPluginsMock.mockResolvedValueOnce([deployTools]).mockReturnValueOnce(scan.promise);
    await render({ projectId: 'one' });
    await act(async () => [...host.querySelectorAll('button')].find(b => b.textContent === '导入 Skill')!.click());
    expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
    await render({ projectId: 'two' });
    expect(host.querySelector('[data-plugin-name]')).toBeNull();
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    await act(async () => scan.reject(new Error('fixture')));
    expect(host.querySelector('[data-plugin-name]')).toBeNull();
  });

  it('shows different source identities for same-named CLI and manually selected definitions', async () => {
    listInstalledPluginsMock.mockResolvedValue([
      { ...deployTools, providerId: 'codebuddy-code', provenance: 'host-inventory' },
      { ...deployTools, pluginId: 'manual', providerId: 'workbuddy', provenance: 'manual-directory' },
    ]);
    await render();
    expect(host.querySelectorAll('[data-plugin-name][data-slot="card"]')).toHaveLength(2);
    expect(host.textContent).toContain('CodeBuddy Code（CLI）');
    expect(host.textContent).toContain('WorkBuddy · 手动来源');
  });

  it('labels bundled and registered desktop material without implying enablement; only a pinned source can be forgotten', async () => {
    listInstalledPluginsMock.mockResolvedValue([
      { ...deployTools, providerId: 'workbuddy', provenance: 'desktop-bundled' },
      { ...deployTools, pluginId: 'registered', providerId: 'workbuddy', provenance: 'desktop-registry', canForget: true },
    ]);
    await render();
    const [bundled, registered] = host.querySelectorAll('[data-plugin-name]');
    expect(bundled.textContent).toContain('WorkBuddy · 桌面版内置');
    expect(bundled.textContent).not.toContain('移除来源');
    expect(registered.textContent).toContain('已登记（不表示已启用）');
    expect(registered.textContent).toContain('移除来源');
  });

  it('still allows import without generation, but never offers writes in a read-only view', async () => {
    listInstalledPluginsMock.mockResolvedValue([deployTools]);
    await render({ canGenerate: false });
    expect(checkboxes(host)).toHaveLength(0);
    expect(host.textContent).toContain('导入 Skill');
    await render({ canGenerate: true, readOnly: true });
    expect(checkboxes(host)).toHaveLength(0);
    expect(host.textContent).not.toContain('导入 Skill');
  });

  it('shows the unsupported directory in the affected Skill row without disabling generation', async () => {
    const blocked = { ...deployTools.assets[0], importable: false, importBlocker: { code: 'unsupported-directory', path: 'examples' } };
    listInstalledPluginsMock.mockResolvedValue([{ ...deployTools, assets: [blocked, ...deployTools.assets.slice(1)] }]);
    const toggle = await render();
    const [row, command, agent] = host.querySelectorAll('li');
    expect(row.textContent).toContain('不支持的目录');
    expect(row.textContent).toContain('examples');
    expect(row.textContent).toContain('可用于生成');
    expect(row.textContent).not.toContain('导入 Skill');
    expect(command.textContent).not.toContain('examples');
    expect(agent.textContent).not.toContain('examples');
    await act(async () => checkboxes(row)[0].click());
    expect(toggle).toHaveBeenCalledExactlyOnceWith({ assetId: blocked.assetId, catalog: 'plugin', source: 'global' });
  });

  it.each([
    [{ code: 'unsupported-file' }, '不支持的文件类型'],
    [{ code: 'hidden-entry' }, '隐藏项'],
    [{ code: 'special-entry' }, '特殊文件系统条目'],
    [{ code: 'depth-limit', limit: 5 }, '超过 5 层'],
    [{ code: 'file-size-limit', limit: 1536 }, '超过 1.5 KiB'],
    [{ code: 'bundle-size-limit', limit: 3 * 1024 * 1024 }, '整个包超过 3 MiB'],
    [{ code: 'file-count-limit', limit: 17 }, '超过 17 个'],
    [{ code: 'file-count-limit' }, '文件数量超过上限'],
    [{ code: 'file-size-limit', limit: -1 }, '单个文件超过大小上限'],
    [{ code: 'inspection-incomplete' }, '未能完整检查包内容'],
    [{ code: 'unrecognized-code', path: 'not-for-display' }, '具体原因未知'],
    [undefined, '具体原因未知'],
  ] as const)('explains the reported import blocker without inventing missing details (%j)', async (importBlocker, expected) => {
    listInstalledPluginsMock.mockResolvedValue([{ ...deployTools, assets: [{ ...deployTools.assets[0], importable: false, importBlocker }] }]);
    await render();
    const row = host.querySelector('li')!;
    expect(row.textContent).toContain(expected);
    expect(row.textContent).not.toMatch(/undefined|null|\{\{|plugin\.importBlocker|not-for-display|unrecognized-code|fixture-deploy/);
  });

  it('wraps long bundle-relative paths, but does not expose absolute paths or parent traversal', async () => {
    const relative = `assets/${'a'.repeat(140)}.py`;
    const assets = [relative, 'C:\\private\\entry.py', '../private/entry.py'].map((path, index) => ({
      ...deployTools.assets[0], assetId: `blocked-${index}`, importable: false, importBlocker: { code: 'unsupported-file', path },
    }));
    listInstalledPluginsMock.mockResolvedValue([{ ...deployTools, assets }]);
    await render();
    const [safe, absolute, traversal] = host.querySelectorAll('li');
    expect(safe.querySelector('bdi.break-all')?.textContent).toBe(relative);
    for (const row of [absolute, traversal]) {
      expect(row.textContent).toContain('不支持的文件类型');
      expect(row.textContent).not.toContain('private');
    }
  });

  it('offers to forget an unavailable manual source, not its files or a same-named host source', async () => {
    const stale = { ...deployTools, pluginId: 'old', providerId: 'workbuddy', provenance: 'manual-directory', unavailable: true, assets: [] };
    listInstalledPluginsMock.mockResolvedValueOnce([deployTools, stale]).mockResolvedValue([deployTools]);
    await render();
    expect(host.textContent).toContain('来源已失效');
    forgetWorkBuddySourceMock.mockReset().mockResolvedValue(true);
    const forget = [...host.querySelectorAll('button')].find(b => b.textContent === '移除来源')!;
    await act(async () => forget.click());
    const dialogButton = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find(b => b.textContent === label)!;
    await act(async () => dialogButton('取消').click());
    expect(forgetWorkBuddySourceMock).not.toHaveBeenCalled();
    await act(async () => forget.click());
    await act(async () => { const ok = dialogButton('确认'); ok.click(); ok.click(); });
    expect(forgetWorkBuddySourceMock).toHaveBeenCalledExactlyOnceWith(null, 'old');
    expect(host.querySelectorAll('[data-plugin-name]')).toHaveLength(1);
    expect(host.querySelector('[data-plugin-provider="claude-acp"]')).not.toBeNull();
  });

  it('replaces only the chosen scope’s manual source; cancel/duplicate picker calls have no effect', async () => {
    const manual: InstalledPluginVm = { ...solo, providerId: 'workbuddy', provenance: 'manual-directory', pluginId: 'old' };
    const replacement = { ...manual, pluginId: 'new', name: 'replacement' };
    listInstalledPluginsMock.mockResolvedValueOnce([deployTools, manual]).mockResolvedValue([deployTools, replacement]);
    const wait = deferred<InstalledPluginVm | null>();
    selectWorkBuddySourceMock.mockReset().mockReturnValueOnce(wait.promise).mockResolvedValueOnce(replacement);
    await render({ projectId: 'one', projectName: 'Fixture' });
    const choose = [...host.querySelectorAll('button')].find(b => b.textContent === '更换目录');
    expect(choose).toBeDefined();
    expect(host.textContent).toContain('只加入素材列表，不安装插件');
    if (!choose) throw new Error('Missing change-directory button');
    await act(async () => { choose.click(); choose.click(); });
    expect(selectWorkBuddySourceMock).toHaveBeenCalledExactlyOnceWith('one');
    await act(async () => wait.resolve(null));
    expect(host.querySelector('[data-plugin-name="solo"]')).not.toBeNull();
    await act(async () => choose.click());
    expect(host.querySelector('[data-plugin-name="solo"]')).toBeNull();
    expect(host.querySelector('[data-plugin-name="replacement"]')).not.toBeNull();
    expect(host.querySelector('[data-plugin-name="deploy-tools"]')).not.toBeNull();
    expect(listInstalledPluginsMock).toHaveBeenCalledTimes(2);
  });

  it('coalesces same-scope refreshes to one trailing inventory read', async () => {
    const scan = deferred<InstalledPluginVm[]>();
    listInstalledPluginsMock.mockReturnValueOnce(scan.promise).mockResolvedValueOnce([deployTools]);
    await render(); await render({ refreshToken: 1 }); await render({ refreshToken: 2 });
    expect(listInstalledPluginsMock).toHaveBeenCalledTimes(1);
    await act(async () => scan.resolve([]));
    expect(listInstalledPluginsMock).toHaveBeenCalledTimes(2);
    expect(host.querySelector('[data-plugin-name="deploy-tools"]')).not.toBeNull();
  });

  it('renders without checkboxes when forging is unavailable', async () => {
    listInstalledPluginsMock.mockResolvedValue([deployTools]);
    await render({ canGenerate: false });
    await until(() => host.querySelector('[data-plugin-name="deploy-tools"]') !== null);
    expect(checkboxes(host)).toHaveLength(0);
    expect(host.textContent).toContain('deploy');
  });
});
