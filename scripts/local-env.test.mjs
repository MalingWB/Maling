import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { LOCAL_ENV_FILENAME, loadLocalEnv } from './local-env.mjs';

function withTempRepo(envText, callback) {
  const root = mkdtempSync(join(tmpdir(), 'gold-band-local-env-'));
  try {
    if (envText !== undefined) {
      writeFileSync(join(root, LOCAL_ENV_FILENAME), envText);
    }
    return callback(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('loadLocalEnv reads WB origins from the repository .env', () => {
  withTempRepo(
    'GOLD_BAND_WB_BASE_URL=https://example.invalid\nGOLD_BAND_WB_MULTICA_URL=https://multica.example.invalid\nGOLD_BAND_WB_INTERNAL_DOMAIN=corp.invalid\n',
    (root) => {
      const env = loadLocalEnv({}, root);

      assert.equal(env.GOLD_BAND_WB_BASE_URL, 'https://example.invalid');
      assert.equal(env.GOLD_BAND_WB_MULTICA_URL, 'https://multica.example.invalid');
      assert.equal(env.GOLD_BAND_WB_INTERNAL_DOMAIN, 'corp.invalid');
    },
  );
});

test('loadLocalEnv keeps already-exported values so shell overrides and CI secrets win', () => {
  withTempRepo(
    'GOLD_BAND_WB_BASE_URL=https://from-file.invalid\nGOLD_BAND_METRICS_API_KEY=file-key\n',
    (root) => {
      const env = loadLocalEnv(
        { GOLD_BAND_WB_BASE_URL: 'https://from-shell.invalid' },
        root,
      );

      assert.equal(env.GOLD_BAND_WB_BASE_URL, 'https://from-shell.invalid');
      assert.equal(env.GOLD_BAND_METRICS_API_KEY, 'file-key');
    },
  );
});

test('loadLocalEnv silently skips a missing .env so default channel dev works unconfigured', () => {
  withTempRepo(undefined, (root) => {
    const env = loadLocalEnv({ KEPT: 'value' }, root);

    assert.deepEqual(env, { KEPT: 'value' });
  });
});

test('loadLocalEnv does not mutate keys absent from the file into undefined values', () => {
  withTempRepo('ONLY_ONE=value\n', (root) => {
    const env = loadLocalEnv({}, root);

    assert.deepEqual(env, { ONLY_ONE: 'value' });
    assert.equal(Object.hasOwn(env, 'GOLD_BAND_WB_BASE_URL'), false);
  });
});
