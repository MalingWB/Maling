/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { loadKnowledgeDoc } = vi.hoisted(() => ({ loadKnowledgeDoc: vi.fn() }));
vi.mock('../../marketing/site/knowledge-docs', async importOriginal => {
  const actual = await importOriginal<typeof import('../../marketing/site/knowledge-docs')>();
  return { ...actual, loadKnowledgeDoc };
});

import { DocumentationPage } from '../../marketing/site/DocumentationPage';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
window.ResizeObserver ||= ResizeObserver;

let root: Root;
let container: HTMLDivElement;
let pendingLoads: Array<{ id: string; resolve: (content: string | null) => void }>;

beforeEach(() => {
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
  pendingLoads = [];
  loadKnowledgeDoc.mockImplementation((id: string | null) => new Promise(resolve => {
    pendingLoads.push({ id: id ?? '', resolve });
  }));
});

afterEach(async () => {
  pendingLoads.forEach(({ resolve }) => resolve(null));
  await act(async () => root.unmount());
  document.body.replaceChildren();
});

describe('website documentation page', () => {
  it('renders the bounded navigation list and the selected Markdown document', async () => {
    await act(async () => root.render(<DocumentationPage docId={null} language="zh" />));
    await act(async () => {});
    pendingLoads.find(load => load.id === '')?.resolve('# 知识库总览');
    await act(async () => {});
    await vi.waitFor(() => expect(container.querySelector('.knowledge-markdown')).toBeTruthy());
    expect(container.querySelectorAll('.documentation-nav-group > a')).toHaveLength(11);
    expect(container.querySelector('[aria-current="page"]')?.textContent).toContain('知识库总览');
  });

  it('keeps the previous document height while a different document loads', async () => {
    await act(async () => root.render(<DocumentationPage docId={null} language="zh" />));
    await act(async () => {});
    pendingLoads.find(load => load.id === '')?.resolve('# 知识库总览');
    await act(async () => {});
    await vi.waitFor(() => expect(container.querySelector('.knowledge-markdown')).toBeTruthy());

    await act(async () => root.render(<DocumentationPage docId="01-product/01-产品概览与定位" language="zh" />));
    await act(async () => {});
    expect(container.querySelector('.docs-content')?.getAttribute('data-state')).toBe('loading');
    expect(container.querySelector('.docs-previous-content')).toBeTruthy();
    expect(container.querySelector('.docs-loading-veil')).toBeTruthy();
    expect(container.querySelector('.docs-status')).toBeNull();

    pendingLoads.find(load => load.id === '01-product/01-产品概览与定位')?.resolve('# 产品概览与定位');
    await act(async () => {});
    await vi.waitFor(() => expect(container.querySelector('.docs-content')?.getAttribute('data-state')).toBe('ready'), { timeout: 3000, interval: 50 });
    expect(container.querySelector('.docs-previous-content')).toBeNull();
    expect(container.querySelector('.docs-loading-veil')).toBeNull();
    expect(container.textContent).toContain('产品概览与定位');
  });

  it('keeps unknown documents inside the documentation empty state', async () => {
    await act(async () => root.render(<DocumentationPage docId="not-a-doc" language="en" />));
    await act(async () => {});
    expect(container.textContent).toContain('Document not found');
    expect(container.querySelectorAll('.documentation-nav-group > a')).toHaveLength(11);
    expect(container.querySelector('.knowledge-markdown')).toBeNull();
  });
});