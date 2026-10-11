/**
 * Pure legacy modules the capture loads into the renderer beside the app's
 * own bundle. The renderer does not expose these functions on `PM`; bundling
 * the same source file from the same commit gives the capture the code the
 * compositor runs, rather than a hand-written copy.
 *
 * - `legacy/core/text-animation.ts`: `textAnimationState` and `animatedGlyphs`,
 *   which `legacy/gl/raster.ts` calls to pose each glyph of an animated or
 *   range-styled text layer. The module has no imports and takes `PM` as an
 *   argument, so it bundles without the renderer.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { rendererSrc } from './catalog';

export const TEXT_ANIMATION_GLOBAL = '__corpusTextAnimation';
export const TEXT_ANIMATION_MODULE = path.join(rendererSrc, 'legacy/core/text-animation.ts');

/** A single JavaScript expression that installs `globalThis.__corpusTextAnimation` and evaluates to true. */
export async function textAnimationScript(): Promise<string> {
  const dir = path.join(tmpdir(), 'powermove-engine-corpus');
  mkdirSync(dir, { recursive: true });
  const entry = path.join(dir, 'text-animation-entry.ts');
  writeFileSync(entry, [
    `import { animatedGlyphs, textAnimationState } from ${JSON.stringify(TEXT_ANIMATION_MODULE)};`,
    `(globalThis as any).${TEXT_ANIMATION_GLOBAL} = { animatedGlyphs, textAnimationState };`,
    '',
  ].join('\n'));
  const result = await Bun.build({ entrypoints: [entry], target: 'browser', format: 'iife', minify: false, sourcemap: 'none' });
  if (!result.success || result.outputs.length !== 1) {
    throw new Error(`Bundling ${TEXT_ANIMATION_MODULE} failed:\n${result.logs.map(log => String(log)).join('\n')}`);
  }
  const code = await result.outputs[0]!.text();
  return `(() => {\n${code}\nreturn typeof globalThis.${TEXT_ANIMATION_GLOBAL}?.animatedGlyphs === 'function';\n})()`;
}
