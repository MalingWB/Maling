import type { AppErrorVm } from '@/types';
import type { EnvironmentCheck, EnvironmentReport, SetupAgent, SetupEvent } from './types';
export interface SetupDetectionState {
  environment: { phase: 'idle' | 'checking' | 'ready' | 'failed'; report?: EnvironmentReport; progress: Partial<Record<EnvironmentCheck['tool'], EnvironmentCheck>> };
  phase: 'idle' | 'detecting' | 'completed' | 'blocked-environment' | 'failed';
  generation: string;
  agents: Record<string, SetupAgent>;
  pending: Record<string, string>;
  nextCursor: string | null;
}
export const initialDetectionState: SetupDetectionState = {
  environment: { phase: 'checking', progress: {} }, phase: 'idle', generation: '',
  agents: {}, pending: {}, nextCursor: null,
};
type Action =
  | { type: 'environment-started' }
  | { type: 'environment-progress'; check: EnvironmentCheck }
  | { type: 'environment-completed'; report: EnvironmentReport }
  | { type: 'environment-failed' }
  | { type: 'runtime-blocked'; report: EnvironmentReport }
  | { type: 'detection-started'; append: boolean }
  | { type: 'detection-stopped' }
  | { type: 'detection-accepted'; generation: string }
  | { type: 'detection-failed' }
  | { type: 'event'; event: SetupEvent }
  | { type: 'model'; agent: SetupAgent };

export function setupFailure(error: unknown): AppErrorVm {
  if (error && typeof error === 'object' && 'code' in error && typeof error.code === 'string') {
    return { code: error.code, params: 'params' in error && error.params && typeof error.params === 'object' ? error.params as Record<string, unknown> : {} };
  }
  return { code: 'setup.probe-failed', params: {} };
}
export function failureEnvironment(error: AppErrorVm): EnvironmentReport | undefined {
  const report = error.params.environment;
  if (report && typeof report === 'object' && 'agentDetectionReady' in report && report.agentDetectionReady === false && 'checks' in report && Array.isArray(report.checks)) return report as EnvironmentReport;
}

export function setupDetectionReducer(state: SetupDetectionState, action: Action): SetupDetectionState {
  switch (action.type) {
    case 'environment-started': return { ...state, phase: 'idle', generation: '', pending: {}, nextCursor: null, environment: { ...state.environment, phase: 'checking', progress: {} } };
    case 'environment-progress': return state.environment.phase === 'checking' ? { ...state, environment: { ...state.environment, progress: { ...state.environment.progress, [action.check.tool]: action.check } } } : state;
    case 'environment-completed': return { ...state, phase: action.report.agentDetectionReady ? 'idle' : 'blocked-environment', environment: { phase: 'ready', report: action.report, progress: {} } };
    case 'environment-failed': return { ...state, environment: { phase: 'failed', progress: {} } };
    case 'runtime-blocked': return { ...state, phase: 'blocked-environment', generation: '', pending: {}, nextCursor: null, environment: { phase: 'ready', report: action.report, progress: {} } };
    case 'detection-started': return { ...state, phase: 'detecting', generation: '', agents: action.append ? state.agents : {}, pending: {}, nextCursor: null };
    case 'detection-stopped': return { ...state, phase: 'idle', generation: '', pending: {} };
    case 'detection-accepted': return state.phase === 'detecting' || state.phase === 'completed' ? { ...state, generation: action.generation } : state;
    case 'detection-failed': return { ...state, phase: 'failed', generation: '', pending: {} };
    case 'model': return { ...state, agents: { ...state.agents, [action.agent.agentType]: action.agent } };
    case 'event': {
      const event = action.event;
      if ((state.phase !== 'detecting' && state.phase !== 'completed') || (state.generation && state.generation !== event.generation)) return state;
      const current = { ...state, generation: event.generation };
      if (event.type === 'failed') {
        const report = failureEnvironment(event);
        return setupDetectionReducer(current, report ? { type: 'runtime-blocked', report } : { type: 'detection-failed' });
      }
      if (event.type === 'checking') return { ...current, pending: { ...state.pending, [event.agentId]: event.label } };
      if (event.type === 'finished') return { ...current, phase: 'completed', nextCursor: event.nextCursor };
      const id = event.type === 'ready' ? event.agent.agentType : event.agentId;
      const pending = { ...state.pending }; delete pending[id];
      const agents = { ...state.agents };
      if (event.type === 'ready') agents[id] = event.agent;
      else delete agents[id];
      return { ...current, pending, agents };
    }
  }
}
