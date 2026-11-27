import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * The Electron app's tree. It ships from `main` and is not on this branch, so
 * tests and the local-dev seeder that read its built-ins or its extension
 * compiler take it from a v1 checkout. Point `PM_V1_REF` at that checkout's
 * root (default `/tmp/pm-ref-v1.2.0`, the same default the `tools/` harnesses
 * use); `apps/desktop` lives under it.
 */
export const v1Desktop = resolve(process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0', 'apps/desktop');

/** Why the v1 fixture can't be used here (missing, or without its installed dependencies), or null when it can. */
export function v1Missing(needs: { deps?: boolean; paths?: string[] } = {}): string | null {
  const root = resolve(v1Desktop, '../..');
  if (!existsSync(v1Desktop)) return `no v1 checkout at ${root} (set PM_V1_REF)`;
  if (needs.deps && !existsSync(resolve(root, 'node_modules/@powermove/registry'))) return `the v1 checkout at ${root} has no installed dependencies (bun install there)`;
  for (const path of needs.paths ?? []) if (!existsSync(resolve(v1Desktop, path))) return `the v1 checkout has no ${path}`;
  return null;
}
