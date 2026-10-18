/*
 * The kernel's real preambles and built-in definitions, outside the app:
 * `legacy/gl/shaders.ts` and `legacy/gl/transitions.ts` install them on a
 * minimal registry (as spike S3's collector did), and the built-in
 * extensions' definitions are imported as they ship.
 */
import { join } from 'node:path';

export const REPO = join(import.meta.dir, '..', '..', '..', '..', '..');
const RENDERER = join(REPO, 'apps', 'desktop', 'src', 'renderer', 'src');
const EXTENSIONS = join(REPO, 'apps', 'desktop', 'src', 'extensions');

const shaders = await import(join(RENDERER, 'legacy/gl/shaders.ts'));
const transitions = await import(join(RENDERER, 'legacy/gl/transitions.ts'));
export const snapshots = await import(join(RENDERER, 'kernel/definition-snapshots.ts'));
const { EFFECTS } = await import(join(EXTENSIONS, 'effects-basic/effects.ts'));
const { BUILTIN_TRANSITIONS } = await import(join(EXTENSIONS, 'transitions-basic/transitions.ts'));

const PM: any = { uid: () => 'fx', P: (v: unknown) => ({ v }) };
shaders.install(PM);
transitions.install(PM);

export const PREAMBLES = { effect: PM.GLSL_PRE as string, transition: PM.TRANSITION_PRE as string, layer: PM.SHADER_HEADER as string };
export const VERTEX: string = PM.VERT;
export const BUILTIN_EFFECTS: any[] = EFFECTS;
export const BUILTIN_TRANSITION_DEFS: any[] = BUILTIN_TRANSITIONS;

/** The built-in definitions as the kernel snapshots them (origin builtIn), effects then transitions, in registration order. */
export function builtinSnapshots(): any[] {
  return [
    ...BUILTIN_EFFECTS.map((def) => snapshots.effectSnapshot(def, 'builtIn', PREAMBLES)),
    ...BUILTIN_TRANSITION_DEFS.map((def) => snapshots.transitionSnapshot(def, 'builtIn', PREAMBLES)),
  ];
}
