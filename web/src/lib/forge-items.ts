import type { TFunction } from 'i18next';
import type { InstalledPluginVm, ForgeSourceItemVm, ForgeSourceSummaryVm, ProfileVm, SkillMetaVm } from '../types';

export const MAX_FORGE_ITEMS = 32;

export function profileForgeItem(profile: Pick<ProfileVm, 'id' | 'scope'>): ForgeSourceItemVm {
  return { catalog: 'profile', source: profile.scope === 'built-in' ? 'built-in' : 'global', assetId: profile.id };
}

export function skillForgeItem(skill: SkillMetaVm): ForgeSourceItemVm {
  return { catalog: 'skill', source: skill.source, assetId: skill.assetId };
}

/** Reconcile only the catalog/scope covered by a successful inventory read. */
export function pruneForgeItems(selected: Map<string, ForgeSourceItemVm>, inScope: (item: ForgeSourceItemVm) => boolean, listed: ForgeSourceItemVm[]) {
  if (!selected.size) return selected;
  const keys = new Set(listed.map(forgeItemKey));
  const kept = new Map([...selected].filter(([key, item]) => !inScope(item) || keys.has(key)));
  return kept.size === selected.size ? selected : kept;
}

export function forgeItemKey(item: ForgeSourceItemVm): string {
  return `${item.catalog}:${item.source}:${item.assetId}`;
}

export function pluginForgeAssets(plugin: InstalledPluginVm) {
  return plugin.assets.map(({ assetId, name, kind, description }) => ({
    item: { catalog: 'plugin' as const, source: plugin.source, assetId },
    name, kind, description,
  }));
}

export function pluginForgeItems(plugin: InstalledPluginVm): ForgeSourceItemVm[] {
  return pluginForgeAssets(plugin).map((entry) => entry.item);
}

/**
 * 铸造素材清单 → composer 预填正文（入口 B/C）。
 *
 * 后端只返回结构化摘要（名称、种类、来源、描述），预填文案由前端按界面语言拼装。
 * 用户可以直接编辑这段正文；素材全文由后端在发送时注入首条 prompt 的隐藏区段。
 */
export function forgeItemsPrefillMarkdown(t: TFunction, items: ForgeSourceSummaryVm[]): string {
  return items
    .map((item) => {
      const origin = item.origin?.trim();
      const source = [
        t(`forge.itemsPrefill.kind.${item.kind}`),
        t(`forge.itemsPrefill.scope.${item.source}`),
        ...(origin ? [origin] : []),
      ].join(' · ');
      const description = item.description.trim();
      return `- **${item.name}**（${source}）${description ? `：${description}` : ''}`;
    })
    .join('\n');
}
