import { createHash } from 'node:crypto';
import {
  access, copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile,
} from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import npa from 'npm-package-arg';
import semver from 'semver';

export const ACP_REGISTRY_URL = 'https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json';
export const LOCAL_SUPPLEMENT_RESOURCE_PATH = 'resources/acp-registry-local.json';
export const BUILTIN_AGENT_IDS = [
  'claude-acp',
  'codebuddy-code',
  'codex-acp',
  'pi-acp',
  'deepseek-harness',
  'zcode-acp',
  'qoder',
  'kimi',
  'cursor',
  'gemini',
  'goose',
  'qwen-code',
  'opencode',
  'amp-acp',
];

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const operatingSystems = ['windows', 'macos', 'linux'];
const architectures = ['x86_64', 'aarch64'];
const integrationOrigins = ['official', 'community-adapter'];

const overrides = {
  'claude-acp': {
    label: 'Claude', iconKey: 'claude', primaryAgentDir: '.claude', compatibleAgentDirs: [],
    localCommand: 'claude-agent-acp', installedCommand: 'claude',
    supportsSystemPrompt: true,
  },
  'codex-acp': {
    label: 'Codex', iconKey: 'codex', primaryAgentDir: '.codex', compatibleAgentDirs: ['.agents'],
    localCommand: 'codex-acp', installedCommand: 'codex',
  },
  cursor: {
    label: 'Cursor', iconKey: 'cursor', command: 'cursor-agent', args: ['acp'],
    primaryAgentDir: '.cursor', compatibleAgentDirs: ['.agents'],
  },
  gemini: {
    label: 'Gemini', iconKey: 'gemini', primaryAgentDir: '.gemini', compatibleAgentDirs: ['.agents'],
    localCommand: 'gemini',
  },
  'codebuddy-code': {
    label: 'WorkBuddy', iconKey: 'codebuddy-code', primaryAgentDir: '.codebuddy', compatibleAgentDirs: [],
    localCommand: 'codebuddy',
  },
  goose: {
    label: 'Goose', iconKey: 'goose', command: 'goose', args: ['acp'],
    primaryAgentDir: '.goose', compatibleAgentDirs: [],
  },
  'qwen-code': {
    label: 'Qwen Code', iconKey: 'qwen-code', primaryAgentDir: '.qwen', compatibleAgentDirs: [],
    localCommand: 'qwen',
  },
  opencode: {
    label: 'OpenCode', iconKey: 'opencode', command: 'opencode', args: ['acp'],
    primaryAgentDir: '.opencode', compatibleAgentDirs: ['.agents'],
  },
  kimi: {
    label: 'Kimi Code', iconKey: 'kimi', command: 'kimi', args: ['acp'],
    primaryAgentDir: '.kimi-code', compatibleAgentDirs: ['.agents'],
  },
  'amp-acp': {
    label: 'Amp', iconKey: 'amp-acp', command: 'amp-acp', args: [],
    primaryAgentDir: '.agents', compatibleAgentDirs: ['.claude'],
  },
  'pi-acp': {
    label: 'Pi', iconKey: 'pi-acp', primaryAgentDir: '.pi/agent',
    localCommand: 'pi-acp', installedCommand: 'pi',
    projectPrimaryAgentDir: '.pi', compatibleAgentDirs: ['.agents'],
  },
  qoder: {
    label: 'Qoder', iconKey: 'qoder', primaryAgentDir: '.qoder', compatibleAgentDirs: ['.agents'],
    excludedPlatforms: [{ os: 'windows', arch: 'aarch64' }],
  },
};

