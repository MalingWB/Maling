import { describe, expect, it } from 'vitest';
import i18n from 'i18next';
import { forgeItemKey, forgeItemsPrefillMarkdown, pluginForgeAssets, pluginForgeItems } from '@/lib/forge-items';
import type { InstalledPluginVm, ForgeSourceItemVm, ForgeSourceSummaryVm } from '@/types';

const t = i18n.t.bind(i18n);

const plugin: InstalledPluginVm = {
  providerId: 'claude-acp', provenance: 'host-inventory',
  name: 'deploy-tools',
  displayName: 'Deploy Tools',
  description: 'Deployment tooling',
  version: '1.2.0',
  pluginId: 'fixture-install', source: 'global',
  assets: [
    { assetId: 'fixture-skill', kind: 'skill', importable: true, name: 'deploy', description: 'Run a deployment' },
    { assetId: 'fixture-command', kind: 'command', importable: false, name: 'release', description: 'Cut a release' },
    { assetId: 'fixture-inline', kind: 'command', importable: false, name: 'about', description: 'About' },
    { assetId: 'fixture-agent', kind: 'agent', importable: true, name: 'reviewer', description: 'Reviews changes' },
  ],
};

describe('pluginForgeAssets', () => {
  it('retains the listed ID without requiring an asset path or inline body', () => {
    const listed = { ...plugin, assets: [{ kind: 'command' as const, importable: false, name: 'fixture', description: '', assetId: 'fixture-id' }] };
    expect(pluginForgeItems(listed)[0]).toEqual({ catalog: 'plugin', source: 'global', assetId: 'fixture-id' });
  });
  it('projects only the selector for every asset kind', () => {
    expect(pluginForgeItems(plugin)).toEqual(plugin.assets.map(({assetId}) => ({assetId, catalog: 'plugin', source: 'global'})));
    // 展示描述随素材一起返回。
    expect(pluginForgeAssets(plugin).map((entry) => entry.description)).toEqual(['Run a deployment', 'Cut a release', 'About', 'Reviews changes']);
  });

  it('preserves the project scope', () => {
    expect(pluginForgeItems({ ...plugin, source: 'project' })[0].source).toBe('project');
  });
});

describe('forgeItemKey', () => {
  it('keys the inventory reference by catalog, scope and stable ID', () => {
    const skill: ForgeSourceItemVm = { catalog: 'skill', source: 'project', assetId: 'fixture-id' };
    expect(forgeItemKey(skill)).not.toBe(forgeItemKey({ ...skill, assetId: 'fixture-other' }));
    expect(forgeItemKey(skill)).not.toBe(forgeItemKey({ ...skill, source: 'global' }));
    expect(forgeItemKey(skill)).not.toBe(forgeItemKey({ ...skill, catalog: 'plugin' }));
  });
});

describe('forgeItemsPrefillMarkdown', () => {
  it('lists each selected item with its kind, scope, origin and description', () => {
    const items: ForgeSourceSummaryVm[] = [
      { name: 'code-review', source: 'project', kind: 'skill', origin: null, description: '审查代码变更' },
      { name: 'release', source: 'global', kind: 'command', origin: 'claude-acp · plugin deploy-tools', description: '' },
    ];
    expect(forgeItemsPrefillMarkdown(t, items)).toBe([
      '- **code-review**（skill · 项目）：审查代码变更',
      '- **release**（command · 全局 · claude-acp · plugin deploy-tools）',
    ].join('\n'));
  });

  it('renders an empty list as an empty body', () => {
    expect(forgeItemsPrefillMarkdown(t, [])).toBe('');
  });
});
