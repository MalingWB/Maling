import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { normalizePath, type Plugin } from 'vite';

export const RELEASE_NOTES_MODULE_ID = 'virtual:gold-band-release-notes';
const RESOLVED_RELEASE_NOTES_MODULE_ID = `\0${RELEASE_NOTES_MODULE_ID}`;
const RELEASE_NOTES_LOCALES = ['zh-CN', 'zh-TW', 'en', 'ja-JP', 'ko-KR', 'pt-BR', 'es'] as const;
const RELEASE_NOTES_IMAGE_PATTERN = /!\[[^\r\n]*\]\(\s*([^)\s]+)(?:\s+"[^"]*")?\s*\)/g;

/**
 * Embeds `release-notes/<package.json version>/<locale>.md` for the version being built, one lazy
 * chunk per locale and bundles referenced `assets/` images, so the app can show its own release
 * notes offline without loading them at startup.
 * Every channel uses the release notes of its base version.
 */
export function releaseNotesModuleSource(repoRoot: string) {
  const { version } = JSON.parse(readFileSync(path.join(repoRoot, 'package.json'), 'utf8')) as { version: string };
  const versionDir = path.join(repoRoot, 'release-notes', version);
  const assets = new Map<string, string>();
  const imports: string[] = [];
  const loaders = RELEASE_NOTES_LOCALES
    .map((locale) => ({ locale, file: path.join(versionDir, `${locale}.md`) }))
    .filter(({ file }) => existsSync(file))
    .map(({ locale, file }) => releaseNotesLoaderSource(locale, file, versionDir, assets));
  for (const [index, [importPath, variableName]] of [...assets.entries()].entries()) {
    imports.push(`import ${variableName} from ${JSON.stringify(`${importPath}?url`)};`);
  }
  return [
    ...imports,
    `export const releaseNotesVersion = ${JSON.stringify(version)};`,
    'export const releaseNotesLoaders = {',
    ...loaders,
    '};',
    '',
  ].join('\n');
}

function releaseNotesLoaderSource(locale: string, file: string, versionDir: string, assets: Map<string, string>) {
  const rawPath = `${normalizePath(file)}?raw`;
  const content = readFileSync(file, 'utf8');
  const references = releaseNotesAssetReferences(content, versionDir);
  if (references.length === 0) {
    return `  ${JSON.stringify(locale)}: () => import(${JSON.stringify(rawPath)}).then((module) => module.default),`;
  }

  const replacements = references.map((reference) => {
    const resolved = path.resolve(versionDir, reference);
    const relative = path.relative(path.join(versionDir, 'assets'), resolved);
    if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) {
      throw new Error(`Release note image must be inside the version assets directory: ${reference}`);
    }
    const importPath = normalizePath(resolved);
    if (!existsSync(resolved)) {
      throw new Error(`Release note image not found: ${importPath}`);
    }
    if (!assets.has(importPath)) assets.set(importPath, `__releaseNotesAsset${assets.size}`);
    return `      .split(${JSON.stringify(reference)}).join(new URL(${assets.get(importPath)}, document.baseURI).href)`;
  });

  return [
    `  ${JSON.stringify(locale)}: async () => {`,
    `    const module = await import(${JSON.stringify(rawPath)});`,
    '    return module.default',
    ...replacements,
    '      ;',
    '  },',
  ].join('\n');
}

function releaseNotesAssetReferences(content: string, versionDir: string) {
  const references: string[] = [];
  for (const [, reference] of content.matchAll(RELEASE_NOTES_IMAGE_PATTERN)) {
    const resolved = path.resolve(versionDir, decodeURIComponent(reference));
    const relative = path.relative(path.join(versionDir, 'assets'), resolved);
    if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) continue;
    if (!references.includes(reference)) references.push(reference);
  }
  return references;
}

export function releaseNotesPlugin(repoRoot: string): Plugin {
  return {
    name: 'gold-band-release-notes',
    resolveId(id) {
      return id === RELEASE_NOTES_MODULE_ID ? RESOLVED_RELEASE_NOTES_MODULE_ID : undefined;
    },
    load(id) {
      if (id !== RESOLVED_RELEASE_NOTES_MODULE_ID) return undefined;
      this.addWatchFile(path.join(repoRoot, 'package.json'));
      return releaseNotesModuleSource(repoRoot);
    },
  };
}
