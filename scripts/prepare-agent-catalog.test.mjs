import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import {
  BUILTIN_AGENT_IDS, SimulatedPublicationCrash, buildAgentCatalog, generateAgentCatalog,
  recoverAgentCatalogPublication,
} from './prepare-agent-catalog.mjs';

function registryFixture() {
  return { agents: BUILTIN_AGENT_IDS.map((id) => ({
    id, version: '0.73.0',
    name: id,
    description: id,
    icon: `https://icons.example/${id}.svg`,
    distribution: { npx: { package: `@example/${id}@0.73.0`, args: ['--acp'] } },
  })) };
}

function officialPolicy(extra = {}) {
  return {
    ...extra,
    validatedVersions: Object.fromEntries(BUILTIN_AGENT_IDS.map((id) => [id, '0.73.0'])),
  };
}

test('local pins override both executable version and metadata without changing registry', () => {
  const registry = registryFixture();
  const before = structuredClone(registry);
  const catalog = buildAgentCatalog(registry, 'fixed', officialPolicy({
    versionPins: { 'claude-acp': '0.72.0' },
  }));
  const claude = catalog.agents.find((agent) => agent.id === 'claude-acp');
  assert.equal(claude.version, '0.72.0');
  assert.deepEqual(claude.provenance, {
    sourceKind: 'official-registry',
    integrationOrigin: 'official',
    validatedVersion: '0.73.0',
  });
  assert.deepEqual(claude.args, ['-y', '@example/claude-acp@0.72.0', '--acp']);
  assert.equal(catalog.agents.find((agent) => agent.id === 'codex-acp').version, '0.73.0');
  assert.deepEqual(registry, before);
});

test('invalid pins fail explicitly instead of silently following registry', () => {
  for (const version of [-1, '', 'latest', '^0.72.0', '0.72', null]) {
    assert.throws(() => buildAgentCatalog(registryFixture(), 'fixed', officialPolicy({
      versionPins: { 'claude-acp': version },
    })), /version/i);
  }
  assert.throws(() => buildAgentCatalog(registryFixture(), 'fixed', officialPolicy({
    versionPins: { typo: '0.72.0' },
  })), /unknown/i);
  assert.throws(() => buildAgentCatalog(registryFixture(), 'fixed', officialPolicy({
    versionPins: { cursor: '0.72.0' },
  })), /npx/i);
});

test('pins support unscoped packages and preserve arguments and environment', () => {
  const registry = registryFixture();
  const agent = registry.agents.find((entry) => entry.id === 'pi-acp');
  agent.distribution.npx = { package: 'pi-acp@latest', args: ['--acp'], env: { EXAMPLE: 'value' } };
  const result = buildAgentCatalog(registry, 'fixed', officialPolicy({
    versionPins: { 'pi-acp': '1.0.0-beta.1' },
  }))
    .agents.find((entry) => entry.id === 'pi-acp');
  assert.deepEqual(result.args, ['-y', 'pi-acp@1.0.0-beta.1', '--acp']);
  assert.deepEqual(result.env, { EXAMPLE: 'value' });
});

test('policy rejects malformed maps and non-registry distributions', () => {
  for (const policy of [null, [], { versionPin: {} }, { versionPins: null }, { versionPins: [] }]) {
      const candidate = policy && !Array.isArray(policy) ? officialPolicy(policy) : policy;
      assert.throws(() => buildAgentCatalog(registryFixture(), 'fixed', candidate));
  }
  const registry = registryFixture();
  registry.agents[0].distribution = {};
  assert.throws(() => buildAgentCatalog(registry, 'fixed', officialPolicy({
    versionPins: { 'claude-acp': '0.72.0' },
  })), /npx/);
  registry.agents[0].distribution = { npx: { package: 'https://example.com/agent.tgz' } };
  assert.throws(() => buildAgentCatalog(registry, 'fixed', officialPolicy({
    versionPins: { 'claude-acp': '0.72.0' },
  })), /registry package/);
});

