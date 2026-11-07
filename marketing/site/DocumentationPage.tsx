import { useEffect, useMemo, useState, type ComponentProps, type ReactElement } from 'react';
import { Loader2, RotateCcw } from 'lucide-react';
import { defaultRehypePlugins, Streamdown } from 'streamdown';
import type { Plugin, PluggableList } from 'unified';
import 'streamdown/styles.css';
import { Button } from '@/components/ui/button';
import { copy, pageHref, type Language } from './content';
import { findKnowledgeDoc, knowledgeDocs, loadKnowledgeDoc, normalizeKnowledgeDocId, prepareKnowledgeMarkdown, resolveKnowledgeHref, type KnowledgeSectionId } from './knowledge-docs';

const [hardenPlugin, hardenOptions] = defaultRehypePlugins.harden as [Plugin, Record<string, unknown>];
const documentationRehypePlugins = [
  defaultRehypePlugins.raw,
  defaultRehypePlugins.sanitize,
  [hardenPlugin, { ...hardenOptions, allowedLinkPrefixes: ['*'], allowedImagePrefixes: ['*'] }],
] as PluggableList;

type DocumentationContent = { status: 'loading' } | { status: 'ready'; content: string } | { status: 'not-found' } | { status: 'error' };
type LastReadyDocument = { id: string; content: string };

function DocumentationAnchor({ children, href = '', node: _node, ...props }: ComponentProps<'a'> & { node?: unknown }) {
  const external = /^https?:\/\//i.test(href);
  return <a {...props} href={href} target={external ? '_blank' : undefined} rel={external ? 'noreferrer' : undefined}>{children}</a>;
}

export function DocumentationPage({ docId, language, base = '/' }: { docId: string | null; language: Language; base?: string }) {
  const t = copy[language];
  const normalizedId = normalizeKnowledgeDocId(docId);
  const [documents, setDocuments] = useState<Record<string, DocumentationContent>>({});
  const [lastReady, setLastReady] = useState<LastReadyDocument | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const isKnownDocument = Boolean(findKnowledgeDoc(normalizedId));
  const state = documents[normalizedId] ?? (isKnownDocument ? { status: 'loading' } : { status: 'not-found' });

  useEffect(() => {
    if (!isKnownDocument) return;
    let active = true;
    setDocuments(current => current[normalizedId] ? current : { ...current, [normalizedId]: { status: 'loading' } });
    loadKnowledgeDoc(normalizedId).then(content => {
      if (!active || content === null) return;
      const preparedContent = prepareKnowledgeMarkdown(content, normalizedId, language, base);
      setDocuments(current => ({ ...current, [normalizedId]: { status: 'ready', content: preparedContent } }));
      setLastReady({ id: normalizedId, content: preparedContent });
    }).catch(() => {
      if (active) setDocuments(current => ({ ...current, [normalizedId]: { status: 'error' } }));
    });
    return () => { active = false; };
  }, [normalizedId, reloadToken, language, base]);

  const overviewDocs = useMemo(() => knowledgeDocs.filter(doc => doc.sectionId === ''), []);
  const groupedDocs = useMemo(() => (['product', 'practice', 'reference'] as const).map(sectionId => ({
    sectionId,
    title: t.docs.sections[sectionId],
    docs: knowledgeDocs.filter(doc => doc.sectionId === sectionId),
  })), [t.docs.sections]);
  const selectedDoc = findKnowledgeDoc(normalizedId);
  const previousContent = state.status === 'loading' && lastReady && lastReady.id !== normalizedId ? lastReady.content : null;
  const transformUrl = (url: string) => {
    if (/^\/(?:zh|en)\/documentation\//.test(url)) return url;
    return resolveKnowledgeHref(url, normalizedId, language, base);
  };

  return <section className="documentation-page">
    {language === 'en' && t.docs.languageNotice ? <p className="docs-language-notice" role="note">{t.docs.languageNotice}</p> : null}
    <div className="documentation-layout">
      <nav className="documentation-nav" aria-label={t.nav[1]}>
        <div className="documentation-nav-group">
          <p>{t.docs.overview}</p>
          {overviewDocs.map(doc => <a key={doc.id || 'overview'} href={pageHref(language, 'documentation', base, doc.id || null)} aria-current={normalizedId === doc.id ? 'page' : undefined}>{doc.id ? doc.title : t.docs.overview}</a>)}
        </div>
        {groupedDocs.map(group => group.docs.length ? (
          <div className="documentation-nav-group" key={group.sectionId}>
            <p>{group.title}</p>
            {group.docs.map(doc => <a key={doc.id} href={pageHref(language, 'documentation', base, doc.id)} aria-current={normalizedId === doc.id ? 'page' : undefined}>{doc.title}</a>)}
          </div>
        ) : null)}
      </nav>
      <article className="docs-content" data-state={state.status} aria-busy={state.status === 'loading'}>
        {state.status === 'loading' && previousContent ? <>
          <div className="docs-previous-content" aria-hidden="true"><Streamdown className="knowledge-markdown" mode="static" urlTransform={transformUrl} rehypePlugins={documentationRehypePlugins} components={{ a: DocumentationAnchor }}>{previousContent}</Streamdown></div>
          <div className="docs-loading-veil" role="status"><Loader2 className="animate-spin" /><span>{t.docs.loading}</span></div>
        </> : state.status === 'loading' ? <div className="docs-status" role="status"><Loader2 className="animate-spin" />{t.docs.loading}</div>
          : state.status === 'not-found' ? <div className="docs-status"><p>{t.docs.notFound}</p><Button asChild><a href={pageHref(language, 'documentation', base, null)}>{t.docs.overview}</a></Button></div>
          : state.status === 'error' ? <div className="docs-status"><p>{t.docs.error}</p><Button variant="secondary" onClick={() => setReloadToken(value => value + 1)}><RotateCcw />{t.docs.retry}</Button></div>
          : selectedDoc ? <Streamdown className="knowledge-markdown" mode="static" urlTransform={transformUrl} rehypePlugins={documentationRehypePlugins} components={{ a: DocumentationAnchor }}>{state.content}</Streamdown> : null}
      </article>
    </div>
  </section>;
}