import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { readChannelConfig } from './channel-config.mjs';

const script = path.resolve('scripts/prepare-release-build.mjs');

function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), 'wb-release-build-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const overlayPath = path.join(root, 'channel', 'tauri.release.conf.json');
  const githubOutput = path.join(root, 'github-output');
  const run = (overrides = {}) => spawnSync(process.execPath, [script, overlayPath], {
    env: {
      ...process.env,
      GOLD_BAND_RELEASE_CHANNEL: 'wb',
      RELEASE_TAG: 'v1.2.3',
      TAURI_SIGNING_PRIVATE_KEY: 'fixture-private-key',
      GOLD_BAND_METRICS_API_KEY: 'fixture-metrics-key',
      GITHUB_OUTPUT: githubOutput,
      ...overrides,
    },
    encoding: 'utf8',
  });
  return { overlayPath, githubOutput, run };
}

test('release preparation uses WB identity and exact tag version without exposing credentials', (t) => {
  const { overlayPath, githubOutput, run } = fixture(t);
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  const overlay = JSON.parse(readFileSync(overlayPath, 'utf8'));
  const wb = readChannelConfig('wb');
  assert.equal(overlay.version, '1.2.3');
  assert.equal(overlay.productName, wb.productName);
  assert.equal(overlay.identifier, wb.identifier);
  assert.equal(overlay.mainBinaryName, wb.mainBinaryName);
  assert.equal(overlay.bundle.targets, undefined, 'inherit native all-platform targets');
  assert.equal(overlay.plugins.updater.pubkey, wb.updaterPublicKey);
  assert.deepEqual(overlay.plugins.updater.endpoints, [wb.updaterEndpoint]);
  assert.equal(readFileSync(githubOutput, 'utf8'), 'release_name=MALING v1.2.3\n');
  assert.doesNotMatch(result.stdout + result.stderr + readFileSync(overlayPath, 'utf8'), /fixture-(private|metrics)-key/);
});

for (const [overrides, message] of [
  [{ TAURI_SIGNING_PRIVATE_KEY: '' }, /Missing TAURI_SIGNING_PRIVATE_KEY/],
  [{ TAURI_SIGNING_PRIVATE_KEY: '   ' }, /Missing TAURI_SIGNING_PRIVATE_KEY/],
  [{ GOLD_BAND_METRICS_API_KEY: '' }, /Missing required build secret GOLD_BAND_METRICS_API_KEY/],
  [{ GOLD_BAND_METRICS_API_KEY: '   ' }, /Missing required build secret GOLD_BAND_METRICS_API_KEY/],
  [{ GOLD_BAND_RELEASE_CHANNEL: '' }, /GOLD_BAND_RELEASE_CHANNEL/],
  [{ GOLD_BAND_RELEASE_CHANNEL: 'unknown' }, /Unsupported channel/],
  [{ RELEASE_TAG: 'main' }, /RELEASE_TAG must be a semver tag/],
  [{ RELEASE_TAG: '' }, /RELEASE_TAG must be a semver tag/],
]) {
  test(`release preparation rejects ${JSON.stringify(overrides)} before writing output`, (t) => {
    const { overlayPath, githubOutput, run } = fixture(t);
    const result = run(overrides);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, message);
    assert.equal(existsSync(overlayPath), false);
    assert.equal(existsSync(githubOutput), false);
  });
}