export function buildAgentCatalog(
  registry,
  fetchedAt = new Date().toISOString(),
  policy = {},
  localSupplement = { agents: [] },
  localSupplementSha256 = '',
) {
  validatePolicy(policy);
  if (!registry || !Array.isArray(registry.agents)) {
    throw new Error('ACP Registry response does not contain an agents array.');
  }
  if (!localSupplement || !Array.isArray(localSupplement.agents)) {
    throw new Error('Local Agent supplement must contain an agents array.');
  }
  const resolvedLocalDigest = localSupplementSha256
    || (localSupplement.agents.length === 0 && sha256(JSON.stringify(localSupplement)));
  if (!/^[a-f0-9]{64}$/.test(resolvedLocalDigest ?? '')) {
    throw new Error('Local Agent supplement digest is required.');
  }

  const pins = policy.versionPins ?? {};
  const validatedVersions = policy.validatedVersions ?? {};
  const launchOverrides = policy.launchOverrides ?? {};
  const byId = new Map(registry.agents.map((agent) => [agent.id, agent]));
  const localById = new Map(localSupplement.agents.map((agent) => [agent.id, agent]));
  for (const id of localById.keys()) {
    if (byId.has(id)) {
      throw new Error(`Agent ${id} exists in both official Registry and local supplement.`);
    }
  }
  const catalogIds = new Set([...byId.keys(), ...localById.keys()]);
  const missing = BUILTIN_AGENT_IDS.filter((id) => !catalogIds.has(id));
  const extra = [...localById.keys()].filter((id) => !BUILTIN_AGENT_IDS.includes(id));
  if (missing.length > 0) {
    throw new Error(`ACP Registry is missing required agents: ${missing.join(', ')}.`);
  }
  if (extra.length > 0) {
    throw new Error(`Agent catalog contains uncurated agents: ${extra.join(', ')}.`);
  }

  const agents = BUILTIN_AGENT_IDS.map((id) => {
    const local = localById.get(id);
    if (local) return localCatalogAgent(id, local);
    const agent = byId.get(id);
    const override = { ...overrides[id], ...(launchOverrides[id] ?? {}) };
    const distribution = resolveDistributionDefaults(agent.distribution);
    const pin = pins[id];
    if (pin !== undefined) {
      if (!agent.distribution?.npx || override.command || override.args) {
        throw new Error(`Agent ${id} version pin requires an npx-managed template.`);
      }
      const spec = npa(agent.distribution.npx.package);
      if (!spec.registry || !spec.name || !['version', 'range', 'tag'].includes(spec.type)) {
        throw new Error(`Agent ${id} version pin requires an npm registry package.`);
      }
      distribution.args[1] = `${spec.name}@${pin}`;
    }
    if (override.extraArgs?.length) distribution.args.push(...override.extraArgs);
    const version = pin ?? String(agent.version ?? '');
    const validatedVersion = validatedVersions[id];
    return {
      id,
      label: override.label ?? agent.name,
      version,
      provenance: {
        sourceKind: 'official-registry',
        integrationOrigin: 'official',
        validatedVersion,
      },
      description: String(agent.description ?? ''),
      repository: agent.repository ?? null,
      website: agent.website ?? null,
      iconKey: override.iconKey ?? id,
      iconUrl: agent.icon,
      command: override.command ?? distribution.command,
      args: override.args ?? distribution.args,
      localLaunch: override.localCommand
        ? {
          command: override.localCommand,
          args: [...(agent.distribution?.npx?.args ?? []), ...(override.extraArgs ?? [])],
        }
        : null,
      installedCommand: override.installedCommand ?? null,
      env: distribution.env,
      primaryAgentDir: override.primaryAgentDir ?? null,
      projectPrimaryAgentDir: override.projectPrimaryAgentDir ?? null,
      compatibleAgentDirs: override.compatibleAgentDirs ?? [],
      supportsSystemPrompt: override.supportsSystemPrompt ?? false,
      supportsExternalSessionSync: false,
      excludedPlatforms: validateExcludedPlatforms(id, override.excludedPlatforms),
    };
  });

  return {
    schemaVersion: 2,
    source: {
      officialRegistry: {
        url: ACP_REGISTRY_URL,
        registryVersion: String(registry.version ?? ''),
        fetchedAt,
      },
      localSupplement: {
        path: LOCAL_SUPPLEMENT_RESOURCE_PATH,
        sha256: resolvedLocalDigest,
        fetchedAt,
      },
    },
    agents,
  };
}

