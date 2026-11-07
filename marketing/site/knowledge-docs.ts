import type { SiteRoute } from './content';
import { GITHUB, pageHref, type Language } from './content';

const RAW_DOCS = import.meta.glob<string>('../../docs/knowledge-base/**/*.md', { query: '?raw', import: 'default' });
const SOURCE_MARKER = 'docs/knowledge-base/';

export type KnowledgeSectionId = '' | 'product' | 'practice' | 'reference';

export interface KnowledgeDocSummary {
  id: string;
  sourcePath: string;
  sectionId: KnowledgeSectionId;
  title: string;
}

function sourcePathFromKey(key: string): string {
  const normalizedKey = key.replaceAll('\\', '/');
  const marker = normalizedKey.indexOf(SOURCE_MARKER);
  if (marker < 0) throw { code: 'docs.source-key-invalid', params: { key: normalizedKey } };
  return normalizedKey.slice(marker + SOURCE_MARKER.length);
}

function knowledgeId(sourcePath: string): string {
  if (sourcePath.toLowerCase() === 'readme.md') return '';
  return sourcePath.replace(/\.md$/i, '').replaceAll('\\', '/');
}

function knowledgeTitle(sourcePath: string): string {
  const filename = sourcePath.split(/[\\/]/).at(-1) ?? '';
  return filename.replace(/\.md$/i, '').replace(/^\d+-/, '').replace(/-/g, ' ');
}

function sectionId(sourcePath: string): KnowledgeSectionId {
  const folder = sourcePath.split(/[\\/]/)[0] ?? '';
  if (folder.toLowerCase() === '01-product') return 'product';
  if (folder.toLowerCase() === '02-practice') return 'practice';
  if (folder.toLowerCase() === '03-reference') return 'reference';
  return '';
}

export const knowledgeDocs: KnowledgeDocSummary[] = Object.keys(RAW_DOCS)
  .map(key => {
    const sourcePath = sourcePathFromKey(key);
    return { id: knowledgeId(sourcePath), sourcePath, sectionId: sectionId(sourcePath), title: knowledgeTitle(sourcePath) };
  })
  .sort((left, right) => {
    if (!left.id) return -1;
    if (!right.id) return 1;
    return left.id.localeCompare(right.id, undefined, { numeric: true });
  });

const loadersById = new Map<string, () => Promise<string>>(
  Object.entries(RAW_DOCS).map(([key, loader]) => {
    const sourcePath = sourcePathFromKey(key);
    return [knowledgeId(sourcePath), loader as () => Promise<string>];
  }),
);

export function normalizeKnowledgeDocId(id: string | null | undefined): string {
  const normalized = (id ?? '').replaceAll('\\', '/').replace(/^\/+/, '').replace(/\.md$/i, '');
  return normalized.toLowerCase() === 'readme' ? '' : normalized;
}

export function findKnowledgeDoc(id: string | null | undefined): KnowledgeDocSummary | null {
  const normalized = normalizeKnowledgeDocId(id);
  return knowledgeDocs.find(doc => doc.id === normalized) ?? null;
}

/** Prefix repo-relative Markdown links so rrweb-style hardening keeps treating them as safe relative URLs. */
function normalizeKnowledgeMarkdown(content: string): string {
  return content.replace(/\]\((?!(?:https?:|mailto:|#|\/|\.\.\/|\.\/)[^)\s]*)([^)\s]+)/g, '](./$1');
}
export async function loadKnowledgeDoc(id: string | null | undefined): Promise<string | null> {
  const normalized = normalizeKnowledgeDocId(id);
  const loader = loadersById.get(normalized);
  if (!loader) return null;
  return normalizeKnowledgeMarkdown(await loader());
}

/** Rewrite knowledge-base links into stable site routes before Markdown hardening. */
export function prepareKnowledgeMarkdown(
  content: string,
  currentId: string | null | undefined,
  language: Language,
  base = '/',
): string {
  return content.replace(/\]\(([^)\s]+)([^)]*)\)/g, (match, url: string, suffix = '') => {
    if (/^(?:https?:|mailto:|#)/i.test(url)) return `](${url}${suffix})`;
    const resolved = resolveKnowledgeHref(url, currentId, language, base);
    if (!resolved) return match;
    return `](${resolved}${suffix})`;
  });
}
/** Resolve a Markdown link to the site route, a repository link, or reject unsafe transports. */
export function resolveKnowledgeHref(
  url: string,
  currentId: string | null | undefined,
  language: Language,
  base = '/',
): string | null {
  if (!url) return null;
  if (/^(?:https?:|mailto:|#)/i.test(url)) return url;
  if (/^(?:javascript|data|file):/i.test(url)) return null;

  const currentPath = normalizeKnowledgeDocId(currentId) ? `${normalizeKnowledgeDocId(currentId)}.md` : 'README.md';
  let resolved: URL;
  try { resolved = new URL(url, `file:///repo/docs/knowledge-base/${currentPath}`); }
  catch { return null; }

  const pathname = decodeURIComponent(resolved.pathname);
  if (!pathname.startsWith('/repo/')) return null;
  const repositoryPath = pathname.slice('/repo/'.length).replaceAll('\\', '/');
  const suffix = `${resolved.search}${resolved.hash}`;

  if (/\.md$/i.test(repositoryPath) && repositoryPath.toLowerCase().startsWith('docs/knowledge-base/')) {
    const id = knowledgeId(repositoryPath.slice('docs/knowledge-base/'.length));
    return `${pageHref(language, 'documentation', base, id)}${suffix}`;
  }

  const encodedPath = repositoryPath.split('/').map(segment => encodeURIComponent(segment)).join('/');
  return `${GITHUB}/blob/main/${encodedPath}${suffix}`;
}