export const SUPPORT_DEVTOOLS_FEATURE = 'support-devtools';
export const GOLD_BAND_METRICS_API_KEY = 'GOLD_BAND_METRICS_API_KEY';

export function parseChannelBuildArgs(args) {
  const [channel = 'default', ...rawOptions] = args;
  const options = {
    channel,
    isCritical: false,
    devtools: false,
  };

  for (const option of rawOptions) {
    if (option === 'critical' || option === '--critical') {
      options.isCritical = true;
      continue;
    }
    if (option === '--devtools') {
      options.devtools = true;
      continue;
    }
    throw new Error(`Unsupported build option: ${option}`);
  }

  return options;
}

export function channelBuildPlan(overlayPath, { channel = 'default', devtools = false } = {}, platform = process.platform) {
  const tauriArgs = ['tauri', 'build', '--config', overlayPath];
  // Local channel versions contain a timestamp and Git SHA, which WiX cannot
  // represent. Official GitHub releases use tag versions and native targets.
  if (channel !== 'default' && platform === 'win32') {
    tauriArgs.push('--bundles', 'nsis');
  }
  if (devtools) {
    tauriArgs.push('--features', SUPPORT_DEVTOOLS_FEATURE);
  }
  return {
    tauriArgs,
    tauriConfigBuildOptions: devtools ? { createUpdaterArtifacts: false } : undefined,
    shouldCollectReleaseArtifacts: !devtools,
  };
}

export function assertChannelBuildSecrets(channel, config, env) {
  if (!config.metricsEnabled) {
    return;
  }

  const metricsApiKey = env[GOLD_BAND_METRICS_API_KEY];
  if (typeof metricsApiKey !== 'string' || metricsApiKey.trim() === '') {
    throw new Error(
      `Missing required build secret ${GOLD_BAND_METRICS_API_KEY} for channel ${channel} because metrics are enabled.`,
    );
  }
}

export function updaterGeneratorArgs(releaseDir, outputPath, baseUrl, version, critical) {
  return ['scripts/generate-updater-json.mjs', releaseDir, outputPath, '--base-url', baseUrl, '--version', version,
    ...(critical ? ['--critical'] : [])];
}
