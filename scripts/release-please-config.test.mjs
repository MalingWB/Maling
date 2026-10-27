import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptsDir, '..');

test('release-please declares the pre-1.0 breaking-change policy', async () => {
  const rawConfig = await readFile(
    path.join(repoRoot, 'release-please-config.json'),
    'utf8',
  );
  const config = JSON.parse(rawConfig);

  assert.equal(config.packages?.['.']?.['bump-minor-pre-major'], true);
});

for (const [file, toolingRoot] of [
  ['release-please.yml', ''],
  ['release.yml', 'release-tools/'],
]) {
  test(`${file} passes the WB base URL to the release preflight Cargo check`, async () => {
    const workflow = parse(await readFile(path.join(repoRoot, '.github/workflows', file), 'utf8'));
    const preflight = workflow.jobs['release-preflight'];
    const check = preflight.steps.find((step) => step.run === 'cargo check --workspace --all-targets');
    assert.ok(check, 'release preflight must check the Rust workspace');
    const env = { ...workflow.env, ...preflight.env, ...check.env };
    assert.equal(env.GOLD_BAND_RELEASE_CHANNEL, 'wb');
    assert.equal(env.GOLD_BAND_WB_BASE_URL, '${{ secrets.GOLD_BAND_WB_BASE_URL }}');
  });

  test(`${file} builds signed WB packages for all four release platforms`, async () => {
    const workflow = parse(await readFile(path.join(repoRoot, '.github/workflows', file), 'utf8'));
    assert.equal(workflow.env.GOLD_BAND_RELEASE_CHANNEL, 'wb');
    const build = workflow.jobs.build;
    assert.deepEqual(build.strategy.matrix.platform, [
      { name: 'windows-x64', runner: 'windows-latest' },
      { name: 'linux-x64', runner: 'ubuntu-22.04' },
      { name: 'macos-x64', runner: 'macos-15-intel' },
      { name: 'macos-arm64', runner: 'macos-15' },
    ]);
    assert.equal(build.env.TAURI_SIGNING_PRIVATE_KEY, '${{ secrets.WB_TAURI_SIGNING_PRIVATE_KEY }}');
    assert.equal(build.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD, '${{ secrets.WB_TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}');
    assert.equal(build.env.GOLD_BAND_METRICS_API_KEY, '${{ secrets.GOLD_BAND_METRICS_API_KEY }}');
    assert.equal(build.env.GOLD_BAND_WB_BASE_URL, '${{ secrets.GOLD_BAND_WB_BASE_URL }}');
    const manifestStep = workflow.jobs['updater-json'].steps.find((step) => step.name === 'Generate localized updater manifests and release body');
    assert.equal(manifestStep.env.GOLD_BAND_WB_BASE_URL, '${{ secrets.GOLD_BAND_WB_BASE_URL }}');
    assert.ok(build.steps.some((step) => step.run === `node ${toolingRoot}scripts/check-wb-domain.mjs`));

    const prepareIndex = build.steps.findIndex((step) => step.id === 'channel');
    assert.ok(prepareIndex >= 0, 'release credentials and channel config must be prepared before building');
    assert.equal(build.steps[prepareIndex].run,
      `node ${toolingRoot}scripts/prepare-release-build.mjs src-tauri/target/channel/tauri.release.conf.json`);
    const installIndex = build.steps.findIndex((step) => step.run === 'npm ci');
    assert.ok(installIndex > prepareIndex, 'missing credentials must fail before installing dependencies');
    const actionIndex = build.steps.findIndex((step) => step.uses?.startsWith('tauri-apps/tauri-action@'));
    assert.ok(actionIndex > prepareIndex);
    const action = build.steps[actionIndex];
    assert.equal(action.with.args, '--config src-tauri/target/channel/tauri.release.conf.json');
    assert.equal(action.with.releaseName, '${{ steps.channel.outputs.release_name }}');
    assert.equal(action.with.releaseDraft, true);
    assert.equal(action.env?.GOLD_BAND_RELEASE_CHANNEL, undefined, 'the build must inherit the workflow channel');
    assert.equal(action.env?.TAURI_SIGNING_PRIVATE_KEY, undefined, 'the action must use the validated WB key');
  });
}