test('checked-in policy applies to the snapshot and future online versions', async () => {
  const policy = JSON.parse(await readFile(new URL('../configs/agent-catalog-policy.json', import.meta.url), 'utf8'));
  assert.deepEqual(policy.versionPins, { 'claude-acp': '0.90.0', 'codex-acp': '2.1.1' });
  assert.equal(policy.validatedVersions.gemini, '0.66.0');
  assert.equal(policy.validatedVersions['qwen-code'], '0.26.6');
  const registry = JSON.parse(await readFile(new URL('../resources/acp-registry.snapshot.json', import.meta.url), 'utf8'));
  for (const version of ['0.73.0', '0.75.1']) {
    const claude = registry.agents.find((entry) => entry.id === 'claude-acp');
    claude.version = version;
    claude.distribution.npx.package = `@agentclientprotocol/claude-agent-acp@${version}`;
    const codex = registry.agents.find((entry) => entry.id === 'codex-acp');
    codex.version = version;
    codex.distribution.npx.package = `@agentclientprotocol/codex-acp@${version}`;
    const catalog = buildAgentCatalog(registry, 'fixed', policy);
    for (const agent of registry.agents.filter((entry) => BUILTIN_AGENT_IDS.includes(entry.id))) {
      const result = catalog.agents.find((entry) => entry.id === agent.id);
      const expected = policy.versionPins?.[agent.id] ?? agent.version;
      assert.equal(result.version, expected, agent.id);
      if (agent.distribution?.npx && result.command === 'npx') {
        assert.ok(result.args[1].endsWith(`@${expected}`), `${agent.id}: ${result.args[1]}`);
      }
    }
  }
});

test('filters the official registry to the curated catalog without GLM', () => {
  const agents = BUILTIN_AGENT_IDS.map((id) => ({
    id,
    name: id,
    version: '1.0.0',
    description: id,
    icon: `https://example.com/${id}.svg`,
    distribution: { npx: { package: `${id}@1.0.0`, args: ['--acp'] } },
  }));
  const catalog = buildAgentCatalog({ version: '1.0.0', agents }, '2026-08-07T00:00:00.000Z', officialPolicy());

  assert.deepEqual(catalog.agents.map((agent) => agent.id), BUILTIN_AGENT_IDS);
  assert.equal(catalog.agents.some((agent) => agent.id.includes('glm')), false);
  assert.equal(catalog.agents.find((agent) => agent.id === 'amp-acp').primaryAgentDir, '.agents');
  assert.deepEqual(catalog.agents.find((agent) => agent.id === 'amp-acp').compatibleAgentDirs, ['.claude']);
  assert.equal(catalog.agents.find((agent) => agent.id === 'claude-acp').supportsSystemPrompt, true);
  const kimi = catalog.agents.find((agent) => agent.id === 'kimi');
  assert.equal(kimi.primaryAgentDir, '.kimi-code');
  assert.deepEqual(kimi.compatibleAgentDirs, ['.agents']);
  assert.equal(kimi.supportsSystemPrompt, false);
  const pi = catalog.agents.find((agent) => agent.id === 'pi-acp');
  assert.equal(pi.command, 'npx');
  assert.deepEqual(pi.args, ['-y', 'pi-acp@1.0.0', '--acp']);
  assert.equal(pi.primaryAgentDir, '.pi/agent');
  assert.equal(pi.projectPrimaryAgentDir, '.pi');
  assert.deepEqual(pi.compatibleAgentDirs, ['.agents']);
  assert.equal(pi.supportsSystemPrompt, false);
  assert.equal(pi.supportsExternalSessionSync, false);
  assert.equal(catalog.schemaVersion, 2);
  const claude = catalog.agents.find((agent) => agent.id === 'claude-acp');
  assert.equal(claude.version, '1.0.0');
  assert.equal(claude.provenance.validatedVersion, '0.73.0');
  assert.equal(catalog.source.officialRegistry.url, 'https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json');
  assert.match(catalog.source.localSupplement.sha256, /^[a-f0-9]{64}$/);
  assert.deepEqual(catalog.agents.map((agent) => agent.excludedPlatforms), BUILTIN_AGENT_IDS.map(() => []));
});

test('official registry versions cannot silently become validation evidence', () => {
  const invalid = officialPolicy();
  invalid.validatedVersions['claude-acp'] = null;
  assert.throws(
    () => buildAgentCatalog(registryFixture(), 'fixed', invalid),
    /validated version/,
  );
});

