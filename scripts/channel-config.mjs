import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const baseTauriConfig = JSON.parse(
  readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
);

const MAIN_BINARY_NAME_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

const WB_BASE_URL_ENV = 'GOLD_BAND_WB_BASE_URL';

function resolveWbConfig(config, env) {
  const raw = env[WB_BASE_URL_ENV];
  let origin;
  try {
    if (typeof raw !== 'string' || !/^https?:\/\/[^\/?#@\\\s]+\/?$/i.test(raw)) throw new Error();
    const url = new URL(raw);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
        url.pathname !== '/' || url.search || url.hash || raw.includes('?') || raw.includes('#')) throw new Error();
    origin = url.origin;
  } catch {
    throw new Error(`Missing or invalid ${WB_BASE_URL_ENV}: expected an HTTP(S) origin without credentials, path, query or fragment.`);
  }
  if (!Number.isInteger(config.multicaPort) || config.multicaPort < 1 || config.multicaPort > 65535) {
    throw new Error('Invalid WB multicaPort.');
  }
  const multica = new URL(origin);
  multica.port = String(config.multicaPort);
  const replace = (value) => {
    if (typeof value === 'string') return value.replaceAll('${GOLD_BAND_WB_BASE_URL}', origin).replaceAll('${WB_MULTICA_URL}', multica.origin);
    if (Array.isArray(value)) return value.map(replace);
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, replace(item)]));
    return value;
  };
  return replace(config);
}

export function readChannelConfig(channel, env = process.env) {
  const configPath = join(repoRoot, 'configs', 'channels', `${channel}.json`);
  let config;
  try {
    config = JSON.parse(readFileSync(configPath, 'utf8'));
  } catch (error) {
    throw new Error(`Unsupported channel: ${channel}. Expected config file at ${configPath}. ${error instanceof Error ? error.message : String(error)}`);
  }

  if (config.channel !== channel) {
    throw new Error(`Channel config mismatch: expected ${channel}, found ${config.channel}.`);
  }

  // The NSIS installer finds and kills running instances by executable name only, so every channel
  // needs its own name to keep installing one channel from closing the others.
  if (typeof config.mainBinaryName !== 'string' || !MAIN_BINARY_NAME_PATTERN.test(config.mainBinaryName)) {
    throw new Error(`Channel ${channel} must set mainBinaryName to a lowercase executable name without extension.`);
  }

  return channel === 'wb' ? resolveWbConfig(config, env) : config;
}


export function channelEnvPrefix(channel) {
  return channel.toUpperCase().replace(/[^A-Z0-9]/g, '_');
}

export function tauriConfigOverlay(config, version, buildOptions = {}) {
  const overlay = {
    productName: config.productName,
    mainBinaryName: config.mainBinaryName,
    identifier: config.identifier,
    app: {
      ...baseTauriConfig.app,
      windows: baseTauriConfig.app.windows.map((windowConfig, index) => index === 0
        ? { ...windowConfig, title: config.windowTitle }
        : { ...windowConfig }),
    },
    plugins: {
      updater: {
        pubkey: config.updaterPublicKey,
        endpoints: [config.updaterEndpoint],
        dangerousInsecureTransportProtocol: Boolean(config.allowHttpUpdater),
        windows: {
          installMode: 'passive',
        },
      },
    },
  };

  if (version) {
    overlay.version = version;
  }

  const bundle = {
    publisher: config.productName,
  };
  if (config.bundleTargets) {
    bundle.targets = config.bundleTargets;
  }
  if (buildOptions.createUpdaterArtifacts !== undefined) {
    bundle.createUpdaterArtifacts = buildOptions.createUpdaterArtifacts;
  }
  overlay.bundle = bundle;

  return overlay;
}

export function writeTauriConfigOverlay(config, outputPath, version, buildOptions) {
  mkdirSync(dirname(outputPath), { recursive: true });
  writeFileSync(
    outputPath,
    `${JSON.stringify(tauriConfigOverlay(config, version, buildOptions), null, 2)}\n`,
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const channel = process.argv[2] ?? 'default';
  const outputPath = process.argv[3] ?? join(repoRoot, 'src-tauri', 'target', 'channel', `tauri.${channel}.conf.json`);
  const version = process.argv[4] || undefined;
  try {
    writeTauriConfigOverlay(readChannelConfig(channel), outputPath, version);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
