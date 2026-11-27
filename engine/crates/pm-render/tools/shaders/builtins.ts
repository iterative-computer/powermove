/*
 * Writes `engine/crates/pm-render/src/shader_layers/builtins.json`: what the
 * engine needs to render shader layers and the built-in extension layers
 * without a kernel (plan sections 10 and 11):
 *
 * - `header`: `PM.SHADER_HEADER` (`legacy/gl/shaders.ts`), which the
 *   compositor prepends to a shader layer's `d.code` (`header + '\n' + code`);
 * - `missing`: the checkerboard body the compositor draws for an extension
 *   layer whose definition is missing (`compositor.ts`, `const missing`);
 * - `template` and `presets`: `PM.SHADER_TEMPLATE` and the five
 *   `PM.SHADER_PRESETS` (user GLSL copied into `d.code`; the engine never
 *   reads them while rendering, the shader gate does);
 * - `layers`: the built-in extension layer definitions as the kernel
 *   snapshots them (contract 10): Studio Cube (a fragment layer) and the OBJ
 *   model layer (a mesh layer).
 *
 * The vitest `kernel/shader-layer-builtins.test.ts` fails when the file
 * drifts from the app.
 *
 * Usage: bun engine/crates/pm-render/tools/shaders/builtins.ts [--check]
 */
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { builtinLayerFile } from './sources';
import { REPO } from '../effects/preambles';

const OUT = join(REPO, 'engine', 'crates', 'pm-render', 'src', 'shader_layers', 'builtins.json');
const text = JSON.stringify(await builtinLayerFile(), null, 1) + '\n';
if (process.argv.includes('--check')) {
  const current = await readFile(OUT, 'utf8').catch(() => '');
  if (current !== text) {
    console.error(`${OUT} is stale; run bun engine/crates/pm-render/tools/shaders/builtins.ts`);
    process.exit(1);
  }
} else {
  await writeFile(OUT, text);
  console.log(`wrote ${OUT}`);
}