function validatePolicy(policy) {
  if (!policy || typeof policy !== 'object' || Array.isArray(policy)
      || Object.keys(policy).some((key) => !['versionPins', 'validatedVersions', 'launchOverrides'].includes(key))) {
    throw new Error('Invalid Agent catalog policy.');
  }
  const pins = policy.versionPins === undefined ? {} : policy.versionPins;
  if (!pins || typeof pins !== 'object' || Array.isArray(pins)) {
    throw new Error('versionPins must be an object.');
  }
  for (const [id, version] of Object.entries(pins)) {
    if (!BUILTIN_AGENT_IDS.includes(id)) throw new Error(`Unknown Agent version pin: ${id}`);
    if (typeof version !== 'string' || !version || npa.resolve('version-pin', version).type !== 'version') {
      throw new Error(`Agent ${id} version pin must be an exact npm version.`);
    }
  }
  const validatedVersions = policy.validatedVersions === undefined ? {} : policy.validatedVersions;
  if (!validatedVersions || typeof validatedVersions !== 'object' || Array.isArray(validatedVersions)) {
    throw new Error('validatedVersions must be an object.');
  }
  for (const [id, version] of Object.entries(validatedVersions)) {
    if (!BUILTIN_AGENT_IDS.includes(id)) throw new Error(`Unknown Agent validated version: ${id}`);
    if (typeof version !== 'string' || !semver.valid(version)) {
      throw new Error(`Agent ${id} validated version must be an exact semantic version.`);
    }
  }
  const launchOverrides = policy.launchOverrides === undefined ? {} : policy.launchOverrides;
  if (!launchOverrides || typeof launchOverrides !== 'object' || Array.isArray(launchOverrides)) {
    throw new Error('launchOverrides must be an object.');
  }
  for (const [id, override] of Object.entries(launchOverrides)) {
    if (!BUILTIN_AGENT_IDS.includes(id)) throw new Error(`Unknown Agent launch override: ${id}`);
    if (!override || typeof override !== 'object' || Array.isArray(override)) {
      throw new Error(`Agent ${id} launch override must be an object.`);
    }
    const allowed = new Set(['extraArgs', 'localCommand', 'installedCommand']);
    if (Object.keys(override).some((key) => !allowed.has(key))) {
      throw new Error(`Agent ${id} launch override contains an unsupported field.`);
    }
    if (override.extraArgs !== undefined && (
      !Array.isArray(override.extraArgs) || override.extraArgs.some((arg) => typeof arg !== 'string' || !arg)
    )) throw new Error(`Agent ${id} extraArgs must be non-empty strings.`);
    for (const field of ['localCommand', 'installedCommand']) {
      if (override[field] !== undefined && (typeof override[field] !== 'string' || !override[field].trim())) {
        throw new Error(`Agent ${id} ${field} must be a non-empty string.`);
      }
    }
  }
}

