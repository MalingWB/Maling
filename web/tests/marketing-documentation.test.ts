import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { GITHUB, pageHref, parseRoute } from '../../marketing/site/content';
import { knowledgeDocs, loadKnowledgeDoc, normalizeKnowledgeDocId, resolveKnowledgeHref } from '../../marketing/site/knowledge-docs';

describe('website documentation routes', () => {
  it('keeps documentation overview and document slugs in the route contract', () => {
    expect(parseRoute('/zh/documentation')).toEqual({ language: 'zh', page: 'documentation', docId: null });
    expect(parseRoute('/en/documentation')).toEqual({ language: 'en', page: 'documentation', docId: null });
    const id = '01-product/01-产品概览与定位';
    const href = pageHref('en', 'documentation', '/product/', id);
    expect(href).toBe('/product/en/documentation/01-product/01-%E4%BA%A7%E5%93%81%E6%A6%82%E8%A7%88%E4%B8%8E%E5%AE%9A%E4%BD%8D');
    expect(parseRoute(href.slice('/product/'.length - 1))).toEqual({ language: 'en', page: 'documentation', docId: id });
    expect(parseRoute('/zh/documentation/not-a-doc')).toEqual({ language: 'zh', page: 'documentation', docId: 'not-a-doc' });
  });

  it('derives a bounded knowledge manifest from canonical Markdown without hardcoding documents', () => {
    expect(knowledgeDocs).toHaveLength(11);
    expect(knowledgeDocs[0]).toMatchObject({ id: '', sectionId: '', sourcePath: 'README.md' });
    expect(new Set(knowledgeDocs.map(doc => doc.id)).size).toBe(knowledgeDocs.length);
    for (const doc of knowledgeDocs) {
      expect(doc.title).not.toContain('.md');
      expect(doc.sourcePath).not.toMatch(/^([\\/])/);
      const sourcePath = resolve('docs/knowledge-base', doc.sourcePath);
      expect(existsSync(sourcePath)).toBe(true);
      expect(readFileSync(sourcePath, 'utf8')).toMatch(/^#/m);
    }
  });

  it('loads only the selected Markdown body', async () => {
    await expect(loadKnowledgeDoc(null)).resolves.toContain('# MALING 客户端知识库');
    await expect(loadKnowledgeDoc(null)).resolves.toContain('](./01-product/01-产品概览与定位.md)');
    const id = '02-practice/01-快速上手';
    await expect(loadKnowledgeDoc(id)).resolves.toContain('# 快速上手');
    await expect(loadKnowledgeDoc('not-a-doc')).resolves.toBeNull();
    expect(normalizeKnowledgeDocId('/README.md')).toBe('');
  });

  it('converts knowledge links to routes and keeps external engineering links in the repository', () => {
    expect(resolveKnowledgeHref('01-product/01-产品概览与定位.md', null, 'zh')).toBe(pageHref('zh', 'documentation', '/', '01-product/01-产品概览与定位'));
    const hashHref = pageHref('en', 'documentation', '/product/', '02-practice/02-运行模式指南');
    const resolvedHashHref = resolveKnowledgeHref('../02-practice/02-运行模式指南.md#5-选型速查', '01-product/01-产品概览与定位', 'en', '/product/');
    expect(resolvedHashHref?.startsWith(hashHref + '#')).toBe(true);
    expect(resolveKnowledgeHref('javascript:alert(1)', null, 'zh')).toBeNull();
    expect(resolveKnowledgeHref('https://example.com/docs', null, 'zh')).toBe('https://example.com/docs');
  });

  it('can resolve every Markdown document link in the shipped knowledge base', async () => {
    for (const doc of knowledgeDocs) {
      const content = await loadKnowledgeDoc(doc.id);
      expect(content).toBeTruthy();
      const links = [...(content?.matchAll(/\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g) ?? [])].map(match => match[1]);
      for (const link of links) {
        if (/^(?:https?:|mailto:|#)/i.test(link)) continue;
        const resolved = resolveKnowledgeHref(link, doc.id, 'zh');
        expect(resolved, `${doc.id} -> ${link}`).toBeTruthy();
        if (resolved?.startsWith('/')) {
          const route = parseRoute(resolved);
          expect(route.page).toBe('documentation');
          expect(route.docId === null || knowledgeDocs.some(item => item.id === route.docId)).toBe(true);
        } else {
          expect(resolved).toMatch(/^https:\/\/github\.com\//u);
        }
      }
    }
  });

  it('loads the documentation surface lazily and does not keep the placeholder path', () => {
    const main = readFileSync(resolve('marketing/site/main.tsx'), 'utf8');
    expect(main).toContain("lazy(() => import('./DocumentationPage')");
    expect(main).not.toContain("import { DocumentationPage } from './DocumentationPage'");
    expect(main).not.toContain('docsText');
    expect(readFileSync(resolve('marketing/site/content.ts'), 'utf8')).not.toContain('docsText');
  });
});