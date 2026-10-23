/**
 * Definitions read from the legacy desktop source, so the generator authors
 * documents against the same effect, transition, 3D, caption, and animator
 * definitions the app registers. Nothing here is copied by hand except where a
 * module cannot be imported outside the renderer (shader presets, see below).
 */
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

export const repoRoot = path.resolve(import.meta.dir, '../../..');
export const desktopRoot = path.join(repoRoot, 'apps/desktop');
export const desktopSrc = path.join(desktopRoot, 'src');
export const rendererSrc = path.join(desktopSrc, 'renderer/src');
export const fixturesDir = path.join(desktopRoot, 'e2e/fixtures');

// The inspector's text-animator module imports pure text-animation helpers
// through the `powermove` alias that Vite maps to the editor helpers. Load a
// copy whose alias points straight at the text-animation module it re-exports.
async function aliasedModule(file: string): Promise<any> {
  const source = readFileSync(file, 'utf8');
  const target = path.join(rendererSrc, 'legacy/core/text-animation.ts');
  const rewritten = source.replace(/from 'powermove'/g, `from ${JSON.stringify(target)}`);
  if (rewritten === source) throw new Error(`${file} no longer imports from 'powermove'`);
  const dir = mkdtempSync(path.join(tmpdir(), 'powermove-engine-corpus-'));
  try {
    const out = path.join(dir, path.basename(file));
    writeFileSync(out, rewritten);
    return await import(out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const { EFFECTS } = await import(path.join(desktopSrc, 'extensions/effects-basic/effects.ts'));
const { BUILTIN_TRANSITIONS } = await import(path.join(desktopSrc, 'extensions/transitions-basic/transitions.ts'));
const { STUDIO_CUBE } = await import(path.join(desktopSrc, 'extensions/layers-3d/studio-cube.ts'));
const { OBJ_MODEL } = await import(path.join(desktopSrc, 'extensions/layers-3d/obj-model.ts'));
const animators = await aliasedModule(path.join(desktopSrc, 'extensions/inspector/text-animators.ts'));
const schema3d = await import(path.join(rendererSrc, 'core/scene3d/schema.ts'));
const layers3d = await import(path.join(rendererSrc, 'core/scene3d/layers.ts'));
const captions = await import(path.join(rendererSrc, 'captions/model.ts'));
const validator = await import(path.join(rendererSrc, 'core/validate/project.ts'));
const projectTypes = await import(path.join(rendererSrc, 'core/types/project.ts'));
const matte = await import(path.join(rendererSrc, 'legacy/core/matte.ts'));
const blender = await import(path.join(desktopSrc, 'shared/blender.ts'));
const ipc = await import(path.join(desktopSrc, 'shared/ipc.ts'));

export interface ParamDef { k: string; def: unknown; type?: string; min?: number; max?: number }
export interface EffectDef { id: string; label: string; params: ParamDef[] }
export interface TransitionDef { id: string; label: string; params: ParamDef[] }
export interface LayerDefinition { id: string; version: number; params: ParamDef[]; defaults?: unknown; renderer: { kind: string } }

export const effects: EffectDef[] = EFFECTS;
export const transitions: TransitionDef[] = BUILTIN_TRANSITIONS;
export const studioCube: LayerDefinition = STUDIO_CUBE;
export const objModel: LayerDefinition = OBJ_MODEL;
export const blendModes: readonly string[] = projectTypes.BLEND_MODES;
export const layerTypes: string[] = Object.keys(projectTypes.TYPE_META);
export const typeMeta: Record<string, { color: string; label: string }> = projectTypes.TYPE_META;
export const matteModes: string[] = matte.MATTE_MODES;

export const animatorPresets: Array<{ id: string }> = animators.ANIMATOR_PRESETS;
export const makeAnimator: (P: (v: unknown) => unknown, id: string, name: string, preset: unknown, size: number) => Record<string, unknown> = animators.makeAnimator;

export const scene3d = {
  definitions: layers3d.LAYER3D_DEFINITIONS as { object: string; light: string; camera: string },
  sceneDefinition: schema3d.SCENE3D_DEFINITION as string,
  parseLayer3DData: layers3d.parseLayer3DData as (role: 'object' | 'light' | 'camera', raw: unknown) => any,
  parseScene: schema3d.parseScene as (raw: unknown) => any,
  createScene: schema3d.createScene as () => any,
  createObject: schema3d.createObject as (id: string, source?: unknown, name?: string) => any,
  createLight: schema3d.createLight as (id: string, type?: string, name?: string) => any,
};

export const captionModel = {
  presetStyle: captions.presetStyle as (preset: string, height: number) => Record<string, unknown>,
  normalizeCaptionsContent: captions.normalizeCaptionsContent as (raw: unknown) => any,
  presets: (captions.CAPTION_PRESETS as Array<{ id: string }>).map(preset => preset.id),
};

export const sanitizeProject: (raw: unknown) => any = validator.sanitizeProject;

/** Composition 3D render settings (`render3d`), parsed strictly as the app parses them. */
export const parseRender3D: (raw: unknown) => Record<string, unknown> = raw => blender.render3DSchema.parse(raw);
/** The IPC channel the renderer sends Blender jobs on; the capture records jobs there instead of running Blender. */
export const BLENDER_RUN_CHANNEL: string = ipc.IPC.blenderRun;

/**
 * Shader-layer presets live inside `legacy/gl/shaders.ts`'s installer, which
 * cannot run outside the renderer. Extract the template literals from the
 * source (they contain no interpolation); the capture tool re-checks every
 * extracted preset against `PM.SHADER_PRESETS` in the running app.
 */
export function shaderPresets(): Record<string, string> {
  const source = readFileSync(path.join(rendererSrc, 'legacy/gl/shaders.ts'), 'utf8');
  const literal = (from: number): { text: string; end: number } => {
    const start = source.indexOf('`', from);
    if (start < 0) throw new Error('Shader preset literal not found');
    let end = start + 1;
    while (source[end] !== '`') {
      if (source[end] === '\\') end++;
      if (source.slice(end, end + 2) === '${') throw new Error('Shader preset literal is interpolated');
      end++;
      if (end >= source.length) throw new Error('Unterminated shader preset literal');
    }
    return { text: source.slice(start + 1, end), end: end + 1 };
  };
  const templateAt = source.indexOf('PM.SHADER_TEMPLATE = `');
  if (templateAt < 0) throw new Error('PM.SHADER_TEMPLATE not found in shaders.ts');
  const template = literal(templateAt).text;
  const presetsAt = source.indexOf('PM.SHADER_PRESETS = {');
  if (presetsAt < 0) throw new Error('PM.SHADER_PRESETS not found in shaders.ts');
  const out: Record<string, string> = {};
  const entry = /\s*'([^']+)':\s*(PM\.SHADER_TEMPLATE|`)/y;
  let cursor = presetsAt + 'PM.SHADER_PRESETS = {'.length;
  for (;;) {
    entry.lastIndex = cursor;
    const match = entry.exec(source);
    if (!match) break;
    if (match[2] === 'PM.SHADER_TEMPLATE') {
      out[match[1]!] = template;
      cursor = entry.lastIndex;
    } else {
      const { text, end } = literal(entry.lastIndex - 1);
      out[match[1]!] = text;
      cursor = end;
    }
    const comma = /\s*,/y;
    comma.lastIndex = cursor;
    if (comma.exec(source)) cursor = comma.lastIndex;
  }
  if (Object.keys(out).length !== 5) throw new Error(`Expected 5 shader presets, found ${Object.keys(out).length}`);
  return out;
}

/**
 * Extension effects from e2e fixture extensions (apps/desktop/e2e/fixtures/<extension>).
 * The capture installs the extension into the isolated profile before launch and
 * checks these parameter keys against the definition the running app registered.
 */
export const EXTENSION_EFFECTS: Record<string, { extension: string; params: string[] }> = {
  typewriter: {
    extension: 'typewriter-effect',
    params: ['progress', 'cursor', 'cursorWidth', 'cursorHeight', 'cursorOffsetX', 'cursorOffsetY', 'cursorRound', 'blinkRate',
      'gradStart', 'gradMid', 'gradEnd', 'gradMidPos', 'gradAngle', 'gradOffset', 'gradCycle'],
  },
};

/** Effect types no extension defines, for the missing-definition behavior. */
export const MISSING_EFFECT_PREFIX = 'com.example.missing-';

export const isForeignEffect = (type: string): boolean => type in EXTENSION_EFFECTS || type.startsWith(MISSING_EFFECT_PREFIX);

export function effectDef(id: string): EffectDef {
  const definition = effects.find(effect => effect.id === id);
  if (!definition) throw new Error(`Unknown built-in effect ${id}`);
  return definition;
}

export function transitionDef(id: string): TransitionDef {
  const definition = transitions.find(transition => transition.id === id);
  if (!definition) throw new Error(`Unknown built-in transition ${id}`);
  return definition;
}
