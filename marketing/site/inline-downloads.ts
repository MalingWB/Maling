import installerTemplates from '../../configs/wb-installer-filenames.json';

const MANIFEST_TIMEOUT_MS = 5_000;
const MAX_MANIFEST_BYTES = 128 * 1024;

// Match a release version as a single filename component; preserve prerelease and build identifiers verbatim.
const RELEASE_VERSION = /^(?:0|[1-9]\d*)(?:\.(?:0|[1-9]\d*)){2}(?:-[\da-zA-Z-]+(?:\.[\da-zA-Z-]+)*)?(?:\+[\da-zA-Z-]+(?:\.[\da-zA-Z-]+)*)?$/;

export type InlineDownloadPlatform = keyof typeof installerTemplates;
interface InlineDownload {
  url: string;
  filename: string;
}
export type InlineDownloadResult =
  | { status: 'ready'; version: string; downloads: Record<InlineDownloadPlatform, InlineDownload> }
  | { status: 'error'; code: 'manifest-unavailable' | 'manifest-invalid' };
export type LoadInlineDownloads = () => Promise<InlineDownloadResult>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function projectInstallers(manifest: unknown, directory: URL): InlineDownloadResult {
  const version = isRecord(manifest) ? manifest.version : undefined;
  if (typeof version !== 'string' || version.length > 128 || !RELEASE_VERSION.test(version)) {
    return { status: 'error', code: 'manifest-invalid' };
  }
  const installer = (template: string): InlineDownload => {
    const filename = template.replace('{version}', version);
    return { filename, url: new URL(encodeURIComponent(filename), directory).href };
  };
  return { status: 'ready', version, downloads: {
    mac: installer(installerTemplates.mac),
    windows: installer(installerTemplates.windows),
    linux: installer(installerTemplates.linux),
  } };
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
