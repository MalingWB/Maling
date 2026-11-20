import { describe, expect, it, vi } from 'vitest';
import { createInlineDownloadLoader } from '../../marketing/site/inline-downloads';

const base = 'https://downloads.example.test/api/file/download/';
const endpoint = `${base}latest.json`;
const windows = `${base}MALING_0.22.0-wb.202611191100%2Babc1234_x64-setup.exe`;
const archive = `${base}MALING_aarch64.app.tar.gz`;
const appImage = `${base}MALING_0.22.0_amd64.AppImage`;
const version = '0.22.0-wb.202611191100+abc1234';
const current = (platforms: Record<string, { url: string }>) => Response.json({ version, platforms });

describe('runtime inline download manifest', () => {
  it('reads exact installer URLs, including escaped build IDs and the actual macOS archive', async () => {
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(current({
      'windows-x86_64': { url: windows },
      'darwin-aarch64': { url: archive },
      'linux-x86_64': { url: appImage },
      'windows-aarch64': { url: `${base}MALING_arm64-setup.exe` },
    }));
    const load = createInlineDownloadLoader(endpoint, request);
    expect(request).not.toHaveBeenCalled();
    expect(await load()).toEqual({ status: 'ready', version, downloads: {
      windows: { url: windows, filename: 'MALING_0.22.0-wb.202611191100+abc1234_x64-setup.exe' },
      mac: { url: archive, filename: 'MALING_aarch64.app.tar.gz' },
      linux: { url: appImage, filename: 'MALING_0.22.0_amd64.AppImage' },
    } });
    expect(request).toHaveBeenCalledExactlyOnceWith(endpoint, { cache: 'no-store', redirect: 'error', signal: expect.any(AbortSignal) });
  });

  it('combines overlapping reads but fetches again after completion, including after an empty response', async () => {
    let finish = (_value: Response) => {};
    const first = new Promise<Response>(resolve => { finish = resolve; });
    const request = vi.fn<typeof fetch>().mockReturnValueOnce(first)
      .mockResolvedValueOnce(Response.json({ version: '0.22.1', platforms: { 'darwin-aarch64': { url: `${base}MALING_0.22.1_aarch64.dmg` } } }));
    const load = createInlineDownloadLoader(endpoint, request);
    const a = load();
    expect(load()).toBe(a);
    expect(request).toHaveBeenCalledTimes(1);
    finish(current({}));
    expect(await a).toEqual({ status: 'empty' });
    expect(await load()).toEqual({ status: 'ready', version: '0.22.1', downloads: { mac: {
      url: `${base}MALING_0.22.1_aarch64.dmg`, filename: 'MALING_0.22.1_aarch64.dmg',
    } } });
    expect(request).toHaveBeenCalledTimes(2);
  });

  it('isolates invalid or unsupported entries without fabricating a fallback link', async () => {
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(current({
      'windows-x86_64': { url: windows },
      'linux-x86_64': { url: 'https://other.example.test/api/file/download/bad.AppImage' },
      'darwin-x86_64': { url: `${base}MALING_x64.app.tar.gz` },
    }));
    expect(await createInlineDownloadLoader(endpoint, request)()).toMatchObject({ status: 'ready', downloads: { windows: { url: windows } } });
    const unsupported = createInlineDownloadLoader(endpoint, vi.fn<typeof fetch>().mockResolvedValueOnce(current({
      'windows-aarch64': { url: `${base}MALING_arm64-setup.exe` },
    })));
    expect(await unsupported()).toEqual({ status: 'empty' });
  });

  it.each([
    'javascript:alert(1)',
    'http://downloads.example.test/api/file/download/other.exe',
    'https://other.example.test/api/file/download/other.exe',
    'https://name:pass@downloads.example.test/api/file/download/other.exe',
    'https://downloads.example.test/api/file/download/../other.exe',
    `${base}sub%2Ffile.exe`,
    `${base}sub%5Cfile.exe`,
    `${base}installer.exe.sig`,
    `${base}installer.exe?redirect=other`,
  ])('rejects a recognized platform with an invalid non-installer or out-of-scope URL (%s)', async url => {
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(current({ 'windows-x86_64': { url } }));
    expect(await createInlineDownloadLoader(endpoint, request)()).toEqual({ status: 'error', code: 'manifest-invalid' });
  });

  it('separates HTTP failure and malformed JSON, and can retry with the same reader', async () => {
    const request = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(new Response('', { status: 503 }))
      .mockResolvedValueOnce(new Response('<html>SSO</html>'))
      .mockResolvedValueOnce(Response.json({ version: '0.22.0', platforms: [] }))
      .mockResolvedValueOnce(current({ 'windows-x86_64': { url: windows } }));
    const load = createInlineDownloadLoader(endpoint, request);
    expect(await load()).toEqual({ status: 'error', code: 'manifest-unavailable' });
    expect(await load()).toEqual({ status: 'error', code: 'manifest-invalid' });
    expect(await load()).toEqual({ status: 'error', code: 'manifest-invalid' });
    expect(await load()).toMatchObject({ status: 'ready' });
  });

  it('aborts an unavailable manifest after the request deadline', async () => {
    vi.useFakeTimers();
    try {
      const request = vi.fn<typeof fetch>((_, init) => new Promise<Response>((_, reject) => {
        init?.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true });
      }));
      const load = createInlineDownloadLoader(endpoint, request);
      const result = load();
      await vi.advanceTimersByTimeAsync(4999);
      expect(request.mock.calls[0][1]?.signal?.aborted).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(request.mock.calls[0][1]?.signal?.aborted).toBe(true);
      expect(await result).toEqual({ status: 'error', code: 'manifest-unavailable' });
    } finally { vi.useRealTimers(); }
  });

  it('cancels an oversized manifest without retaining the entire body', async () => {
    const cancel = vi.fn();
    const body = new ReadableStream<Uint8Array>({
      start(controller) { controller.enqueue(new Uint8Array(128 * 1024 + 1)); },
      cancel,
    });
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(new Response(body));
    expect(await createInlineDownloadLoader(endpoint, request)()).toEqual({ status: 'error', code: 'manifest-invalid' });
    expect(cancel).toHaveBeenCalledOnce();
  });
});