function localCatalogAgent(id, agent) {
  if (!agent || typeof agent !== 'object') throw new Error(`Local Agent ${id} must be an object.`);
  const provenance = agent.provenance;
  if (!provenance || provenance.sourceKind !== 'local-supplement'
      || !integrationOrigins.includes(provenance.integrationOrigin)
      || typeof provenance.validatedVersion === 'undefined') {
    throw new Error(`Local Agent ${id} has invalid provenance.`);
  }
  if (agent.version !== provenance.validatedVersion) {
    throw new Error(`Local Agent ${id} version must equal its validated version.`);
  }
  const iconKey = String(agent.iconKey ?? id);
  validateIconKey(id, iconKey);
  return {
    id,
    label: String(agent.label ?? ''),
    version: String(agent.version ?? ''),
    provenance: {
      sourceKind: 'local-supplement',
      integrationOrigin: provenance.integrationOrigin,
      validatedVersion: String(provenance.validatedVersion),
    },
    description: String(agent.description ?? ''),
    repository: agent.repository ?? null,
    website: agent.website ?? null,
    iconKey,
    iconUrl: String(agent.iconUrl ?? ''),
    command: String(agent.command ?? ''),
    args: Array.isArray(agent.args) ? agent.args.map(String) : [],
    localLaunch: agent.localLaunch ?? null,
    installedCommand: agent.installedCommand ?? null,
    env: agent.env ?? {},
    primaryAgentDir: agent.primaryAgentDir ?? null,
    projectPrimaryAgentDir: agent.projectPrimaryAgentDir ?? null,
    compatibleAgentDirs: Array.isArray(agent.compatibleAgentDirs) ? agent.compatibleAgentDirs : [],
    supportsSystemPrompt: agent.supportsSystemPrompt ?? false,
    supportsExternalSessionSync: false,
    excludedPlatforms: validateExcludedPlatforms(id, agent.excludedPlatforms),
  };
}

function validateExcludedPlatforms(id, value = []) {
  if (!Array.isArray(value)) throw new Error(`Agent ${id} excludedPlatforms must be an array.`);
  const seen = new Set();
  return value.map((platform) => {
    if (!platform || !operatingSystems.includes(platform.os) || !architectures.includes(platform.arch)) {
      throw new Error(`Agent ${id} has an invalid platform selector.`);
    }
    const key = `${platform.os}/${platform.arch}`;
    if (seen.has(key)) throw new Error(`Agent ${id} has duplicate platform selector ${key}.`);
    seen.add(key);
    return { os: platform.os, arch: platform.arch };
  });
}

function validateIconKey(id, iconKey) {
  if (!iconKey || iconKey === '.' || iconKey === '..' || iconKey.includes('/') || iconKey.includes('\\')) {
    throw new Error(`Agent ${id} has an invalid icon key.`);
  }
}

function resolveDistributionDefaults(distribution = {}) {
  if (distribution.npx) {
    return {
      command: 'npx',
      args: ['-y', distribution.npx.package, ...(distribution.npx.args ?? [])],
      env: distribution.npx.env ?? {},
    };
  }
  if (distribution.uvx) {
    return {
      command: 'uvx',
      args: [distribution.uvx.package, ...(distribution.uvx.args ?? [])],
      env: distribution.uvx.env ?? {},
    };
  }
  return { command: '', args: [], env: {} };
}

export class SimulatedPublicationCrash extends Error {
  constructor(message = 'Simulated Agent catalog publication crash.') {
    super(message);
    this.name = 'SimulatedPublicationCrash';
  }
}

async function fetchJson(url) {
  const response = await fetch(url, { headers: { accept: 'application/json' } });
  if (!response.ok) throw new Error(`Failed to fetch ${url}: HTTP ${response.status}`);
  return response.json();
}

async function fetchIcon(url) {
  const response = await fetch(url, { headers: { accept: 'image/svg+xml' } });
  if (!response.ok) throw new Error(`Failed to fetch icon ${url}: HTTP ${response.status}`);
  return response.text();
}

