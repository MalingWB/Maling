import { execFileSync } from 'node:child_process';
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function findDomainReferences(files, hostname) {
  const needle = Buffer.from(hostname.toLowerCase());
  return files.filter(({ content }) => content.toString('latin1').toLowerCase().includes(needle.toString('latin1')))
    .map(({ path }) => path);
}

export function scanTrackedFiles(root, hostname) {
  const paths = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], { cwd: root })
    .toString('utf8').split('\0').filter(Boolean);
  const matches = [];
  for (const path of new Set(paths)) {
    try {
      const absolute = resolve(root, path);
      if (statSync(absolute).isFile()) matches.push(...findDomainReferences([{ path, content: readFileSync(absolute) }], hostname));
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
    const hostnames = new Set([
      hostnameFromEnv('GOLD_BAND_WB_BASE_URL'),
      hostnameFromEnv('GOLD_BAND_WB_MULTICA_URL'),
    ]);
    const matches = [...new Set([...hostnames].flatMap((hostname) => scanTrackedFiles(process.cwd(), hostname)))];
    if (matches.length) throw new Error(`WB service domain is present in repository files:\n${matches.join('\n')}`);
    console.log('WB service domains are absent from repository files.');
  } catch (error) {
    console.error(error.message || 'Invalid WB service origin.');
    process.exitCode = 1;
  }
}
