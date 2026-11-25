import { execFileSync } from 'node:child_process';
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readInternalDomain } from './channel-config.mjs';

export function findDomainReferences(files, hostnames) {
  const needles = hostnames.map((name) => name.toLowerCase());
  return files.filter(({ content }) => {
    const text = content.toString('latin1').toLowerCase();
    return needles.some((needle) => text.includes(needle));
  }).map(({ path }) => path);
}

export function scanTrackedFiles(root, hostnames) {
  const paths = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], { cwd: root })
    .toString('utf8').split('\0').filter(Boolean);
  const matches = [];
  for (const path of new Set(paths)) {
    try {
      const absolute = resolve(root, path);
      if (statSync(absolute).isFile()) matches.push(...findDomainReferences([{ path, content: readFileSync(absolute) }], hostnames));
    } catch (error) {
      if (error.code !== 'ENOENT') throw error; // A tracked file may have been deleted in the working tree.
    }
  }
  return matches;
}

function hostnameFromEnv(name) {
  const value = process.env[name];
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) throw new Error();
    return url.hostname;
  } catch {
    throw new Error(`Missing or invalid ${name}.`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const hostnames = [
      readInternalDomain(process.env),
      hostnameFromEnv('GOLD_BAND_WB_BASE_URL'),
      hostnameFromEnv('GOLD_BAND_WB_MULTICA_URL'),
    ];
    const matches = scanTrackedFiles(process.cwd(), hostnames);
    if (matches.length) throw new Error(`WB service domain is present in repository files:\n${matches.join('\n')}`);
    console.log('WB service domains are absent from repository files.');
  } catch (error) {
    console.error(error.message || 'Invalid WB service origin.');
    process.exitCode = 1;
  }
}
