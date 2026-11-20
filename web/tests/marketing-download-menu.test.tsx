/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { siteConfig } from '../../marketing/site/config';
import { copy } from '../../marketing/site/content';
import { DownloadMenu } from '../../marketing/site/DownloadMenu';
import type { InlineDownloadResult, LoadInlineDownloads } from '../../marketing/site/inline-downloads';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
window.ResizeObserver ||= class implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const config = siteConfig({ GOLD_BAND_WB_BASE_URL: 'https://updates.example.test' }, false);
const labels = copy.zh.downloadMenu;
const windowAsset = (version: string) => ({
  filename: `MALING_${version}_x64-setup.exe`,
  url: `https://updates.example.test/api/file/download/MALING_${encodeURIComponent(version)}_x64-setup.exe`,
});
const ready = (version: string) => ({ status: 'ready' as const, version, downloads: { windows: windowAsset(version) } });

let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

async function renderMenu(loadInlineDownloads: LoadInlineDownloads) {
  const props = { config, label: '下载客户端', labels, loadInlineDownloads };
  await act(async () => root.render(<DownloadMenu {...props} />));
}
async function openMenu() {
  const button = container.querySelector('button')!;
  await act(async () => {
    button.focus();
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  });
  expect(button.getAttribute('aria-expanded')).toBe('true');
}
async function closeMenu() {
  const menu = document.querySelector('[role="menu"]')!;
  await act(async () => {
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  });
  expect(container.querySelector('button')?.getAttribute('aria-expanded')).toBe('false');
}
const inlineLinks = () => [...document.querySelectorAll<HTMLAnchorElement>('a[data-inline-download-option]')].map(link => link.href);
const externalLink = () => document.querySelector<HTMLAnchorElement>('a[data-external-download-link]')?.href;

describe('website download menu', () => {
  it('uses the current manifest links on every open without changing the built site', async () => {
    const first = '0.21.1-wb.202611181115+abc1234';
    const second = '0.22.0-wb.202611190907+def5678';
    const load = vi.fn().mockResolvedValueOnce(ready(first)).mockResolvedValueOnce(ready(second));
    await renderMenu(load);
    expect(load).not.toHaveBeenCalled();
    await openMenu();
    expect(inlineLinks()).toEqual([windowAsset(first).url]);
    expect(externalLink()).toBe(config.downloadUrl);
    await closeMenu();
    await openMenu();
    expect(inlineLinks()).toEqual([windowAsset(second).url]);
    expect(document.querySelector('[role="menu"]')?.textContent).toContain(second);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it('uses the real macOS format and ignores a response from a closed menu', async () => {
    let finishOld: (value: InlineDownloadResult) => void = () => {};
    const pending = new Promise<InlineDownloadResult>(resolve => { finishOld = resolve; });
    const mac = { filename: 'MALING_aarch64.app.tar.gz', url: 'https://updates.example.test/api/file/download/MALING_aarch64.app.tar.gz' };
    const load = vi.fn().mockReturnValueOnce(pending).mockResolvedValueOnce({
      status: 'ready', version: '0.22.0', downloads: { mac },
    });
    await renderMenu(load);
    await openMenu();
    await closeMenu();
    await openMenu();
    expect(inlineLinks()).toEqual([mac.url]);
    expect(document.querySelector('a[data-inline-download-option="mac"]')?.textContent).toBe(labels.macArchive);
    await act(async () => finishOld(ready('0.21.1')));
    expect(inlineLinks()).toEqual([mac.url]);
  });

  it('distinguishes loading, failure and an empty release without hiding external downloads', async () => {
    let finish: (() => void) | undefined;
    const first = new Promise<{ status: 'error'; code: 'manifest-unavailable' }>(resolve => {
      finish = () => resolve({ status: 'error', code: 'manifest-unavailable' });
    });
    const load = vi.fn().mockReturnValueOnce(first).mockResolvedValueOnce({ status: 'empty' });
    await renderMenu(load);
    await openMenu();
    expect(inlineLinks()).toEqual([]);
    expect(document.querySelector('[role="status"]')?.textContent).toBe('正在获取行内安装包…');
    expect(externalLink()).toBe(config.downloadUrl);
    await act(async () => finish?.());
    expect(document.querySelector('[role="status"]')?.textContent).toBe('暂时无法获取行内安装包');
    expect(inlineLinks()).toEqual([]);
    expect(externalLink()).toBe(config.downloadUrl);
    await closeMenu();
    await openMenu();
    expect(document.querySelector('[role="status"]')?.textContent).toBe('暂无行内安装包');
    expect(inlineLinks()).toEqual([]);
    expect(externalLink()).toBe(config.downloadUrl);
  });
});
