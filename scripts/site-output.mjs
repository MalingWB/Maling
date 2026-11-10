import { readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Clear a published output directory without deleting the directory itself.
 * Windows tools (Explorer, terminals, editors) can hold the root as a CWD or
 * directory handle; retaining the root keeps website builds usable in that case.
 */
export function clearDirectoryContents(directory) {
  let entries;
  try {
    entries = readdirSync(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === 'ENOENT') return;
    throw { code: 'site.output-root-invalid', params: { directory, reason: error.code ?? 'unknown' } };
  }
  for (const entry of entries) {
    rmSync(join(directory, entry.name), {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 100,
    });
  }
}