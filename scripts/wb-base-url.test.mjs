import assert from 'node:assert/strict';
import test from 'node:test';
import { readChannelConfig, readInternalDomain } from './channel-config.mjs';

const WB_BASE_URL = 'https://example.invalid/';
const WB_MULTICA_URL = 'https://multica.example.invalid/';

test('WB channel resolves its endpoints from injected origins', () => {
  const config = readChannelConfig('wb', {
    GOLD_BAND_WB_BASE_URL: WB_BASE_URL,
    GOLD_BAND_WB_MULTICA_URL: WB_MULTICA_URL,
    GOLD_BAND_WB_INTERNAL_DOMAIN: 'corp.invalid',
  });
  assert.equal(config.updaterEndpoint, 'https://example.invalid/api/file/download/latest.json');
  assert.equal(config.releaseBaseUrl, 'https://example.invalid/api/file/download');
  assert.equal(config.metricsBaseUrl, 'https://example.invalid');
  assert.equal(config.multicaBaseUrl, 'https://multica.example.invalid');
  assert.equal(config.multicaAppUrl, 'https://multica.example.invalid');
  assert.equal(config.builtinMcpServers[0].transport.url, 'https://example.invalid/mcp/');
  assert.match(config.builtinMcpServers[0].helpMessage, /https:\/\/example\.invalid/);
});

test('WB configuration rejects absent or invalid service origins without disclosing them', () => {
  for (const value of ['', '  ', 'ftp://example.invalid', 'https://example.invalid/path', 'https://user:pass@example.invalid', 'https://@example.invalid', 'https://example.invalid/a/..', 'https://example.invalid/?secret=1']) {
    assert.throws(
      () => readChannelConfig('wb', { GOLD_BAND_WB_BASE_URL: value, GOLD_BAND_WB_MULTICA_URL: WB_MULTICA_URL }),
      /GOLD_BAND_WB_BASE_URL/,
    );
  }
  assert.doesNotThrow(() => readChannelConfig('default', {}));
});

test('WB configuration rejects absent or invalid multica origins without disclosing them', () => {
  for (const value of ['', '  ', 'ftp://example.invalid', 'https://example.invalid/path', 'https://user:pass@example.invalid', 'https://@example.invalid', 'https://example.invalid/a/..', 'https://example.invalid/?secret=1']) {
    assert.throws(
      () => readChannelConfig('wb', { GOLD_BAND_WB_BASE_URL: WB_BASE_URL, GOLD_BAND_WB_MULTICA_URL: value }),
      /GOLD_BAND_WB_MULTICA_URL/,
    );
  }
});

test('WB source configuration references the public environment variable names', async () => {
  const { readFileSync } = await import('node:fs');
  const { repoRoot } = await import('./channel-config.mjs');
  const { join } = await import('node:path');
  const config = JSON.parse(readFileSync(join(repoRoot, 'configs/channels/wb.json'), 'utf8'));
  const bookmarks = JSON.parse(readFileSync(join(repoRoot, 'configs/browser-bookmarks.json'), 'utf8'));
  assert.equal(config.metricsBaseUrl, '${GOLD_BAND_WB_BASE_URL}');
  assert.equal(config.multicaBaseUrl, '${GOLD_BAND_WB_MULTICA_URL}');
  assert.equal(config.multicaAppUrl, '${GOLD_BAND_WB_MULTICA_URL}');
  assert.equal(config.multicaPort, undefined);
  assert.equal(bookmarks.wb.find((item) => item.id === 'maling').url, '${GOLD_BAND_WB_BASE_URL}/');
});

test('internal domain is a bare DNS suffix, not an origin, IP, port, path or command', () => {
  assert.equal(readInternalDomain({ GOLD_BAND_WB_INTERNAL_DOMAIN: 'Rd.Corp-2.Example' }), 'rd.corp-2.example');
  for (const value of [undefined, '', 'corp', '127.0.0.1', 'https://corp.invalid', 'corp.invalid:80', 'user@corp.invalid',
    'corp.invalid/path', 'corp.invalid#x', 'corp..invalid', 'corp.invalid.', '-bad.invalid', 'bad-.invalid',
    'corp.invalid\n', 'corp.invalid;whoami', `${'a'.repeat(64)}.invalid`]) {
    assert.throws(() => readChannelConfig('wb', { GOLD_BAND_WB_BASE_URL: WB_BASE_URL, GOLD_BAND_WB_MULTICA_URL: WB_MULTICA_URL,
      GOLD_BAND_WB_INTERNAL_DOMAIN: value }), { message: 'Missing or invalid GOLD_BAND_WB_INTERNAL_DOMAIN: expected a bare DNS domain suffix.' });
  }
});
