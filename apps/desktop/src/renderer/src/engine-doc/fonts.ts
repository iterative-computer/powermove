/*
 * Fonts for pm-wasm's text layout (docs/engine/text.md, "The registry
 * boundary"): main owns font resolution, pm-wasm lays text out over the
 * registry's answers and metrics-and-shaping subsets.
 *
 * A text query that meets a query the answers lack, or a face whose bytes are
 * not loaded, comes back `{fonts: true}`. With a synchronous provider (the
 * registry in the same process: the headless document host) the loop runs at
 * once: send the misses, add the answers and the subsets of every face they
 * name, and ask again, until a pass needs nothing (or 6 rounds). With an
 * asynchronous provider (the editor, through main) the query falls back to the
 * legacy answer and the same loop runs in the background, so the next query
 * answers natively. Without a provider, or once the provider has failed
 * `MAX_FAILURES` times in a row (main without POWERMOVE_ENGINE_FONTS=1 has no
 * channel), text stays with the legacy code (counted as `fonts` fallbacks).
 */
import type { PmWasm } from './wasm';

export interface FontProvider {
  /** `Query[]` JSON → `{ generation?, faces, answers }` JSON. */
  answer(queries: string): string | Promise<string>;
  /** A face's metrics-and-shaping subset. */
  subset(blob: string, index: number): Uint8Array | Promise<Uint8Array>;
  /**
   * The whole face, outlines included (the fonts lane's service, when it
   * provides one). With it every face is loaded whole and pm-wasm is told
   * its faces carry outlines (`pm_fonts_outlines(1)`), so answers that need
   * glyph ink (text bounds, picks over text, raster placement) are native
   * instead of `{ink}` (docs/engine/document-host.md, "Interfaces").
   */
  whole?(blob: string, index: number): Uint8Array | Promise<Uint8Array>;
  /** Whether both calls return values directly (no promises). */
  readonly sync: boolean;
}

const MAX_ROUNDS = 6;
/** Consecutive failed loads after which the provider is given up. */
export const MAX_FAILURES = 3;

/** Loads what the last layout lacked; returns whether anything was added. */
function loadOnce(wasm: PmWasm, provider: FontProvider, loaded: Set<string>, generation: { value: number | null }):
  boolean | Promise<boolean> {
  const report = wasm.fontReport();
  const pending = new Map<string, [string, number]>();
  for (const [blob, index] of report.pending) pending.set(`${blob}.${index}`, [blob, index]);
  const addSubsets = (): boolean | Promise<boolean> => {
    const todo = [...pending.entries()].filter(([key]) => !loaded.has(key));
    if (!todo.length) return false;
    const load = provider.whole ? provider.whole.bind(provider) : provider.subset.bind(provider);
    const results = todo.map(([key, [blob, index]]) => {
      loaded.add(key);
      return [blob, index, load(blob, index)] as const;
    });
    if (provider.sync) {
      for (const [blob, index, bytes] of results) wasm.addFont(blob, index, bytes as Uint8Array);
      return true;
    }
    return Promise.all(results.map(async ([blob, index, bytes]) => wasm.addFont(blob, index, await bytes))).then(() => true);
  };
  const addAnswers = (json: string): boolean | Promise<boolean> => {
    const parsed = JSON.parse(json);
    if (typeof parsed?.generation === 'number') {
      if (generation.value !== null && generation.value !== parsed.generation) {
        // The registry changed (project fonts): start again from empty answers.
        wasm.call(() => wasm.x.pm_fonts_clear());
        loaded.clear();
      }
      generation.value = parsed.generation;
    }
    wasm.addFontAnswers(json);
    for (const face of parsed?.faces ?? []) if (typeof face?.blob === 'string') pending.set(`${face.blob}.${face.index}`, [face.blob, face.index]);
    const added = addSubsets();
    return added instanceof Promise ? added.then(() => true) : true;
  };
  if (report.misses.length) {
    const answer = provider.answer(JSON.stringify(report.misses));
    return answer instanceof Promise ? answer.then(addAnswers) : addAnswers(answer);
  }
  return addSubsets();
}

export class FontLoader {
  /** Faces and answers each module instance holds: a trapped instance's
   *  replacement starts empty and loads again. */
  private perInstance = new WeakMap<object, { loaded: Set<string>; generation: { value: number | null } }>();
  private state(wasm: PmWasm) {
    let s = this.perInstance.get(wasm);
    if (!s) { s = { loaded: new Set(), generation: { value: null } }; this.perInstance.set(wasm, s); }
    return s;
  }
  private busy: Promise<void> | null = null;
  rounds = 0;
  failures = 0;
  private failedInARow = 0;
  /** Set after MAX_FAILURES failures in a row: the provider is unavailable. */
  disabled = false;

  constructor(readonly provider: FontProvider) {}

  /** Resolves when no background load is running (an asynchronous provider). */
  whenIdle(): Promise<void> { return this.busy ?? Promise.resolve(); }

  /** Declares whole faces to a module instance once (a replaced instance asks again). */
  private declared = new WeakSet<object>();
  private declareOutlines(wasm: PmWasm): void {
    if (!this.provider.whole || this.declared.has(wasm)) return;
    this.declared.add(wasm);
    wasm.call(() => wasm.x.pm_fonts_outlines(1));
  }

  /**
   * After a `{fonts}` reply: with a synchronous provider, loads until `retry`
   * stops asking (returns its last reply); with an asynchronous one, starts
   * loading in the background and returns null.
   */
  resolve<T>(wasm: PmWasm, retry: () => T, needsFonts: (reply: T) => boolean): T | null {
    if (this.disabled) return null;
    this.declareOutlines(wasm);
    if (!this.provider.sync) {
      this.busy ??= (async () => {
        try {
          for (let i = 0; i < MAX_ROUNDS; i++) {
            this.rounds += 1;
            if (!(await loadOnce(wasm, this.provider, this.state(wasm).loaded, this.state(wasm).generation))) break;
          }
          this.failedInARow = 0;
        } catch (error) {
          this.failed(error);
        } finally { this.busy = null; }
      })();
      return null;
    }
    try {
      for (let i = 0; i < MAX_ROUNDS; i++) {
        this.rounds += 1;
        if (!(loadOnce(wasm, this.provider, this.state(wasm).loaded, this.state(wasm).generation) as boolean)) return null;
        const reply = retry();
        if (!needsFonts(reply)) { this.failedInARow = 0; return reply; }
      }
    } catch (error) {
      this.failed(error);
    }
    return null;
  }

  private failed(error: unknown) {
    this.failures += 1;
    this.failedInARow += 1;
    if (this.failedInARow >= MAX_FAILURES) {
      this.disabled = true;
      console.warn('[engine-doc] the font provider failed repeatedly; text layout stays with the legacy code', error);
    } else console.warn('[engine-doc] font loading failed', error);
  }
}