export async function generateAgentCatalog(options = {}) {
  const root = resolve(options.repoRoot ?? repoRoot);
  const paths = catalogPaths(root);
  await recoverAgentCatalogPublication(root);
  const offline = options.offline ?? false;
  const hooks = options.hooks ?? {};
  const policy = JSON.parse(await readFile(paths.policy, 'utf8'));
  const localRaw = await readFile(paths.localSupplement, 'utf8');
  const localSupplement = JSON.parse(localRaw);
  const localDigest = sha256(localRaw);

  await hooks.beforeRegistryFetch?.();
  const registry = offline
    ? JSON.parse(await readFile(paths.snapshot, 'utf8'))
    : await (options.fetchJson ?? fetchJson)(ACP_REGISTRY_URL);
  await hooks.afterRegistryFetch?.();
  const raw = `${JSON.stringify(registry, null, 2)}\n`;
  const fetchedAt = process.env.SOURCE_DATE_EPOCH
    ? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000).toISOString()
    : new Date().toISOString();
  const catalog = buildAgentCatalog(registry, fetchedAt, policy, localSupplement, localDigest);

  if (!offline) {
    for (const agent of catalog.agents) {
      if (!Object.hasOwn(policy.versionPins ?? {}, agent.id)) continue;
      const spec = npa(agent.args[1]);
      const url = `https://registry.npmjs.org/${encodeURIComponent(spec.name)}/${encodeURIComponent(spec.fetchSpec)}`;
      await hooks.beforeNpmPinValidation?.();
      const metadata = await (options.fetchJson ?? fetchJson)(url);
      if (!metadata || metadata.version !== spec.fetchSpec) {
        throw new Error(`Pinned package ${agent.args[1]} validation failed.`);
      }
    }
  }

  const staging = await mkdtemp(join(tmpdir(), 'gold-band-agent-catalog-'));
  try {
    await mkdir(staging, { recursive: true });
    await writeFile(join(staging, 'snapshot.json'), raw, 'utf8');
    await writeFile(join(staging, 'agent-catalog.json'), `${JSON.stringify(catalog, null, 2)}\n`, 'utf8');
    await stageIcons(root, staging, catalog, offline, options.fetchIcon ?? fetchIcon, hooks);
    await hooks.beforeCatalogValidation?.();
    const staged = await validateStagedCatalog(staging, catalog);
    await publishStagedCatalog(root, staged, hooks);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
  return catalog;
}

function catalogPaths(root) {
  return {
    snapshot: join(root, 'resources', 'acp-registry.snapshot.json'),
    catalog: join(root, 'resources', 'agent-catalog.json'),
    policy: join(root, 'configs', 'agent-catalog-policy.json'),
    localSupplement: join(root, LOCAL_SUPPLEMENT_RESOURCE_PATH),
    icons: join(root, 'web', 'public', 'agent-icons'),
    journal: join(root, 'resources', '.agent-catalog-publish-journal.json'),
  };
}

function relativeTarget(root, target) {
  const value = relative(root, target);
  if (!value || value.startsWith('..') || isAbsolute(value)) {
    throw new Error(`Agent catalog target escapes repository: ${target}`);
  }
  return value.split(sep).join('/');
}

async function stageIcons(root, staging, catalog, offline, fetchIconImpl, hooks) {
  const sourceDir = join(root, 'web', 'public', 'agent-icons');
  const targetDir = join(staging, 'icons');
  await mkdir(targetDir, { recursive: true });
  const iconKeys = new Set();
  for (const agent of catalog.agents) {
    if (iconKeys.has(agent.iconKey)) throw new Error(`Duplicate Agent icon key: ${agent.iconKey}`);
    iconKeys.add(agent.iconKey);
    if (!agent.iconUrl?.startsWith('https://')) {
      throw new Error(`Agent ${agent.id} does not provide an HTTPS icon URL.`);
    }
    const svg = offline
      ? await readFile(join(sourceDir, `${agent.iconKey}.svg`), 'utf8')
      : await (async () => {
        await hooks.beforeIconDownload?.();
        return fetchIconImpl(agent.iconUrl);
      })();
    await hooks.beforeIconValidation?.();
    const trimmed = svg.trimStart();
    const svgDocument = trimmed.startsWith('<svg')
      || trimmed.startsWith('<?xml')
      || trimmed.startsWith('<!DOCTYPE svg');
    if (!svgDocument || !svg.includes('<svg')) throw new Error(`Agent ${agent.id} icon is not SVG.`);
    await writeFile(join(targetDir, `${agent.iconKey}.svg`), svg, 'utf8');
  }
}

function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

async function fileDigest(path) {
  return sha256(await readFile(path));
}

