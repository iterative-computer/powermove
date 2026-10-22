import { exportEffects, type EffectRef } from './export-effects';

// These built-ins have no temporal input. The linked program must confirm
// that property too: a replacement under the same owner can use time.
export const staticEffectSupported = (id: unknown): id is string =>
  exportEffects.supports(id) && !['grain', 'displace'].includes(id);
export type StaticMaskProgram = 'masks' | 'maskApply' | 'pathMaskApply';

/** Live checks for one export, without retaining programs beyond its lifetime. */
export function trackStaticEffects(PM: any) {
  let uniforms = new WeakMap<object, boolean>();
  let disposed = false;
  let previous: { provider: ReturnType<typeof exportEffects.capture>; values: unknown[] } | null = null;
  const stationary = (GL: any, gl: WebGL2RenderingContext, program: WebGLProgram) => {
    let result = uniforms.get(program);
    if (result === undefined) {
      // A replaced program can inherit an older record's uniform map. Ask the
      // linked program itself and cache only by WebGLProgram identity.
      result = gl.getUniformLocation(program, 'u_time') === null
        && gl.getUniformLocation(program, 'u_prog') === null;
      if (GL.contextLost || gl.isContextLost()) return false;
      uniforms.set(program, result);
    }
    return result;
  };
  return {
    capture(refs: readonly EffectRef[], maskPrograms: readonly StaticMaskProgram[] = []) {
      if (disposed || !refs.length && !maskPrograms.length) return null;
      const GL = PM.GL, gl = GL?.gl;
      if (!gl || GL.contextLost || gl.isContextLost() || GL.compiling
          || !refs.every(ref => staticEffectSupported(typeof ref === 'string' ? ref : ref?.type))) return null;
      const provider = refs.length ? exportEffects.capture(PM, refs) : null;
      if (refs.length && !provider) return null;
      const values: unknown[] = [GL, gl, PM.quality];
      for (const selected of provider?.entries || []) {
        const program = GL.progs?.get(selected.key);
        const legacy = typeof PM.effectDef === 'function' ? PM.effectDef({ type: selected.id, version: selected.version }) : PM.FX?.[selected.id];
        if (!program?.pr || !legacy || legacy.backdrop || GL.errors?.has(selected.key)) return null;
        if (!stationary(GL, gl, program.pr)) return null;
        values.push(selected.id, selected.version, selected.key, program, program.pr, legacy,
          legacy.frag, legacy.passes, legacy.keepOrig, legacy.backdrop, legacy.params);
      }
      for (const name of maskPrograms) {
        const program = GL.progs?.get(name);
        if (!program?.pr || GL.errors?.has(name) || !stationary(GL, gl, program.pr)) return null;
        values.push(name, program, program.pr);
        if (name === 'masks') values.push(PM.FRAG_MASK);
        if (name === 'maskApply') values.push(PM.FRAG_MASK_APPLY);
      }
      if (!previous || previous.provider && !exportEffects.valid(PM, previous.provider)
          || values.length !== previous.values.length
          || values.some((value, index) => !Object.is(value, previous!.values[index]))) {
        previous = { provider, values };
      }
      // Context loss may happen during a uniform query without throwing.
      return GL === PM.GL && gl === GL.gl && !GL.contextLost && !gl.isContextLost() ? previous : null;
    },
    dispose() { disposed = true; previous = null; uniforms = new WeakMap(); },
  };
}
