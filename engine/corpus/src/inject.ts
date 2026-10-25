/**
 * Pure legacy modules the capture loads into the renderer beside the app's
 * own bundle. The renderer does not expose these functions on `PM`; bundling
 * the same source files from the same commit gives the capture the code the
 * compositor runs, rather than hand-written copies.
 *
 * - `legacy/core/text-animation.ts`: `textAnimationState` and `animatedGlyphs`,
 *   which `legacy/gl/raster.ts` calls to pose each glyph of an animated or
 *   range-styled text layer.
 * - `legacy/core/retiming.ts`: `sourceTime`, the one source-time mapping for
 *   video, precomps, and audio.
 *
 * Every function takes `PM` as an argument, so the bundles run against the
 * live app without sharing module state with it.
 */
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { rendererSrc } from './catalog';

export const LEGACY_GLOBAL = '__corpusLegacy';
export const TEXT_ANIMATION_MODULE = path.join(rendererSrc, 'legacy/core/text-animation.ts');
export const RETIMING_MODULE = path.join(rendererSrc, 'legacy/core/retiming.ts');
/** Upper bound on the bundle: keep renderer and media modules out. */
const MAX_BUNDLE_BYTES = 64 * 1024;

/** A single JavaScript expression that installs `globalThis.__corpusLegacy` and evaluates to true. */
export async function legacyScript(): Promise<string> {
  const dir = mkdtempSync(path.join(tmpdir(), 'powermove-engine-corpus-'));
  try {
    const entry = path.join(dir, 'legacy-entry.ts');
    writeFileSync(entry, [
      `import { animatedGlyphs, textAnimationState } from ${JSON.stringify(TEXT_ANIMATION_MODULE)};`,
      `import { sourceTime } from ${JSON.stringify(RETIMING_MODULE)};`,
      `(globalThis as any).${LEGACY_GLOBAL} = { animatedGlyphs, textAnimationState, sourceTime };`,
      '',
    ].join('\n'));
    const result = await Bun.build({ entrypoints: [entry], target: 'browser', format: 'iife', minify: false, sourcemap: 'none' });
    if (!result.success || result.outputs.length !== 1) {
      throw new Error(`Bundling the legacy helpers failed:\n${result.logs.map(log => String(log)).join('\n')}`);
    }
    const code = await result.outputs[0]!.text();
    if (code.length > MAX_BUNDLE_BYTES) throw new Error(`The legacy helper bundle is ${code.length} bytes; an import pulled in a renderer or media module`);
    const names = ['animatedGlyphs', 'textAnimationState', 'sourceTime'];
    return `(() => {\n${code}\nreturn ${names.map(name => `typeof globalThis.${LEGACY_GLOBAL}?.${name} === 'function'`).join(' && ')};\n})()`;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