async function validateStagedCatalog(staging, catalog) {
  if (catalog.schemaVersion !== 2) throw new Error('Agent catalog schema version must be 2.');
  if (catalog.agents.length !== BUILTIN_AGENT_IDS.length) {
    throw new Error('Agent catalog has an unexpected size.');
  }
  if (!catalog.source?.officialRegistry?.url || !catalog.source?.localSupplement?.sha256) {
    throw new Error('Agent catalog source is incomplete.');
  }
  const targets = new Map([
    ['resources/acp-registry.snapshot.json', join(staging, 'snapshot.json')],
    ['resources/agent-catalog.json', join(staging, 'agent-catalog.json')],
  ]);
  const iconKeys = new Set();
  for (const agent of catalog.agents) {
    if (!BUILTIN_AGENT_IDS.includes(agent.id)) throw new Error(`Uncurated Agent in catalog: ${agent.id}`);
    if (iconKeys.has(agent.iconKey)) throw new Error(`Duplicate Agent icon key: ${agent.iconKey}`);
    iconKeys.add(agent.iconKey);
    targets.set(`web/public/agent-icons/${agent.iconKey}.svg`, join(staging, 'icons', `${agent.iconKey}.svg`));
  }
  const digests = new Map();
  for (const [relativePath, stagedPath] of targets) {
    digests.set(relativePath, await fileDigest(stagedPath));
  }
  const catalogText = await readFile(targets.get('resources/agent-catalog.json'), 'utf8');
  const actualCatalog = JSON.parse(catalogText);
  if (sha256(catalogText) !== digests.get('resources/agent-catalog.json')
      || JSON.stringify(actualCatalog) !== JSON.stringify(catalog)) {
    throw new Error('Staged Agent catalog does not match the built catalog.');
  }
  return { targets, digests };
}

async function publishStagedCatalog(root, staged, hooks) {
  const paths = catalogPaths(root);
  const generated = new Map([
    ['resources/acp-registry.snapshot.json', paths.snapshot],
    ['resources/agent-catalog.json', paths.catalog],
  ]);
  for (const [relativePath, stagedPath] of staged.targets) {
    if (relativePath.startsWith('web/public/agent-icons/')) {
      if (stagedPath) generated.set(relativePath, resolve(root, relativePath));
    } else if (!generated.has(relativePath)) {
      throw new Error(`Unexpected staged target: ${relativePath}`);
    }
  }
  await mkdir(paths.icons, { recursive: true });
  const previousManagedIcons = new Set();
  if (await fileExists(paths.catalog)) {
    const previousCatalog = JSON.parse(await readFile(paths.catalog, 'utf8'));
    if (!Array.isArray(previousCatalog.agents)) {
      throw new Error('Existing Agent catalog does not contain an agents array.');
    }
    for (const agent of previousCatalog.agents) {
      previousManagedIcons.add(`web/public/agent-icons/${agent.iconKey}.svg`);
    }
  }
  const existingIcons = (await readdir(paths.icons, { withFileTypes: true }))
    .filter((entry) => entry.isFile() && entry.name.endsWith('.svg'))
    .map((entry) => `web/public/agent-icons/${entry.name}`);
  const staleIcons = existingIcons.filter(
    (relativePath) => previousManagedIcons.has(relativePath) && !generated.has(relativePath),
  );
  const records = [];
  const token = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 10)}`;
  for (const relativePath of [...generated.keys(), ...staleIcons]) {
    const target = resolve(root, relativePath);
    await mkdir(dirname(target), { recursive: true });
    const exists = await fileExists(target);
    const operation = generated.has(relativePath) ? 'publish' : 'remove';
    const backup = exists ? `${target}.catalog-backup-${token}` : null;
    if (backup) await copyFile(target, backup);
    const temp = operation === 'publish' ? `${target}.catalog-temp-${token}` : null;
    if (temp) await copyFile(staged.targets.get(relativePath), temp);
    records.push({
      target: relativeTarget(root, target),
      operation,
      temp: temp && relativeTarget(root, temp),
      backup: backup && relativeTarget(root, backup),
      originalDigest: exists ? await fileDigest(target) : null,
      newDigest: operation === 'publish' ? staged.digests.get(relativePath) : null,
    });
  }

  let journalReady = false;
  try {
    await writeAtomicFile(paths.journal, `${JSON.stringify({ schemaVersion: 1, records }, null, 2)}\n`);
    journalReady = true;
    for (const record of records) {
      await hooks.beforeReplace?.(record);
      const target = resolve(root, record.target);
      if (record.operation === 'publish') {
        await rename(resolve(root, record.temp), target);
      } else {
        await rm(target, { force: true });
      }
      await hooks.afterReplace?.(record);
    }
    for (const record of records) {
      if (record.backup) await rm(resolve(root, record.backup), { force: true });
    }
    await rm(paths.journal, { force: true });
  } catch (error) {
    if (error instanceof SimulatedPublicationCrash) throw error;
    if (journalReady) await recoverAgentCatalogPublication(root);
    else await cleanupRecords(root, records);
    throw error;
  } finally {
    await cleanupRecords(root, records, { tempsOnly: true });
  }
}

async function cleanupRecords(root, records, { tempsOnly = false } = {}) {
  for (const record of records) {
    if (record.temp) await rm(resolve(root, record.temp), { force: true });
    if (!tempsOnly && record.backup) await rm(resolve(root, record.backup), { force: true });
  }
}

async function fileExists(path) {
  try {
    await access(path);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') return false;
    throw error;
  }
}

async function writeAtomicFile(target, content) {
  const temp = `${target}.tmp-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  await mkdir(dirname(target), { recursive: true });
  try {
    await writeFile(temp, content, 'utf8');
    await rename(temp, target);
  } catch (error) {
    await rm(temp, { force: true });
    throw error;
  }
}

