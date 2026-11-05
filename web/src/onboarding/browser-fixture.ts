import type { EnvironmentReport, SetupAgent, SetupApi, SetupStatus } from './types';

let browserApi: SetupApi | undefined;
export function browserSetupApi(): SetupApi {
  return browserApi ??= createBrowserSetupApi();
}

/** Explicit development fixture: it never diagnoses or writes the host installation. */
export function createBrowserSetupApi(): SetupApi {
  const fixture = new URLSearchParams(location.search).get('onboardingFixture');
  const status: SetupStatus = {
    platform: 'windows',
    state: { status: 'pending', gitDecision: 'undecided' }, gitMinimumVersion: '2.36.0', language: 'zh-cn', defaultExecutionProfile: null,
    appearance: { schemaVersion: 2, themeId: 'builtin.gold-band', colorScheme: 'dark', visualQualityByTheme: {} },
    environmentPolicy: { node: { ranges: ['>=22.20.0 <23', '>=24 <25'], recommendedVersion: '24.22.0', helpUrl: 'https://nodejs.org/en/download' }, npm: { ranges: ['>=10.9 <12'] }, gitHelpUrl: 'https://git-scm.com/downloads' },
  };
  const agent: SetupAgent = {
    agentType: 'fixture-agent', displayName: 'Local Agent',
    supportedModels: fixture === 'legacy-models' ? null : [{ id: 'fixture-model', name: 'Standard' }],
    supportedModes: [{ id: 'default', name: 'Ask before changes' }],
    configOptions: [{ id: 'output', name: 'Output format', currentValue: 'text', options: [{ value: 'text', name: 'Text' }] }],
    modelBoundCatalogs: fixture === 'legacy-models' ? null : {
      'fixture-model': [{ id: 'reasoning', name: 'Reasoning effort', category: 'thought_level', currentValue: 'medium', options: [{ value: 'medium', name: 'Medium' }, { value: 'high', name: 'High' }] }],
    },
  };
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  let environment: { requestId: string; report?: EnvironmentReport } | null = null;
  let checks = 0;
  const report = (failed = false): EnvironmentReport => ({ runtimeReady: !failed, checks: (['git', 'node', 'npm', 'npx'] as const).map((tool) => ({ tool, status: failed && tool === 'npm' ? 'missing' : 'ready', version: tool === 'git' ? '2.53.0' : tool === 'node' ? '24.22.0' : '11.19.1', executablePath: `C:\\Users\\示例用户\\Programs with spaces\\${tool}.exe` })) });
  const unverifiedNodeReport = (): EnvironmentReport => ({
    runtimeReady: true,
    checks: report().checks.map((check) => check.tool === 'node' ? {
      ...check,
      status: 'unverified',
      version: '23.11.0',
      error: { code: 'setup.runtime-version-unverified', params: { tool: 'node', version: '23.11.0' } },
    } : check),
  });
  const blockedReport = (): EnvironmentReport => ({ ...report(), runtimeReady: false, checks: report().checks.map((check) => check.tool === 'npx' ? { ...check, status: 'incompatible', nodeExecutablePath: 'C:\\Other Node\\node.exe', error: { code: 'setup.runtime-incompatible', params: { reason: 'node-mismatch', expectedNodePath: 'C:\\Users\\示例用户\\Programs with spaces\\node.exe', actualNodePath: 'C:\\Other Node\\node.exe' } } } : check) });
  const requireEnvironment = () => {
    if (!environment?.report) throw { code: 'setup.environment-check-required', params: {} };
    if (!environment.report.runtimeReady) throw { code: 'setup.runtime-incompatible', params: { environment: environment.report } };
  };
  return {
    status: async () => status,
    environment: async (requestId) => {
      environment = { requestId };
      await Promise.resolve();
      const failed = fixture === 'runtime-failure' && checks === 0;
      const result = fixture === 'runtime-path-mismatch' && checks === 0
        ? blockedReport()
        : fixture === 'unverified-node'
          ? unverifiedNodeReport()
          : report(failed);
      if (environment?.requestId === requestId) { checks += 1; environment.report = result; }
      return result;
    },
    skipGit: async () => { status.state = { status: 'pending', gitDecision: 'skip' }; },
    cancelEnvironment: async (requestId) => { if (environment?.requestId === requestId) environment = null; },
    start: async (receive) => {
      requireEnvironment();
      const generation = crypto.randomUUID();
      receive({ type: 'checking', generation, agentId: agent.agentType, label: agent.displayName });
      timers.set(generation, setTimeout(() => {
        receive({ type: 'ready', generation, agent });
        receive({ type: 'unavailable', generation, agentId: 'needs-login' });
        receive({ type: 'finished', generation, nextCursor: null });
      }, 600));
      return generation;
    },
    cancel: async (generation) => { clearTimeout(timers.get(generation)); timers.delete(generation); },
    probeModel: async () => {
      if (fixture === 'legacy-models') throw { code: 'acp.model-config-unavailable', params: {} };
      return agent;
    },
    preview: async () => ({ fingerprint: 'fixture-preview', slotCount: 4, templates: [{ id: 'default', name: 'Default workflow', slotCount: 3, previousBindings: [] }, { id: 'lightweight', name: 'Lightweight workflow', slotCount: 1, previousBindings: [] }] }),
    apply: async (request) => { requireEnvironment(); status.state = { status: 'completed' }; status.defaultExecutionProfile = request.selection.profile; },
    preferences: async (language, appearance) => { status.language = language; status.appearance = appearance; },
  };
}
