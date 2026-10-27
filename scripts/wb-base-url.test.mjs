import assert from 'node:assert/strict';
import test from 'node:test';
import { readChannelConfig } from './channel-config.mjs';

test('WB channel resolves its endpoints from an injected origin', () => {
  const config = readChannelConfig('wb', { GOLD_BAND_WB_BASE_URL: 'https://example.invalid/' });
  assert.equal(config.updaterEndpoint, 'https://example.invalid/api/file/download/latest.json');
  assert.equal(config.releaseBaseUrl, 'https://example.invalid/api/file/download');
  assert.equal(config.metricsBaseUrl, 'https://example.invalid');
  assert.equal(config.multicaBaseUrl, 'https://example.invalid:5005');
  assert.equal(config.multicaAppUrl, 'https://example.invalid:5005');
  assert.equal(config.builtinMcpServers[0].transport.url, 'https://example.invalid/mcp/');
  assert.match(config.builtinMcpServers[0].helpMessage, /https:\/\/example\.invalid/);
});

test('WB configuration rejects absent or invalid origins without disclosing them', () => {
  for (const value of ['', '  ', 'ftp://example.invalid', 'https://example.invalid/path', 'https://user:pass@example.invalid', 'https://@example.invalid', 'https://example.invalid/a/..', 'https://example.invalid/?secret=1']) {
    assert.throws(() => readChannelConfig('wb', { GOLD_BAND_WB_BASE_URL: value }), /GOLD_BAND_WB_BASE_URL/);
  }
  assert.doesNotThrow(() => readChannelConfig('default', {}));
});

test('WB source configuration references the public environment variable name', async () => {
  const { readFileSync } = await import('node:fs');
  const { repoRoot } = await import('./channel-config.mjs');
  const { join } = await import('node:path');
  const config = JSON.parse(readFileSync(join(repoRoot, 'configs/channels/wb.json'), 'utf8'));
  const bookmarks = JSON.parse(readFileSync(join(repoRoot, 'configs/browser-bookmarks.json'), 'utf8'));
  assert.equal(config.metricsBaseUrl, '${GOLD_BAND_WB_BASE_URL}');
  assert.equal(bookmarks.wb.find((item) => item.id === 'maling').url, '${GOLD_BAND_WB_BASE_URL}/');
});