export async function recoverAgentCatalogPublication(root) {
  const paths = catalogPaths(root);
  if (!await fileExists(paths.journal)) return false;
  const journal = JSON.parse(await readFile(paths.journal, 'utf8'));
  if (journal.schemaVersion !== 1 || !Array.isArray(journal.records)) {
    throw new Error('Invalid Agent catalog recovery journal.');
  }
  for (const record of journal.records) {
    if (!record.target || !['publish', 'remove'].includes(record.operation)) {
      throw new Error('Invalid Agent catalog recovery record.');
    }
    const target = resolve(root, record.target);
    const targetExists = await fileExists(target);
    const targetDigest = targetExists ? await fileDigest(target) : null;
    if (targetDigest === record.newDigest) continue;
    if (targetDigest === record.originalDigest) continue;
    if (record.backup) {
      const backup = resolve(root, record.backup);
      if (await fileDigest(backup) !== record.originalDigest) {
        throw new Error(`Agent catalog backup digest mismatch: ${record.target}`);
      }
      if (targetExists) await rm(target, { force: true });
      await rename(backup, target);
    } else if (targetExists) {
      await rm(target, { force: true });
    } else {
      throw new Error(`Agent catalog recovery source is missing: ${record.target}`);
    }
    if (record.temp) await rm(resolve(root, record.temp), { force: true });
  }
  for (const record of journal.records) {
    if (record.temp) await rm(resolve(root, record.temp), { force: true });
    if (record.backup) await rm(resolve(root, record.backup), { force: true });
  }
  await rm(paths.journal, { force: true });
  return true;
}

async function main() {
  const offline = process.argv.includes('--offline');
  const catalog = await generateAgentCatalog({ offline });
  const digest = sha256(JSON.stringify(catalog, null, 2));
  console.log(`Prepared ${catalog.agents.length} Agent templates from ACP Registry ${catalog.source.officialRegistry.registryVersion} (${digest.slice(0, 12)}).`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
