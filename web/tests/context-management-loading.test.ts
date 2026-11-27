import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const source = readFileSync(fileURLToPath(new URL('../src/pages/ContextManagementPage.tsx', import.meta.url)), 'utf8');
const appSource = readFileSync(fileURLToPath(new URL('../src/App.tsx', import.meta.url)), 'utf8');

describe('context management loading boundaries', () => {
  it('loads only profile data when the profile tab mounts', () => {
    expect(source).not.toContain('getConversationSidebar');
    expect(source).toContain("const needsSkillContext = activeTab === 'skills' || skillSheetMode === 'create';");
    expect(source).toContain('if (!needsSkillContext) return;');
    expect(source).toContain("reads.current.run('forge-workspaces', getConversationWorkspaces)");
    expect(source).not.toContain('getAgentRegistry().then(setAgentRegistry)');
    expect(source).toContain("if (activeTab === 'profiles'");
    expect(source).toContain("useEffect(() => { if (activeTab === 'mcp'");
    expect(source).toContain("if (activeTab !== 'skills') return;");
  });

  it('reuses the app-level persisted agent registry for MCP compatibility', () => {
    // 页面的 agent 注册表由 App 层下发（页面可以再有其他入口型 props，如铸造素材入口）。
    expect(source).toMatch(/export function ContextManagementPage\(\{ agentRegistry, onAgentRegistryChange,[^}]*\}: ContextManagementPageProps\)/);
    expect(source).not.toContain('useState<AgentRegistryVm | null>(null)');
    expect(appSource).toMatch(/<ContextManagementPage agentRegistry=\{agentRegistry\} onAgentRegistryChange=\{setAgentRegistry\}[^>]*\/>/);
    expect(appSource).toContain('listen<ManagedAgentVm>(AGENT_REGISTRY_UPDATED_EVENT');
  });
});
