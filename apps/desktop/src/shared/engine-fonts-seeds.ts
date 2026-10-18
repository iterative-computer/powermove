/*
 * The editor renderer's font cache seeds (native engine plan, amendment 19;
 * docs/engine/text.md, "The renderer's font cache").
 *
 * Chromium's renderer-wide font cache keys platform data by family,
 * effective size, and weight/width/slope, not by specified size, and the
 * first description that creates an entry fixes its optical size. The app's
 * own UI text (system-ui at --fs-sm and --fs-md, 12 and 13 px, weight 400)
 * is laid out at the window's device scale factor, so on a 2x display it
 * creates the 24 and 26 px entries with opsz 12 and 13, and every later
 * canvas that measures 24 or 26 px system-ui text at weight 400 gets the
 * text-size advances. pm-wasm and the engines measure as that renderer
 * does when they are told these entries.
 *
 * A window keeps the entries of every scale factor it has been on (moving
 * to a 1x display adds the 12 and 13 px entries; the 24 and 26 px ones stay).
 * `appUiFontCacheSeeds` equals pm-text's `app_ui_font_cache_seeds`.
 */

/** pm-text's `FontCacheSeed`. */
export interface FontCacheSeed {
  family: string;
  weight: number;
  italic: boolean;
  /** The effective size (the cache key's size). */
  size: number;
  /** The specified size of the description that created the entry. */
  specified: number;
}

/** The CSS sizes of the UI text that creates system-ui entries (pm-text `APP_UI_TEXT_SIZES`). */
export const APP_UI_TEXT_SIZES = [12, 13] as const;

/** Blink's `FontDescription::EffectiveFontSize`: `floorf(size × 100) / 100` in float. */
export function effectiveFontSize(size: number): number {
  const f = Math.fround;
  return f(Math.floor(f(f(size) * 100)) / 100);
}

/** The seeds of a renderer whose window has been on displays with these scale factors, in that order. */
export function appUiFontCacheSeeds(scaleFactors: readonly number[]): FontCacheSeed[] {
  const out: FontCacheSeed[] = [];
  for (const scale of scaleFactors) {
    if (!(Number.isFinite(scale) && scale > 0)) continue;
    for (const css of APP_UI_TEXT_SIZES) {
      const seed: FontCacheSeed = { family: 'system-ui', weight: 400, italic: false, size: effectiveFontSize(Math.fround(css * Math.fround(scale))), specified: css };
      // The first creator of a key fixes it.
      if (!out.some((s) => s.size === seed.size && s.weight === seed.weight && s.italic === seed.italic)) out.push(seed);
    }
  }
  return out;
}

/** The seeds as the engine's font port and pm-wasm take them. */
export function seedsJson(seeds: readonly FontCacheSeed[]): string {
  return JSON.stringify({ seeds });
}

/**
 * Scale factors each window has been on, in order (main tracks its editor
 * windows; Electron-free so it is testable).
 */
export class FontSeedTracker {
  private readonly factors = new Map<number, number[]>();
  private readonly listeners = new Set<(owner: number | null) => void>();

  /** The window `owner` (a webContents id) is on a display with this scale factor. */
  observe(owner: number, scaleFactor: number): void {
    if (!(Number.isFinite(scaleFactor) && scaleFactor > 0)) return;
    const seen = this.factors.get(owner) ?? [];
    if (seen.includes(scaleFactor)) {
      if (!this.factors.has(owner)) this.factors.set(owner, seen);
      return;
    }
    seen.push(scaleFactor);
    this.factors.set(owner, seen);
    for (const l of this.listeners) l(owner);
  }

  /** The window closed: its renderer and its cache are gone. */
  forget(owner: number): void {
    if (this.factors.delete(owner)) for (const l of this.listeners) l(null);
  }

  /** The seeds of one window's renderer (pm-wasm in that window). */
  seedsFor(owner: number): FontCacheSeed[] {
    return appUiFontCacheSeeds(this.factors.get(owner) ?? []);
  }

  /** The engines serve every window: the seeds of all windows' renderers,
   *  windows in the order they were first seen (identical for windows that
   *  were only ever on the same kind of display). */
  allSeeds(): FontCacheSeed[] {
    const scales: number[] = [];
    for (const seen of this.factors.values()) for (const s of seen) if (!scales.includes(s)) scales.push(s);
    return appUiFontCacheSeeds(scales);
  }

  /** Called with the owner whose seeds changed (null: a window went away). */
  onChange(listener: (owner: number | null) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}
