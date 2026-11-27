import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

describe('conversation task deletion identity wiring', () => {
  it('commits a canonical page only when its identity actually changes', () => {
    const source = fs.readFileSync(path.resolve(process.cwd(), 'web/src/App.tsx'), 'utf8');
    const navigationCommit = source.match(
      /const currentPage = conversationPageRef\.current;[\s\S]*?const latestFollowState/u,
    )?.[0] ?? '';

    expect(navigationCommit).toContain(
      'canonicalizeConversationPageIdentity(currentPage, run.taskUuid)',
    );
    expect(navigationCommit).toMatch(
      /if \(canonicalPage !== currentPage\) \{\s*setConversationPage\(canonicalPage\);\s*\}/u,
    );
  });

});
