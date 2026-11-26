import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseEnv } from 'node:util';

import { repoRoot } from './channel-config.mjs';

export const LOCAL_ENV_FILENAME = '.env';

/**
 * 把仓库根 `.env` 中的变量合并进目标环境对象。
 *
 * 语义与 Node 内置 `process.loadEnvFile` 对齐：已存在的键优先，命令行显式导出的
 * 变量或 CI Secret 不会被文件悄悄覆盖。`.env` 缺失时静默跳过——default 渠道的
 * 本地开发不需要任何 WB 配置，缺失不应打断启动。
 */
export function loadLocalEnv(env, root = repoRoot) {
  const path = join(root, LOCAL_ENV_FILENAME);
  if (!existsSync(path)) {
    return env;
  }
  const parsed = parseEnv(readFileSync(path, 'utf8'));
  for (const [key, value] of Object.entries(parsed)) {
    if (env[key] === undefined) {
      env[key] = value;
    }
  }
  return env;
}
