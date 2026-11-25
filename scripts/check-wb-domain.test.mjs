import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { findDomainReferences } from './check-wb-domain.mjs';

test('domain scanner checks each file case-insensitively without exposing the value', () => {
  const files = [
    { path: 'bad.txt', content: Buffer.from('prefix EXAMPLE.INVALID:5005/path') },
    { path: 'internal.md', content: Buffer.from('http://service1.CORP.invalid:8001/path') },
    { path: 'safe.txt', content: Buffer.from('https://other.invalid/') },
  ];
  assert.deepEqual(findDomainReferences(files, ['example.invalid', 'corp.invalid']), ['bad.txt', 'internal.md']);
});

test('release scan rejects a service subdomain even when both origins have unrelated hosts', (t) => {
  const cwd = mkdtempSync(join(tmpdir(), 'wb-scan-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const init = spawnSync('git', ['init', '-q'], { cwd });
  assert.equal(init.status, 0);
  writeFileSync(join(cwd, 'profile.md'), 'http://wnpm.engineering.example:8001');
  const scan = spawnSync(process.execPath, [fileURLToPath(new URL('./check-wb-domain.mjs', import.meta.url))], {
    cwd, encoding: 'utf8', env: { ...process.env, GOLD_BAND_WB_BASE_URL: 'https://portal.invalid',
      GOLD_BAND_WB_MULTICA_URL: 'https://multica.invalid', GOLD_BAND_WB_INTERNAL_DOMAIN: 'Engineering.Example' },
  });
  assert.equal(scan.status, 1);
  assert.match(scan.stderr, /profile\.md/);
  assert.doesNotMatch(scan.stderr, /engineering\.example/i);
});
