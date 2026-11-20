import { describe, expect, it, vi } from 'vitest';
import { createInlineDownloadLoader } from '../../marketing/site/inline-downloads';

const base = 'https://downloads.example.test/api/file/download/';
const endpoint = `${base}latest.json`;

function expectedDownloads(version: string) {
  const asset = (filename: string) => ({ filename, url: `${base}${encodeURIComponent(filename)}` });
  return {
    mac: asset(`MALING_${version}_aarch64.dmg`),
    windows: asset(`MALING_${version}_x64-setup.exe`),
    linux: asset(`MALING_${version}_amd64.AppImage`),
  };
}

describe('runtime inline download manifest', () => {
  it.each([
    { version: '0.22.0' },
    { version: '0.22.0-wb.202611191100+abc1234', platforms: {} },
    { version: '1.2.3+build.7', platforms: {
      'darwin-aarch64': { url: `${base}MALING_aarch64.app.tar.gz` },
      'windows-x86_64': { url: 'https://other.example.test/old-setup.exe' },
    } },
  ])('generates all three installer links solely from $version, not updater URLs', async manifest => {
    const request = vi.fn<typeof fetch>().mockResolvedValueOnce(Response.json(manifest));
    const load = createInlineDownloadLoader(endpoint, request);
    expect(request).not.toHaveBeenCalled();
    expect(await load()).toEqual({ status: 'ready', version: manifest.version, downloads: expectedDownloads(manifest.version) });
    expect(request).toHaveBeenCalledExactlyOnceWith(endpoint, { cache: 'no-store', redirect: 'error', signal: expect.any(AbortSignal) });
  });

  it('shares only overlapping requests and switches every installer to the next version on reopen', async () => {
    let finish = (_value: Response) => {};
    const first = new Promise<Response>(resolve => { finish = resolve; });
    const request = vi.fn<typeof fetch>().mockReturnValueOnce(first).mockResolvedValueOnce(Response.json({ version: '0.22.1' }));
    const load = createInlineDownloadLoader(endpoint, request);
    const a = load();
    expect(load()).toBe(a);
    expect(request).toHaveBeenCalledTimes(1);
    finish(Response.json({ version: '0.22.0' }));
    expect(await a).toEqual({ status: 'ready', version: '0.22.0', downloads: expectedDownloads('0.22.0') });
    expect(await load()).toEqual({ status: 'ready', version: '0.22.1', downloads: expectedDownloads('0.22.1') });
    expect(request).toHaveBeenCalledTimes(2);
  });

  it.each([undefined, null, 22, '', ' v1.2.3', '1.2', '1.2.3-', '1.2.3-rc..1', '1.2.3/../../x',
    '1.2.3%2Fother', '1.2.3?x', '1.2.3#x', '1.2.3\n', `1.2.3+${'a'.repeat(128)}`])(
    'rejects an invalid or unsafe release version (%s) without returning links', async version => {
      const request = vi.fn<typeof fetch>().mockResolvedValueOnce(Response.json({ version, platforms: {} }));
      expect(await createInlineDownloadLoader(endpoint, request)()).toEqual({ status: 'error', code: 'manifest-invalid' });
    },
  );

  it('separates HTTP failure and malformed JSON, and can retry with the same reader', async () => {
    const request = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(new Response('', { status: 503 }))
      .mockResolvedValueOnce(new Response('<html>SSO</html>'))
      .mockResolvedValueOnce(Response.json({ version: '1.2.3' }));
    const load = createInlineDownloadLoader(endpoint, request);
    expect(await load()).toEqual({ status: 'error', code: 'manifest-unavailable' });
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
