import type { RuntimeApi } from '@/api/client';
import type { ForgeSourceItemVm, ForgeSourceSummaryVm } from '@/types';

// Synthetic library shared by the App DOM test and the isolated browser preview.
export const materialLocators: ForgeSourceItemVm[] = [
  { catalog: 'profile', source: 'global', assetId: 'fixture.review-role' },
  { catalog: 'skill', source: 'global', assetId: 'fixture.check' },
  { catalog: 'plugin', source: 'global', assetId: 'fixture.report' },
];
const summaries: ForgeSourceSummaryVm[] = [
  { name: 'Reviewer', source: 'global', kind: 'role', description: 'Review release readiness.' },
  { name: 'Verify', source: 'global', kind: 'skill', description: 'Check the release checklist.' },
  { name: 'Report', source: 'global', kind: 'command', description: 'Summarize the findings.' },
];
export const directFixture = { agentType: 'codex', modelId: 'fixture-model', autoAccept: false };
export function forgeMaterialFixture(direct = false) {
  return {
    getProfiles: async () => ({ profiles: [{ id: materialLocators[0].assetId, name: summaries[0].name, scope: 'user', isBuiltIn: false,
      summary: summaries[0].description, content: 'private role file contents', path: '/private/role', createdAt: '2026-01-01', updatedAt: '2026-01-01', dynamicTemplate: false }] }),
    listSkills: async () => ({ global: [{ assetId: materialLocators[1].assetId, name: summaries[1].name, source: 'global', description: summaries[1].description,
      agentSource: '.maling', directoryPath: '/private/skill', syncedAgentTypes: [], loadWarnings: [] }], project: [] }),
    listInstalledPlugins: async () => [{ pluginId: 'fixture-tools', providerId: 'workbuddy', provenance: 'desktop-registry', name: 'Release tools', displayName: 'Release tools',
      version: '', description: '', source: 'global', assets: [{ assetId: materialLocators[2].assetId, name: summaries[2].name, kind: 'command', importable: false, description: summaries[2].description }] }],
    getConversationRunMode: async () => direct ? { mode: 'direct', directConfig: directFixture } : { mode: 'workflow' },
    forgeItemsDigest: async (_projectId, items) => items.map(item => {
      const index = materialLocators.findIndex(it => item.catalog === it.catalog && item.assetId === it.assetId && item.source === it.source);
      if (index < 0) throw { code: 'plugin.asset-unavailable', params: {} };
      return summaries[index];
    }),
  } satisfies Partial<RuntimeApi>;
}
