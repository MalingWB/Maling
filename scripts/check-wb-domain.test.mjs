import assert from 'node:assert/strict';
import test from 'node:test';
import { findDomainReferences } from './check-wb-domain.mjs';

test('domain scanner checks each file case-insensitively without exposing the value', () => {
  const files = [
    { path: 'bad.txt', content: Buffer.from('prefix EXAMPLE.INVALID:5005/path') },
    { path: 'safe.txt', content: Buffer.from('https://other.invalid/') },
  ];
  assert.deepEqual(findDomainReferences(files, 'example.invalid'), ['bad.txt']);
});
