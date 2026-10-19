/**
 * Expression styles for the paired P6 measurements (docs/engine/perf.md):
 * the Node benchmark (scripts/expression-benchmark.ts) and the Electron 44
 * measurement (e2e/engine-expression-performance.spec.ts) run the same suites.
 *
 * Each suite has four sources; a fixture puts each on 100 layers (400
 * channels). The suites cover the typed numeric plan's arithmetic, its
 * helpers and conditionals, the sin-based oscillators, and sources only the
 * general bytecode evaluator answers (strings, dynamic indexes, `==`).
 */
export const EXPRESSION_SUITES: Record<string, string[]> = {
  numeric: ['value + time * 40', 'param("amp") + comp.layers.length + time', 'layer.p.opacity.v + t * 13', 'Math.sin(time) * 10 + idx + value'],
  helpers: ['clamp(value + time * 3, 0, 50)', 'linear(time, 0, 2, 0, 100)', 'time > 1 ? value * 2 : value + time', 'ease(time, 0, 3, value, 100) + loop(2, time)'],
  conditionals: ['time < 2 ? param("amp") : value', 'value + (idx % 2 ? time : -time)', 'lerp(value, 100, time / 10)', 'pingpong(time, 3) * value'],
  oscillators: ['wiggle(2, 10) + value', 'Math.sin(time * 2 * Math.PI) * 50 + value', 'random(idx) * 10 + bounce(t % 1) * value', 'Math.cos(t * 3) * param("amp") + Math.abs(Math.sin(time))'],
  general: ['comp.layers[0].p.opacity.v + time * 2', 'layer.name == "Solid" ? value + time : value', 'value + (layer.p.opacity.v + "").length', 'Math.max(value, comp.layers[1].p.opacity.v) + time'],
};

/**
 * Suites whose answers involve `Math.sin`/`Math.cos`. Node 24's V8 computes
 * some of these one ulp away from Electron 44's V8 (which pm-expr matches bit
 * for bit), so the Node benchmark cannot check them exactly and skips them;
 * Electron measures them.
 */
export const ELECTRON_ONLY_SUITES = new Set(['oscillators']);

/** How a frame reads the 400 channels. */
export const READ_PATTERNS = {
  /** Every channel, every frame. */
  all: 'all',
  /**
   * Ten consecutive layers (40 channels) per frame, the window moving one
   * layer per frame, after every channel has been read: sequential layers,
   * most of them inactive at any time.
   */
  window: 'window',
} as const;

/** The channel indexes a frame at `time` reads, for 100 layers of 4. */
export function frameChannels(pattern: keyof typeof READ_PATTERNS, time: number, count: number): [number, number] {
  if (pattern === 'all') return [0, count];
  const per = count / 100 | 0;
  const start = (Math.round(time * 30) % 91) * per;
  return [start, start + 10 * per];
}
