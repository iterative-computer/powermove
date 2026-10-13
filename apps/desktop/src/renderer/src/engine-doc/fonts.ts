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
 * answers natively. Without a provider (the editor today: the preload exposes
 * no engine-fonts channel) text stays with the legacy code.
 */
import type { PmWasm } from './wasm';

export interface FontProvider {
  /** `Query[]` JSON → `{ generation?, faces, answers }` JSON. */
  answer(queries: string): string | Promise<string>;
  /** A face's metrics-and-shaping subset. */
  subset(blob: string, index: number): Uint8Array | Promise<Uint8Array>;
  /** Whether both calls return values directly (no promises). */
  readonly sync: boolean;
}

const MAX_ROUNDS = 6;

/** Loads what the last layout lacked; returns whether anything was added. */
function loadOnce(wasm: PmWasm, provider: FontProvider, loaded: Set<string>, generation: { value: number | null }):
  boolean | Promise<boolean> {
  const report = wasm.fontReport();
  const pending = new Map<string, [string, number]>();
  for (const [blob, index] of report.pending) pending.set(`${blob}.${index}`, [blob, index]);
  const addSubsets = (): boolean | Promise<boolean> => {
    const todo = [...pending.entries()].filter(([key]) => !loaded.has(key));
    if (!todo.length) return false;
    const results = todo.map(([key, [blob, index]]) => {
      loaded.add(key);
      return [blob, index, provider.subset(blob, index)] as const;
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
  private loaded = new Set<string>();
  private generation = { value: null as number | null };
  private busy: Promise<void> | null = null;
  rounds = 0;
  failures = 0;

  constructor(readonly provider: FontProvider) {}

  /**
   * After a `{fonts}` reply: with a synchronous provider, loads until `retry`
   * stops asking (returns its last reply); with an asynchronous one, starts
   * loading in the background and returns null.
   */
  resolve<T>(wasm: PmWasm, retry: () => T, needsFonts: (reply: T) => boolean): T | null {
    if (!this.provider.sync) {
      this.busy ??= (async () => {
        try {
          for (let i = 0; i < MAX_ROUNDS; i++) {
            this.rounds += 1;
            if (!(await loadOnce(wasm, this.provider, this.loaded, this.generation))) break;
          }
        } catch (error) {
          this.failures += 1;
          console.warn('[engine-doc] font loading failed', error);
        } finally { this.busy = null; }
      })();
      return null;
    }
    try {
      for (let i = 0; i < MAX_ROUNDS; i++) {
        this.rounds += 1;
        if (!(loadOnce(wasm, this.provider, this.loaded, this.generation) as boolean)) return null;
        const reply = retry();
        if (!needsFonts(reply)) return reply;
      }
    } catch (error) {
      this.failures += 1;
      console.warn('[engine-doc] font loading failed', error);
    }
    return null;
  }
}
