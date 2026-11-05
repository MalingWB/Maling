import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { releaseNotesModuleSource } from '../config/release-notes-plugin';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function repo(version: string, locales: string[]) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'gold-band-release-notes-'));
  roots.push(root);
  writeFileSync(path.join(root, 'package.json'), JSON.stringify({ version }));
  if (locales.length) {
    const dir = path.join(root, 'release-notes', version);
    mkdirSync(dir, { recursive: true });
    for (const locale of locales) writeFileSync(path.join(dir, `${locale}.md`), `# ${locale}`);
  }
  return root;
}

describe('release notes build module', () => {
  it('embeds only the package version notes as one lazy raw chunk per existing locale', () => {
    const root = repo('1.2.3', ['zh-CN', 'en']);
    mkdirSync(path.join(root, 'release-notes', '1.2.2'), { recursive: true });
    writeFileSync(path.join(root, 'release-notes', '1.2.2', 'ja-JP.md'), '# old');
    const source = releaseNotesModuleSource(root);
    expect(source).toContain('export const releaseNotesVersion = "1.2.3";');
    const imports = [...source.matchAll(/"([^"]+)": \(\) => import\("([^"]+)"\)/g)].map(([, locale, file]) => [locale, file]);
    expect(imports.map(([locale]) => locale)).toEqual(['zh-CN', 'en']);
    for (const [locale, file] of imports) {
      expect(file.endsWith(`/release-notes/1.2.3/${locale}.md?raw`)).toBe(true);
      expect(file).not.toContain('\\');
    }
    expect(source).not.toContain('1.2.2');
  });

  it('exposes no loaders when the version has no release notes', () => {
    const source = releaseNotesModuleSource(repo('9.9.9', []));
    expect(source).toContain('export const releaseNotesVersion = "9.9.9";');
    expect(source).not.toContain('import(');
  });

  it('bundles release note assets and rewrites their Markdown references', () => {
    const root = repo('1.2.3', ['zh-CN', 'en']);
    const assetDirectory = path.join(root, 'release-notes', '1.2.3', 'assets');
    mkdirSync(assetDirectory, { recursive: true });
    writeFileSync(path.join(assetDirectory, 'first.png'), 'first');
    writeFileSync(path.join(assetDirectory, 'second.jpg'), 'second');
    writeFileSync(
      path.join(root, 'release-notes', '1.2.3', 'zh-CN.md'),
      [
        '![First](assets/first.png)',
        '![Second](assets/second.jpg "Second")',
        '![Remote](https://example.invalid/remote.png)',
      ].join('\n'),
    );

    const source = releaseNotesModuleSource(root);
    const imports = [...source.matchAll(/^import (__releaseNotesAsset\d+) from "([^"]+)\?url";$/gm)]
      .map(([, variable, file]) => [variable, file]);
    expect(imports).toEqual([
      ['__releaseNotesAsset0', expect.stringMatching(/release-notes[\\/]1\.2\.3[\\/]assets[\\/]first\.png$/)],
      ['__releaseNotesAsset1', expect.stringMatching(/release-notes[\\/]1\.2\.3[\\/]assets[\\/]second\.jpg$/)],
    ]);
    expect(source).toContain('.split("assets/first.png").join(new URL(__releaseNotesAsset0, document.baseURI).href)');
    expect(source).toContain('.split("assets/second.jpg").join(new URL(__releaseNotesAsset1, document.baseURI).href)');
    expect(source).not.toContain('remote.png?url');
  });

  it('rejects a referenced release note asset that is missing', () => {
    const root = repo('1.2.3', ['zh-CN']);
    writeFileSync(path.join(root, 'release-notes', '1.2.3', 'zh-CN.md'), '![Missing](assets/missing.png)');
    expect(() => releaseNotesModuleSource(root)).toThrow('Release note image not found');
  });
});
