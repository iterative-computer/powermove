/*
 * `render.gl.context` (a deprecated compatibility member, amendment 23;
 * docs/engine/extensions-compat.md).
 *
 * Today it is the compositor's WebGL2 context: null until a viewer hands a
 * canvas to `render.gl.init`, then that canvas's context. Old custom viewers
 * (forks frozen before the engine) read it to find their canvas
 * (`context.canvas`, to recover their stage after a reload) and test it to
 * decide whether to call `init` (`if (!context) init(canvas)`).
 *
 * With "Use New Engine" on there is no compositor context. The engine
 * presents over the canvas `init` bound (engine-connection/canvas-binding.ts),
 * which stays in the document, hidden under the presentation. The
 * compatibility context is that canvas's own WebGL2 context, made when first
 * asked for: the same lifecycle (null before `init`), the same `canvas`, and
 * a live WebGL2 context an extension may use. Drawing into it does not show:
 * the canvas is under the engine's presentation (docs/engine/extensions-compat.md,
 * "What the compatibility context does not do").
 */
type LegacyPM = Record<string, any>;

const ATTRIBUTES: WebGLContextAttributes = { alpha: true, premultipliedAlpha: true, antialias: false, preserveDrawingBuffer: true };

export function compatContext(PM: LegacyPM): WebGL2RenderingContext | null {
  // Without the engine (or before it binds): the compositor's own context.
  const binding = PM?.EngineCanvas;
  const bound: HTMLCanvasElement | null = binding?.bound ?? null;
  if (!PM?.EngineRender || !bound) return (PM?.GL?.gl ?? null) as WebGL2RenderingContext | null;
  // A context of the bound canvas: the compositor's when it made one there
  // (getContext returns it), otherwise a new one.
  try {
    return bound.getContext('webgl2', ATTRIBUTES);
  } catch {
    return null;
  }
}
