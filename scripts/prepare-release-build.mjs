import { appendFileSync } from 'node:fs';

import { assertChannelBuildSecrets } from './build-channel-options.mjs';
import { readChannelConfig, writeTauriConfigOverlay } from './channel-config.mjs';

// Both GitHub release workflows use the channel config for product identity,
// and the release tag (not a local timestamp) for the executable version.
try {
  const channel = process.env.GOLD_BAND_RELEASE_CHANNEL;
  const tag = process.env.RELEASE_TAG;
  const outputPath = process.argv[2];
  if (!channel || !outputPath) {
    throw new Error('GOLD_BAND_RELEASE_CHANNEL and a Tauri overlay output path are required.');
  }
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag ?? '')) {
    throw new Error('RELEASE_TAG must be a semver tag such as v0.20.0.');
  }

  const config = readChannelConfig(channel);
  assertChannelBuildSecrets(channel, config, process.env);
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY?.trim()) {
    throw new Error(`Missing TAURI_SIGNING_PRIVATE_KEY for signed ${channel} release builds.`);
  }

  writeTauriConfigOverlay(config, outputPath, tag.slice(1));
  const releaseName = `${config.productName} ${tag}`;
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `release_name=${releaseName}\n`);
  }
  console.log(`Prepared ${releaseName} (${channel}).`);
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
