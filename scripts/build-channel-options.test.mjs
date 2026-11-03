import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertChannelBuildSecrets,
  channelBuildPlan,
  GOLD_BAND_METRICS_API_KEY,
  parseChannelBuildArgs,
  SUPPORT_DEVTOOLS_FEATURE,
} from './build-channel-options.mjs';

test('channel build options keep the default release path unchanged', () => {
  assert.deepEqual(parseChannelBuildArgs(['default']), {
    channel: 'default',
    isCritical: false,
    devtools: false,
  });
  assert.deepEqual(channelBuildPlan('tauri.default.conf.json'), {
    tauriArgs: ['tauri', 'build', '--config', 'tauri.default.conf.json'],
    tauriConfigBuildOptions: undefined,
    shouldCollectReleaseArtifacts: true,
  });
});

test('channel build options enable release-profile support devtools independently from critical updates', () => {
  assert.deepEqual(parseChannelBuildArgs(['wb', '--critical', '--devtools']), {
    channel: 'wb',
    isCritical: true,
    devtools: true,
  });
  assert.deepEqual(channelBuildPlan('tauri.wb.conf.json', { devtools: true }), {
    tauriArgs: [
      'tauri',
      'build',
      '--config',
      'tauri.wb.conf.json',
      '--features',
      SUPPORT_DEVTOOLS_FEATURE,
    ],
    tauriConfigBuildOptions: { createUpdaterArtifacts: false },
    shouldCollectReleaseArtifacts: false,
  });
});

test('channel build options retain the legacy positional critical flag and reject typos', () => {
  assert.equal(parseChannelBuildArgs(['wb', 'critical']).isCritical, true);
  assert.throws(
    () => parseChannelBuildArgs(['wb', '--devtool']),
    /Unsupported build option: --devtool/,
  );
});

test('local timestamped WB builds use NSIS on Windows and native bundles elsewhere', () => {
  for (const devtools of [false, true]) {
    for (const platform of ['win32', 'darwin', 'linux']) {
      const plan = channelBuildPlan('tauri.wb.conf.json', { channel: 'wb', devtools }, platform);
      const bundlesIndex = plan.tauriArgs.indexOf('--bundles');
      if (platform === 'win32') {
        assert.ok(bundlesIndex >= 0, 'timestamped local versions cannot be packaged as MSI');
        assert.equal(plan.tauriArgs[bundlesIndex + 1], 'nsis');
      } else {
        assert.equal(bundlesIndex, -1, 'use native bundle targets outside Windows');
      }
      assert.equal(plan.shouldCollectReleaseArtifacts, !devtools);
    }
  }
});

test('metrics-enabled channel requires a non-empty metrics API key', () => {
  assert.doesNotThrow(() => assertChannelBuildSecrets(
    'wb',
    { metricsEnabled: true },
    { [GOLD_BAND_METRICS_API_KEY]: 'secret' },
  ));

  for (const invalid of [undefined, '', '   ']) {
    assert.throws(
      () => assertChannelBuildSecrets(
        'wb',
        { metricsEnabled: true },
        { [GOLD_BAND_METRICS_API_KEY]: invalid },
      ),
      /Missing required build secret GOLD_BAND_METRICS_API_KEY/,
    );
  }
});

test('metrics-disabled channel does not require a metrics API key', () => {
  assert.doesNotThrow(() => assertChannelBuildSecrets(
    'default',
    { metricsEnabled: false },
    {},
  ));
});


test('updater generator receives the URL as a single argument without shell interpolation', async () => {
  const { updaterGeneratorArgs } = await import('./build-channel-options.mjs');
  assert.deepEqual(updaterGeneratorArgs('release dir', 'latest.json', 'https://example.invalid/download', '1.2.3', true),
    ['scripts/generate-updater-json.mjs', 'release dir', 'latest.json', '--base-url', 'https://example.invalid/download', '--version', '1.2.3', '--critical']);
});
