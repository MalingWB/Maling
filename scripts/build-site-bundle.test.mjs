import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { clearDirectoryContents } from './site-output.mjs';

const bundleScript = resolve(import.meta.dirname, 'build-site-bundle.mjs');

test('clears output children while retaining a locked output root', () => {
  const output = mkdtempSync(join(tmpdir(), 'website-output-'));
  try {
    mkdirSync(join(output, 'assets'), { recursive: true });
    writeFileSync(join(output, 'index.html'), '<!doctype html>');
    writeFileSync(join(output, 'assets/site.js'), 'export {}');

    clearDirectoryContents(output);

    assert.ok(existsSync(output));
    assert.equal(readdirSync(output, { withFileTypes: true }).length, 0);
  } finally {
    rmSync(output, { recursive: true, force: true });
  }
});

test('treats a missing output root as already clear', () => {
  const output = join(mkdtempSync(join(tmpdir(), 'website-missing-')), 'missing');
  try {
    assert.doesNotThrow(() => clearDirectoryContents(output));
  } finally {
    rmSync(output, { recursive: true, force: true });
  }
});

for (const failure of ['site', 'demo', null]) {
  test(failure ? `retains the previous delivery if the ${failure} build fails` : 'replaces stale delivery files after both builds succeed', t => {
    const root = mkdtempSync(join(tmpdir(), 'website-bundle-'));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const output = join(root, 'delivery');
    mkdirSync(output);
    writeFileSync(join(output, 'last-good.txt'), 'last successful bundle');
    writeFileSync(join(root, 'package.json'), JSON.stringify({
      private: true, scripts: { 'site:build': 'node fixture.cjs site', 'demo:build': 'node fixture.cjs demo' },
    }));
    writeFileSync(join(root, 'fixture.cjs'), `
      const { mkdirSync, writeFileSync } = require('node:fs');
      const { join } = require('node:path');
      const stage = process.argv[2];
      if (stage === process.env.FIXTURE_FAIL) { console.error('fixture failed: ' + stage); process.exit(1); }
      const dist = stage === 'site' ? process.env.SITE_DIST : process.env.DEMO_DIST;
      mkdirSync(dist, { recursive: true });
      writeFileSync(join(dist, 'index.html'), stage);
    `);
    const result = spawnSync(process.execPath, [bundleScript], {
      cwd: root, env: { ...process.env, WEBSITE_DIST: output, FIXTURE_FAIL: failure ?? '' }, encoding: 'utf8',
    });
    assert.ifError(result.error);
    if (failure) {
      assert.notEqual(result.status, 0);
      assert.ok(result.stderr.includes(`fixture failed: ${failure}`), result.stderr);
      assert.deepEqual(readdirSync(output), ['last-good.txt']);
      assert.equal(readFileSync(join(output, 'last-good.txt'), 'utf8'), 'last successful bundle');
    } else {
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.ok(!existsSync(join(output, 'last-good.txt')));
      assert.equal(readFileSync(join(output, 'index.html'), 'utf8'), 'site');
      assert.equal(readFileSync(join(output, 'demo', 'index.html'), 'utf8'), 'demo');
    }
  });
}
