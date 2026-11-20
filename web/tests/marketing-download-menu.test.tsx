/** @vitest-environment jsdom */
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { EventType, IncrementalSource, MouseInteractions, ReplayerEvents, type eventWithTime } from '@rrweb/types';
import { createSceneEngine } from '../../marketing/site/Replay';
import { SceneManifestSchema } from '../../marketing/site/replay-model';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { siteConfig } from '../../marketing/site/config';
import { copy } from '../../marketing/site/content';
import { DownloadMenu } from '../../marketing/site/DownloadMenu';
import { createInlineDownloadLoader, type InlineDownloadResult, type LoadInlineDownloads } from '../../marketing/site/inline-downloads';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
window.ResizeObserver ||= class implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const config = siteConfig({ GOLD_BAND_WB_BASE_URL: 'https://updates.example.test' }, false);
const labels = copy.zh.downloadMenu;
const expectedLinks = (version: string) => ['aarch64.dmg', 'x64-setup.exe', 'amd64.AppImage'].map(suffix =>
  new URL(`MALING_${encodeURIComponent(version)}_${suffix}`, config.inlineDownloadManifestUrl).href);

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
  it('offers the three manual installers and switches all three to the newly published version on reopen', async () => {
    const first = '0.21.1-wb.202611181115+abc1234';
    const second = '0.22.0';
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(Response.json({ version: first }))
      .mockResolvedValueOnce(Response.json({ version: second, platforms: { 'darwin-aarch64': { url: 'https://other.example.test/MALING_aarch64.app.tar.gz' } } }));
    await renderMenu(createInlineDownloadLoader(config.inlineDownloadManifestUrl, request));
    expect(request).not.toHaveBeenCalled();
    await openMenu();
    expect(inlineLinks()).toEqual(expectedLinks(first));
    expect(externalLink()).toBe(config.downloadUrl);
    await closeMenu();
    await openMenu();
    expect(inlineLinks()).toEqual(expectedLinks(second));
    expect(document.querySelector('[data-inline-download-option="mac"]')?.textContent).toBe(labels.mac);
    expect(document.querySelector('[role="menu"]')?.textContent).toContain(second);
    expect(request).toHaveBeenCalledTimes(2);
  });

  it('keeps the open download menu and its focused link when the recording replays a focus event', async () => {
    const asset = SceneManifestSchema.parse(JSON.parse(readFileSync('marketing/site/media/before/manifest.json', 'utf8'))).assets[0];
    const { events }: { events: eventWithTime[] } = JSON.parse(readFileSync(`marketing/site${asset.eventsUrl}`, 'utf8'));
    const focusIndex = events.findIndex(e => e.type === EventType.IncrementalSnapshot
      && e.data.source === IncrementalSource.MouseInteraction && e.data.type === MouseInteractions.Focus && e.data.id > 0);
    expect(focusIndex).toBeGreaterThan(0);
    const at = events[focusIndex].timestamp - events[0].timestamp;
    const stage = document.body.appendChild(document.createElement('div'));
    // JSDOM needs the reflected sandbox tokens for rrweb to rebuild its actual iframe.
    const sandbox = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'sandbox');
    if (!sandbox) Object.defineProperty(HTMLIFrameElement.prototype, 'sandbox', { configurable: true,
      get() { return (this.getAttribute('sandbox') ?? '').split(/\s+/); } });
    let engine: ReturnType<typeof createSceneEngine> | undefined;
    try {
      engine = createSceneEngine(events.slice(0, focusIndex + 1), asset, stage);
      engine.iframe.contentWindow!.scrollTo = () => {};
      engine.pause(at - 1);
      await renderMenu(createInlineDownloadLoader(config.inlineDownloadManifestUrl, async () => Response.json({ version: '0.22.0' })));
      await openMenu();
      const link = document.querySelector<HTMLAnchorElement>('a[data-inline-download-option="windows"]')!;
      await act(async () => link.focus());
      expect(document.activeElement).toBe(link);
      const interaction = vi.fn();
      engine.on(ReplayerEvents.MouseInteraction, interaction);
      await act(async () => engine!.pause(at + 1));
      expect(interaction).toHaveBeenCalledWith(expect.objectContaining({ type: MouseInteractions.Focus }));
      expect(document.activeElement?.getAttribute('data-inline-download-option')).toBe('windows');
      expect(document.querySelector('[role="menu"]')?.contains(link)).toBe(true);
      await closeMenu();
    } finally {
      engine?.destroy(); stage.remove();
      if (!sandbox) delete (HTMLIFrameElement.prototype as Partial<HTMLIFrameElement>).sandbox;
    }
  });

  it('ignores a late error from a closed menu', async () => {
    let finishOld: (value: InlineDownloadResult) => void = () => {};
    const pending = new Promise<InlineDownloadResult>(resolve => { finishOld = resolve; });
    const load = vi.fn<LoadInlineDownloads>().mockReturnValueOnce(pending).mockImplementationOnce(
      createInlineDownloadLoader(config.inlineDownloadManifestUrl, async () => Response.json({ version: '0.22.0' })),
    );
    await renderMenu(load);
    await openMenu();
    await closeMenu();
    await openMenu();
    expect(inlineLinks()).toEqual(expectedLinks('0.22.0'));
    await act(async () => finishOld({ status: 'error', code: 'manifest-unavailable' }));
    expect(inlineLinks()).toEqual(expectedLinks('0.22.0'));
  });

  it('keeps external downloads available while loading, after failure, and when retrying', async () => {
    let finish: (() => void) | undefined;
    const first = new Promise<Response>(resolve => { finish = () => resolve(new Response('', { status: 503 })); });
    const request = vi.fn<typeof fetch>().mockReturnValueOnce(first).mockResolvedValueOnce(Response.json({ version: '1.2.3' }));
    await renderMenu(createInlineDownloadLoader(config.inlineDownloadManifestUrl, request));
    await openMenu();
    expect(inlineLinks()).toEqual([]);
    expect(document.querySelector('[role="status"]')?.textContent).toBe(labels.inlineLoading);
    expect(externalLink()).toBe(config.downloadUrl);
    await act(async () => finish?.());
    expect(document.querySelector('[role="status"]')?.textContent).toBe(labels.inlineUnavailable);
    expect(inlineLinks()).toEqual([]);
    expect(externalLink()).toBe(config.downloadUrl);
    await closeMenu();
    await openMenu();
    expect(inlineLinks()).toEqual(expectedLinks('1.2.3'));
    expect(externalLink()).toBe(config.downloadUrl);
  });
});
