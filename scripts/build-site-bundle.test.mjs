import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { clearDirectoryContents } from './site-output.mjs';

const script = readFileSync(resolve(import.meta.dirname, './build-site-bundle.mjs'), 'utf8');

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

test('build output preparation retains the root instead of deleting it', () => {
  assert.match(script, /clearDirectoryContents\(output\)/);
  assert.doesNotMatch(script, /rmSync\(output/);
});