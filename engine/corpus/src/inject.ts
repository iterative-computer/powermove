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
 * - `legacy/core/frame-preparation.ts`: `blenderFrameRequests`, the ordered
 *   (composition, time, size) list `prepareFrame` hands to Blender, which
 *   labels the recorded Blender snapshots. Its other imports (video seeking,
 *   the three.js renderer) are replaced by stubs that throw when called, so the
 *   bundle stays small and cannot drive the app's media or GPU state.
 *
 * Every function takes `PM` as an argument, so the bundles run against the
 * live app without sharing module state with it.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import type { BunPlugin } from 'bun';
import { rendererSrc } from './catalog';

export const LEGACY_GLOBAL = '__corpusLegacy';
export const TEXT_ANIMATION_MODULE = path.join(rendererSrc, 'legacy/core/text-animation.ts');
export const RETIMING_MODULE = path.join(rendererSrc, 'legacy/core/retiming.ts');
export const FRAME_PREPARATION_MODULE = path.join(rendererSrc, 'legacy/core/frame-preparation.ts');

/** Imports of frame-preparation.ts that `blenderFrameRequests` does not use. */
const STUBBED_IMPORTS: Record<string, string[]> = {
  '../../core/scene3d/rendering': ['beginBlenderCapture', 'prepareBlenderFrame'],
  './video-seek': ['cancelPreviewVideoSeek'],
  './video-instances': ['pauseVideoInstances'],
  '../../../../shared/video-timing': ['videoPlaybackTime'],
};

/** Upper bound on the bundle: the stubs keep three.js, zod, and the media modules out. */
const MAX_BUNDLE_BYTES = 64 * 1024;

const stubs: BunPlugin = {
  name: 'corpus-stubs',
  setup(build) {
    build.onResolve({ filter: /.*/ }, args => {
      if (args.importer !== FRAME_PREPARATION_MODULE || !(args.path in STUBBED_IMPORTS)) return undefined;
      return { path: args.path, namespace: 'corpus-stub' };
    });
    build.onLoad({ filter: /.*/, namespace: 'corpus-stub' }, args => ({
      loader: 'js',
      contents: STUBBED_IMPORTS[args.path]!.map(name =>
        `export function ${name}() { throw new Error(${JSON.stringify(`${name} is stubbed in the corpus bundle`)}); }`).join('\n'),
    }));
  },
};

/** A single JavaScript expression that installs `globalThis.__corpusLegacy` and evaluates to true. */
export async function legacyScript(): Promise<string> {
  const dir = path.join(tmpdir(), 'powermove-engine-corpus');
  mkdirSync(dir, { recursive: true });
  const entry = path.join(dir, 'legacy-entry.ts');
  writeFileSync(entry, [
    `import { animatedGlyphs, textAnimationState } from ${JSON.stringify(TEXT_ANIMATION_MODULE)};`,
    `import { sourceTime } from ${JSON.stringify(RETIMING_MODULE)};`,
    `import { blenderFrameRequests } from ${JSON.stringify(FRAME_PREPARATION_MODULE)};`,
    `(globalThis as any).${LEGACY_GLOBAL} = { animatedGlyphs, textAnimationState, sourceTime, blenderFrameRequests };`,
    '',
  ].join('\n'));
  const result = await Bun.build({ entrypoints: [entry], target: 'browser', format: 'iife', minify: false, sourcemap: 'none', plugins: [stubs] });
  if (!result.success || result.outputs.length !== 1) {
    throw new Error(`Bundling the legacy helpers failed:\n${result.logs.map(log => String(log)).join('\n')}`);
  }
  const code = await result.outputs[0]!.text();
  if (code.length > MAX_BUNDLE_BYTES) throw new Error(`The legacy helper bundle is ${code.length} bytes; a new import in frame-preparation.ts probably needs a stub`);
  const names = ['animatedGlyphs', 'textAnimationState', 'sourceTime', 'blenderFrameRequests'];
  return `(() => {\n${code}\nreturn ${names.map(name => `typeof globalThis.${LEGACY_GLOBAL}?.${name} === 'function'`).join(' && ')};\n})()`;
}