test('local supplement entries carry provenance and typed platform exclusions', () => {
  const registry = registryFixture();
  registry.agents = registry.agents.filter((agent) => agent.id !== 'pi-acp');
  const local = {
    agents: [{
      id: 'pi-acp',
      label: 'Local Pi',
      version: '0.2.1-rc.2',
      provenance: {
        sourceKind: 'local-supplement',
        integrationOrigin: 'official',
        validatedVersion: '0.2.1-rc.2',
      },
      description: 'fixture',
      iconUrl: 'https://icons.example/deepseek-harness.svg',
      command: 'pi-acp-local',
      args: ['--acp'],
      primaryAgentDir: '.pi/agent',
      compatibleAgentDirs: ['.agents'],
    }],
  };
  const digest = createHash('sha256').update(JSON.stringify(local)).digest('hex');
  const catalog = buildAgentCatalog(registry, 'fixed', officialPolicy(), local, digest);
  const agent = catalog.agents.find((entry) => entry.id === 'pi-acp');

  assert.equal(agent.provenance.sourceKind, 'local-supplement');
  assert.equal(agent.provenance.validatedVersion, '0.2.1-rc.2');
  assert.equal(catalog.source.localSupplement.sha256, digest);
  assert.throws(() => buildAgentCatalog(registry, 'fixed', officialPolicy(), {
    agents: [{ ...local.agents[0], excludedPlatforms: [{ os: 'win32', arch: 'arm64' }] }],
  }, digest), /invalid platform selector/);
  assert.throws(() => buildAgentCatalog(registryFixture(), 'fixed', officialPolicy(), {
    agents: [{ ...local.agents[0], id: 'claude-acp' }],
  }, digest), /both official Registry and local supplement/);
});

test('fails rather than silently publishing an incomplete catalog', () => {
  assert.throws(
    () => buildAgentCatalog({ version: '1.0.0', agents: [] }),
    /missing required agents/,
  );
});

test('catalog describes installed ACP commands independently of npm launchers', () => {
  const catalog = buildAgentCatalog(registryFixture(), 'fixed', officialPolicy());
  for (const [id, command] of [
    ['claude-acp', 'claude-agent-acp'], ['codex-acp', 'codex-acp'],
    ['gemini', 'gemini'], ['codebuddy-code', 'codebuddy'],
    ['qwen-code', 'qwen'], ['pi-acp', 'pi-acp'],
  ]) {
    const agent = catalog.agents.find((entry) => entry.id === id);
    assert.deepEqual(agent.localLaunch, { command, args: ['--acp'] });
  }
  assert.equal(catalog.agents.find((entry) => entry.id === 'claude-acp').installedCommand, 'claude');
  assert.equal(catalog.agents.find((entry) => entry.id === 'codex-acp').installedCommand, 'codex');
  assert.equal(catalog.agents.find((entry) => entry.id === 'pi-acp').installedCommand, 'pi');
  assert.equal(catalog.agents.find((entry) => entry.id === 'cursor').localLaunch, null);
});

async function createGenerationRoot(policy = officialPolicy()) {
  const root = await mkdtemp(join(tmpdir(), 'gold-band-agent-catalog-test-'));
  await mkdir(join(root, 'resources'), { recursive: true });
  await mkdir(join(root, 'configs'), { recursive: true });
  await writeFile(join(root, 'configs', 'agent-catalog-policy.json'), JSON.stringify(policy));
  await writeFile(join(root, 'resources', 'acp-registry-local.json'), '{"agents":[]}');
  return root;
}

function generationFetchers() {
  return {
    fetchJson: async (url) => {
      if (url.endsWith('/registry.json')) return registryFixture();
      const version = decodeURIComponent(url.split('/').pop());
      return { version };
    },
    fetchIcon: async (url) => `<svg xmlns="https://www.w3.org/2000/svg">${url}</svg>`,
  };
}

async function officialState(root) {
  const iconDir = join(root, 'web', 'public', 'agent-icons');
  const iconNames = (await readdir(iconDir)).filter((name) => name.endsWith('.svg')).sort();
  return {
    snapshot: await readFile(join(root, 'resources', 'acp-registry.snapshot.json'), 'utf8'),
    catalog: await readFile(join(root, 'resources', 'agent-catalog.json'), 'utf8'),
    icons: await Promise.all(iconNames.map(async (name) => [
      name,
      await readFile(join(iconDir, name), 'utf8'),
    ])),
  };
}

async function initializeGenerationRoot(policy = {}) {
  const root = await createGenerationRoot(policy);
  await generateAgentCatalog({ repoRoot: root, ...generationFetchers() });
  return root;
}

