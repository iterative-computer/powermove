/*
 * Writes `engine/crates/pm-render/src/effects/builtins.json`: the 16
 * built-in effects and 4 built-in transitions as the kernel snapshots them
 * (contract 10), plus the compositor's vertex shader. pm-render embeds it so
 * `pm-cli` and tests can render built-ins without a kernel; the vitest
 * `kernel/definition-snapshots.test.ts` fails when it drifts from the kernel.
 *
 * Usage: bun engine/crates/pm-render/tools/effects/builtins.ts [--check]
 */
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { REPO, VERTEX, builtinSnapshots } from './preambles';

const OUT = join(REPO, 'engine', 'crates', 'pm-render', 'src', 'effects', 'builtins.json');
const text = JSON.stringify({ vertex: VERTEX, definitions: builtinSnapshots() }, null, 1) + '\n';
if (process.argv.includes('--check')) {
  const current = await readFile(OUT, 'utf8').catch(() => '');
  if (current !== text) {
    console.error(`${OUT} is stale; run bun engine/crates/pm-render/tools/effects/builtins.ts`);
    process.exit(1);
  }
} else {
  await writeFile(OUT, text);
  console.log(`wrote ${OUT}`);
}
