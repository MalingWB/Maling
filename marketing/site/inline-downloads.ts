const MANIFEST_TIMEOUT_MS = 5_000;
const MAX_MANIFEST_BYTES = 128 * 1024;

const installerPlatforms = {
  mac: { key: 'darwin-aarch64', extension: /\.(?:dmg|app\.tar\.gz)$/i },
  windows: { key: 'windows-x86_64', extension: /\.(?:exe|msi)$/i },
  linux: { key: 'linux-x86_64', extension: /\.appimage$/i },
} as const;

export type InlineDownloadPlatform = keyof typeof installerPlatforms;
interface InlineDownload {
  url: string;
  filename: string;
}
export type InlineDownloadResult =
  | { status: 'ready'; version: string; downloads: Partial<Record<InlineDownloadPlatform, InlineDownload>> }
  | { status: 'empty' }
  | { status: 'error'; code: 'manifest-unavailable' | 'manifest-invalid' };
export type LoadInlineDownloads = () => Promise<InlineDownloadResult>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function trustedInstaller(raw: unknown, directory: URL, extension: RegExp): InlineDownload | null {
  if (!isRecord(raw) || typeof raw.url !== 'string') return null;
  try {
    const url = new URL(raw.url);
    if (url.protocol !== directory.protocol || url.origin !== directory.origin || url.username || url.password
      || url.search || url.hash || !url.pathname.startsWith(directory.pathname)) return null;
    const filename = decodeURIComponent(url.pathname.slice(directory.pathname.length));
    if (!/^[\w.+-]+$/.test(filename) || !extension.test(filename)) return null;
    return { url: url.href, filename };
  } catch {
    return null;
  }
}

function projectInstallers(manifest: unknown, directory: URL): InlineDownloadResult {
  if (!isRecord(manifest) || !isRecord(manifest.platforms) || typeof manifest.version !== 'string'
    || !manifest.version.trim() || manifest.version.length > 128) return { status: 'error', code: 'manifest-invalid' };
  const downloads: Partial<Record<InlineDownloadPlatform, InlineDownload>> = {};
  let invalid = false;
  for (const platform of Object.keys(installerPlatforms) as InlineDownloadPlatform[]) {
    const { key, extension } = installerPlatforms[platform];
    if (!Object.hasOwn(manifest.platforms, key)) continue;
    const installer = trustedInstaller(manifest.platforms[key], directory, extension);
    if (installer) downloads[platform] = installer;
    else invalid = true;
  }
  if (Object.keys(downloads).length) return { status: 'ready', version: manifest.version, downloads };
  return invalid ? { status: 'error', code: 'manifest-invalid' } : { status: 'empty' };
}

async function readManifest(response: Response): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) return null;
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let bytes = 0;
  let text = '';
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) return JSON.parse(text + decoder.decode()) as unknown;
      bytes += value.byteLength;
      if (bytes > MAX_MANIFEST_BYTES) throw new RangeError('Download manifest exceeds the response limit');
      text += decoder.decode(value, { stream: true });
    }
  } catch (error) {
    void reader.cancel().catch(() => undefined);
    throw error;
  } finally {
    reader.releaseLock();
  }
}

async function fetchInstallers(endpoint: URL, directory: URL, request: typeof fetch): Promise<InlineDownloadResult> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), MANIFEST_TIMEOUT_MS);
  try {
    const response = await request(endpoint.href, { cache: 'no-store', redirect: 'error', signal: controller.signal });
    if (!response.ok) {
      void response.body?.cancel().catch(() => undefined);
      return { status: 'error', code: 'manifest-unavailable' };
    }
    try {
      return projectInstallers(await readManifest(response), directory);
    } catch {
      return { status: 'error', code: controller.signal.aborted ? 'manifest-unavailable' : 'manifest-invalid' };
    }
  } catch {
    return { status: 'error', code: 'manifest-unavailable' };
  } finally {
    clearTimeout(timeout);
  }
}

// The homepage's two download buttons share only the active request, never its result.
export function createInlineDownloadLoader(manifestUrl: string, request: typeof fetch = fetch): LoadInlineDownloads {
  const endpoint = new URL(manifestUrl);
  const directory = new URL('.', endpoint);
  let pending: Promise<InlineDownloadResult> | undefined;
  return () => pending ??= fetchInstallers(endpoint, directory, request).finally(() => { pending = undefined; });
}
