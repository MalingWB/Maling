import type { AcpModeVm, AcpSelectConfigOptionVm, AppErrorVm, AppearancePreference, DesktopLanguage, DesktopPlatform, WorkerModelBinding } from '@/types';

export interface DefaultExecutionProfile {
  agentId: string;
  modelSelection: { type: 'explicit'; modelId: string } | { type: 'provider-default' };
  configOptions: Record<string, string>;
  modelBoundOverrides: Record<string, Record<string, string>>;
}
export interface SetupStatus {
  platform: DesktopPlatform;
  state: { status: 'pending'; gitDecision: 'undecided' | 'skip' } | { status: 'completed' | 'existing-install' };
  gitMinimumVersion: string;
  environmentPolicy: { node: { ranges: string[]; recommendedVersion: string; helpUrl: string }; npm: { ranges: string[] }; gitHelpUrl: string };
  defaultExecutionProfile: DefaultExecutionProfile | null;
  language: DesktopLanguage;
  appearance: AppearancePreference;
}
export interface EnvironmentReport {
  runtimeReady: boolean;
  checks: Array<{ tool: 'git' | 'node' | 'npm' | 'npx'; status: 'ready' | 'missing' | 'unsupported' | 'unverified' | 'incompatible' | 'failed' | 'timeout' | 'cancelled'; version: string | null; executablePath: string | null; nodeExecutablePath?: string | null; error?: AppErrorVm | null }>;
}
export type EnvironmentCheck = EnvironmentReport['checks'][number];
export interface SetupAgent {
  agentType: string;
  displayName: string;
  iconKey?: string;
  supportedModels?: AcpModeVm[] | null;
  supportedModes?: AcpModeVm[] | null;
  configOptions?: AcpSelectConfigOptionVm[] | null;
  modelBoundCatalogs?: Record<string, AcpSelectConfigOptionVm[]> | null;
}
export type SetupEvent =
  | { type: 'checking'; generation: string; agentId: string; label: string }
  | { type: 'ready'; generation: string; agent: SetupAgent }
  | { type: 'unavailable'; generation: string; agentId: string }
  | { type: 'finished'; generation: string; nextCursor: string | null }
  | ({ type: 'failed'; generation: string } & AppErrorVm);
export interface SetupSelection { profile: DefaultExecutionProfile; permissionModeId: string | null }
export interface SetupPreview { fingerprint: string; slotCount: number; templates: Array<{ id: string; name: string; slotCount: number; previousBindings: WorkerModelBinding[] }> }
export interface SetupApply { selection: SetupSelection; previewFingerprint: string; operationId: string; gitSkipped: boolean }
export interface SetupApi {
  status(): Promise<SetupStatus>;
  environment(requestId: string, onChecked?: (check: EnvironmentCheck) => void): Promise<EnvironmentReport>;
  cancelEnvironment(requestId: string): Promise<void>;
  skipGit(): Promise<void>;
  start(onEvent: (event: SetupEvent) => void, after?: string): Promise<string>;
  cancel(generation: string): Promise<void>;
  probeModel(generation: string, agentId: string, modelId: string): Promise<SetupAgent>;
  preview(selection: SetupSelection): Promise<SetupPreview>;
  apply(request: SetupApply): Promise<void>;
  preferences(language: DesktopLanguage, appearance: AppearancePreference): Promise<void>;
}