test('catalog publication removes only icons managed by the previous catalog', async () => {
  const root = await initializeGenerationRoot();
  try {
    const stale = join(root, 'web', 'public', 'agent-icons', 'stale.svg');
    await writeFile(stale, '<svg>stale</svg>');
    const catalogPath = join(root, 'resources', 'agent-catalog.json');
    const previousCatalog = JSON.parse(await readFile(catalogPath, 'utf8'));
    const claude = previousCatalog.agents.find((agent) => agent.id === 'claude-acp');
    claude.iconKey = 'claude-legacy';
    await writeFile(catalogPath, `${JSON.stringify(previousCatalog, null, 2)}\n`);
    await rename(join(root, 'web', 'public', 'agent-icons', 'claude.svg'), join(root, 'web', 'public', 'agent-icons', 'claude-legacy.svg'));
    await generateAgentCatalog({ repoRoot: root, ...generationFetchers() });
    assert.equal((await readdir(join(root, 'web', 'public', 'agent-icons'))).includes('stale.svg'), true);
    assert.equal((await readdir(join(root, 'web', 'public', 'agent-icons'))).includes('claude-legacy.svg'), false);
    assert.equal((await officialState(root)).icons.length, BUILTIN_AGENT_IDS.length + 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('ordinary catalog generation failures leave official outputs byte-for-byte unchanged', async () => {
  const scenarios = [
    ['registry-fetch', { fetchJson: async () => { throw new Error('offline'); } }],
    ['npm-pin-validation', { hooks: { beforeNpmPinValidation: () => { throw new Error('pin unavailable'); } } }],
    ['icon-download', { fetchIcon: async () => { throw new Error('icon unavailable'); } }],
    ['icon-validation', { hooks: { beforeIconValidation: () => { throw new Error('invalid icon'); } } }],
    ['catalog-validation', { hooks: { beforeCatalogValidation: () => { throw new Error('invalid catalog'); } } }],
    ['official-replace', { hooks: { beforeReplace: () => { throw new Error('replace failed'); } } }],
  ];
  for (const [name, options] of scenarios) {
    const policy = name === 'npm-pin-validation' ? officialPolicy({
      versionPins: { 'claude-acp': '0.72.0' },
    }) : officialPolicy();
    const root = await initializeGenerationRoot(policy);
    try {
      const before = await officialState(root);
      await assert.rejects(
        () => generateAgentCatalog({ repoRoot: root, ...generationFetchers(), ...options }),
        /offline|unavailable|failed|invalid/,
        name,
      );
      assert.deepEqual(await officialState(root), before, name);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});

test('interrupted catalog publication recovers to the complete previous set', async () => {
  const root = await initializeGenerationRoot();
  try {
    const before = await officialState(root);
    await assert.rejects(
      () => generateAgentCatalog({
        repoRoot: root,
        ...generationFetchers(),
        fetchIcon: async (url) => `<svg xmlns="https://www.w3.org/2000/svg">changed ${url}</svg>`,
        hooks: {
          afterReplace: () => {
            throw new SimulatedPublicationCrash();
          },
        },
      }),
      SimulatedPublicationCrash,
    );
    assert.equal((await readdir(join(root, 'resources'))).includes('.agent-catalog-publish-journal.json'), true);
    await recoverAgentCatalogPublication(root);
    assert.deepEqual(await officialState(root), before);
    assert.equal((await readdir(join(root, 'resources'))).includes('.agent-catalog-publish-journal.json'), false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('interrupted catalog publication can complete to the consistent new set', async () => {
  const root = await initializeGenerationRoot();
  try {
    await assert.rejects(
      () => generateAgentCatalog({
        repoRoot: root,
        ...generationFetchers(),
        fetchIcon: async (url) => `<svg xmlns="https://www.w3.org/2000/svg">changed ${url}</svg>`,
        hooks: {
          afterReplace: (record) => {
            if (record.target.endsWith('pi-acp.svg')) throw new SimulatedPublicationCrash();
          },
        },
      }),
      SimulatedPublicationCrash,
    );
    const afterCrash = await officialState(root);
    const resources = await readdir(join(root, 'resources'), { withFileTypes: true });
    for (const entry of resources.filter((entry) => entry.name.includes('.catalog-backup-'))) {
      await rm(join(root, 'resources', entry.name), { force: true });
    }
    const icons = await readdir(join(root, 'web', 'public', 'agent-icons'), { withFileTypes: true });
    for (const entry of icons.filter((entry) => entry.name.includes('.catalog-backup-'))) {
      await rm(join(root, 'web', 'public', 'agent-icons', entry.name), { force: true });
    }
    await recoverAgentCatalogPublication(root);
    assert.deepEqual(await officialState(root), afterCrash);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
