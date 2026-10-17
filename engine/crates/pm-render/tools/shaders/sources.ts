/*
 * The app's shader-layer runtime and built-in layer definitions, outside
 * the app (`legacy/gl/shaders.ts` installed on a minimal registry, as the
 * effects tools and spike S3's collector do).
 */
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { PREAMBLES, REPO, snapshots } from '../effects/preambles';

const RENDERER = join(REPO, 'apps', 'desktop', 'src', 'renderer', 'src');
const EXTENSIONS = join(REPO, 'apps', 'desktop', 'src', 'extensions');

const shaders = await import(join(RENDERER, 'legacy/gl/shaders.ts'));
export const extensionLayers = await import(join(RENDERER, 'kernel/extension-layers.ts'));
const { STUDIO_CUBE } = await import(join(EXTENSIONS, 'layers-3d/studio-cube.ts'));
const { OBJ_MODEL } = await import(join(EXTENSIONS, 'layers-3d/obj-model.ts'));

export const PM: any = { uid: () => 'fx', P: (v: unknown) => ({ v }) };
shaders.install(PM);

export const SHADER_HEADER: string = PM.SHADER_HEADER;
export const SHADER_TEMPLATE: string = PM.SHADER_TEMPLATE;
export const SHADER_PRESETS: Record<string, string> = PM.SHADER_PRESETS;
export const parseUniforms: (code: string) => any[] = PM.parseUniforms;
export const BUILTIN_LAYERS: any[] = [STUDIO_CUBE, OBJ_MODEL];

/** The checkerboard body `contentQuad` draws for a missing extension layer. */
export async function missingLayerBody(): Promise<string> {
  const text = await readFile(join(RENDERER, 'legacy/gl/compositor.ts'), 'utf8');
  const match = /const missing = `([\s\S]*?)`;/.exec(text);
  if (!match) throw new Error('missing-layer placeholder not found in compositor.ts');
  return match[1]!;
}

export async function builtinLayerFile() {
  return {
    header: SHADER_HEADER,
    missing: await missingLayerBody(),
    template: SHADER_TEMPLATE,
    presets: Object.entries(SHADER_PRESETS).map(([name, code]) => ({ name, code })),
    layers: BUILTIN_LAYERS.map((def) => snapshots.layerSnapshot(def, 'builtIn', PREAMBLES)),
  };
}
