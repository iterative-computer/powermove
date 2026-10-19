/*
 * The editor renderer's font cache seeds (native engine plan, amendment 19;
 * docs/engine/text.md, "The renderer's font cache").
 *
 * Chromium's renderer-wide font cache keys platform data by family,
 * effective size, weight/width/slope, and FontCacheStyle identity (below),
 * not by specified size. The first creator of a key fixes its optical size.
 * UI text with the same identity can seed later canvas measurements. The
 * UI uses the system font (`--f-ui`: "SF Pro Text" when
 * installed, else -apple-system) at its CSS size times the window's device
 * pixel ratio, so on a 2x display 12 px UI text creates the 24 px entry with
 * opsz 12, and every later canvas that measures 24 px system-ui text at that
 * weight gets the text-size advances. pm-wasm and the engines measure as
 * that renderer does when they are told these entries.
 *
 * Which entries a renderer has depends on what its UI has laid out: the
 * density setting (`--fs-md` is 12, 13 or 14 px), the panels and menus
 * shown (11, 15 and 18 px; weights 400, 500, 600), page zoom, and every
 * display the window has been on. The renderer reports what it lays out
 * (`UiFontCensus`, renderer engine-doc/ui-font-census.ts); main resolves
 * each family stack as Chromium does (`uiFontSeed`) and keeps each
 * window's seeds in the order its renderer created them (`FontSeedTracker`).
 * Each window is its own renderer with its own cache: the engines measure a
 * view with the seeds of its window, never another window's.
 * `appUiFontCacheSeeds` (equal to pm-text's `app_ui_font_cache_seeds`) is
 * the startup model at the default density, for tools with no window
 * (corpus tests, parity).
 */

/** Fields in Blink FontDescription::CacheKey/FontCacheKey beyond family,
 * effective size and selection request. Kerning/feature/variant shaping
 * state is deliberately separate: it is not in the platform-data key. */
export interface FontCacheStyle {
  textRendering?: string; fontOpticalSizing?: string; fontSynthesis?: string;
  fontSynthesisWeight?: string; fontSynthesisStyle?: string; fontStyle?: string;
  fontStretch?: string; writingMode?: string; textOrientation?: string;
  fontVariantEmoji?: string; fontSizeAdjust?: string; fontVariationSettings?: string;
  fontPalette?: string; fontVariantAlternates?: string;
}

export function fontCacheKey(style: FontCacheStyle = {}): string {
  const value = (v: string | undefined, fallback: string) => (v || fallback).trim().toLowerCase();
  const synthesis = value(style.fontSynthesis, 'weight style small-caps').split(/\s+/);
  const allow = (explicit: string | undefined, token: string) => explicit ? explicit !== 'none' : synthesis.includes(token);
  const variations = (style.fontVariationSettings || 'normal').trim();
  const axes = variations === 'normal' ? [] : variations.split(',').map(v => v.trim()).sort();
  const width = value(style.fontStretch, '100%');
  const writing = value(style.writingMode, 'horizontal-tb');
  return JSON.stringify([
    value(style.textRendering, 'auto'), value(style.fontOpticalSizing, 'auto'),
    allow(style.fontSynthesisWeight, 'weight'), allow(style.fontSynthesisStyle, 'style'),
    value(style.fontStyle, 'normal'), width === 'normal' ? '100%' : width,
    writing === 'horizontal-tb' ? 'horizontal' : `${writing}:${value(style.textOrientation, 'mixed')}`,
    value(style.fontVariantEmoji, 'normal'), value(style.fontSizeAdjust, 'none'), axes,
    (style.fontPalette || 'normal').trim(), (style.fontVariantAlternates || 'normal').trim(),
    true, false, false, false, // subpixel position; synthetic flags; unique-face match
  ]);
}
export const CANVAS_FONT_CACHE_KEY = fontCacheKey();

/** pm-text's `FontCacheSeed`. */
export interface FontCacheSeed {
  family: string;
  weight: number;
  italic: boolean;
  /** The effective size (the cache key's size). */
  size: number;
  /** The specified size of the description that created the entry. */
  specified: number;
  /** Absent only in legacy/static capture seeds (the default canvas key). */
  cacheKey?: string;
}

/** The CSS sizes of the default-density UI text at startup (`--fs-sm`,
 *  `--fs-md`; pm-text `APP_UI_TEXT_SIZES`), for tools with no window. */
export const APP_UI_TEXT_SIZES = [12, 13] as const;

/** Blink's `FontDescription::EffectiveFontSize`: `floorf(size × 100) / 100` in float. */
export function effectiveFontSize(size: number): number {
  const f = Math.fround;
  return f(Math.floor(f(f(size) * 100)) / 100);
}

/** The startup seeds of a default-density renderer whose window has been on
 *  displays with these scale factors, in that order (no window to ask). */
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
 * The engines' seeds: each window's own (the engine lays a view out with the
 * seeds of the window its port belongs to, and an export with those of the
 * window it was started from), and every window's for work no window is
 * known for (pm-node `SeedTable`).
 */
export interface EngineSeedTable {
  seeds: FontCacheSeed[];
  owners: { owner: number; seeds: FontCacheSeed[] }[];
}

/** The window each job engine session main opened is for. */
export type SessionOwner = { session: number; owner: number };

/** The engines' seeds as their font ports take them (`Op::Seeds`). */
export function engineSeedsJson(table: EngineSeedTable, sessions: readonly SessionOwner[] = []): string {
  return JSON.stringify(sessions.length ? { ...table, sessions } : table);
}

/** One description of system-font text a renderer laid out (renderer
 *  engine-doc/ui-font-census.ts). */
export interface UiFontUse {
  /** The computed `font-family`, lower-cased and unquoted, in order. */
  families: string[];
  /** The computed font size in CSS px (before zoom). */
  size: number;
  weight: number;
  italic: boolean;
  cacheKey?: string;
}

/** What a renderer reports: text it laid out at this device pixel ratio
 *  (the display's scale factor times page zoom). */
export interface UiFontCensus {
  dpr: number;
  uses: UiFontUse[];
}

/** The names Blink resolves to the system font on macOS: one cache key. */
export const SYSTEM_FONT_ALIASES: ReadonlySet<string> = new Set(['system-ui', '-apple-system', 'blinkmacsystemfont']);

/** Generic families that resolve to a named font (Helvetica, Times,
 *  Courier, ...), never the system font, so a stack stops at them. The
 *  `ui-*` generics are counted here too: they never reach `system-ui`. */
const NAMED_GENERICS: ReadonlySet<string> = new Set(['serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'math', 'emoji', 'fangsong', 'ui-serif', 'ui-sans-serif', 'ui-monospace', 'ui-rounded']);

/** Splits a computed `font-family` into lower-cased, unquoted names. */
export function parseFontFamilies(css: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: string | null = null;
  for (const ch of css) {
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === ',') {
      if (cur.trim()) out.push(cur.trim().toLowerCase());
      cur = '';
    } else {
      cur += ch;
    }
  }
  if (cur.trim()) out.push(cur.trim().toLowerCase());
  return out;
}

/**
 * The font cache entry a use creates in the system font, or null when its
 * stack reaches another font first: the first family that is a system font
 * alias wins; an installed family (`installed`, the registry's family names
 * as Chromium sees them) or a named generic before it means the text is not
 * in the system font. Only the system font's entries change later canvas
 * measurement (amendment 19).
 */
export function uiFontSeed(use: UiFontUse, dpr: number, installed: (family: string) => boolean): FontCacheSeed | null {
  if (!(Number.isFinite(dpr) && dpr > 0 && Number.isFinite(use.size) && use.size > 0 && Number.isFinite(use.weight))) return null;
  for (const family of use.families) {
    if (SYSTEM_FONT_ALIASES.has(family)) {
      return { family: 'system-ui', weight: use.weight, italic: use.italic, size: effectiveFontSize(Math.fround(Math.fround(use.size) * Math.fround(dpr))), specified: Math.fround(use.size), ...(use.cacheKey ? { cacheKey: use.cacheKey } : {}) };
    }
    if (NAMED_GENERICS.has(family) || installed(family)) return null;
  }
  return null;
}

const seedKey = (s: FontCacheSeed) => `${s.size}|${s.weight}|${s.italic}|${s.cacheKey ?? CANVAS_FONT_CACHE_KEY}`;

/**
 * Each window's renderer font cache, as DOM and canvas text created it.
 * Main tracks its editor windows; this tracker is Electron-free. Censuses are kept
 * raw, in order, so the seeds follow a change of installed fonts.
 */
export class FontSeedTracker {
  private readonly reports = new Map<number, UiFontCensus[]>();
  private readonly listeners = new Set<(owner: number | null) => void>();
  private installed: (family: string) => boolean = () => false;

  /** The renderer of window `owner` (a webContents id) laid this text out.
   *  Uses that add no entry change nothing. */
  report(owner: number, census: UiFontCensus): void {
    const before = this.seedsFor(owner).length;
    const list = this.reports.get(owner) ?? [];
    list.push({ dpr: census.dpr, uses: census.uses.map((u) => ({ ...u, families: [...u.families] })) });
    this.reports.set(owner, list);
    if (this.seedsFor(owner).length !== before) for (const l of this.listeners) l(owner);
  }

  /** The families installed as Chromium sees them changed (a font was
   *  installed or removed): every window's stacks resolve again. */
  setInstalled(installed: (family: string) => boolean): void {
    const before = [...this.reports.keys()].map((o) => JSON.stringify(this.seedsFor(o)));
    this.installed = installed;
    [...this.reports.keys()].forEach((o, i) => {
      if (JSON.stringify(this.seedsFor(o)) !== before[i]) for (const l of this.listeners) l(o);
    });
  }

  /** The window closed: its renderer and its cache are gone. */
  forget(owner: number): void {
    if (this.reports.delete(owner)) for (const l of this.listeners) l(null);
  }

  /** The seeds of one window's renderer (pm-wasm in that window): the first
   *  description to create a key fixes it. */
  seedsFor(owner: number): FontCacheSeed[] {
    const out: FontCacheSeed[] = [];
    const keys = new Set<string>();
    for (const census of this.reports.get(owner) ?? []) {
      for (const use of census.uses) {
        const seed = uiFontSeed(use, census.dpr, this.installed);
        if (seed && !keys.has(seedKey(seed))) {
          keys.add(seedKey(seed));
          out.push(seed);
        }
      }
    }
    return out;
  }

  /** The seeds of all windows' renderers, windows in the order they first
   *  reported: the engines' fallback for work of no known window. */
  allSeeds(): FontCacheSeed[] {
    const out: FontCacheSeed[] = [];
    const keys = new Set<string>();
    for (const owner of this.reports.keys()) {
      for (const seed of this.seedsFor(owner)) {
        if (!keys.has(seedKey(seed))) {
          keys.add(seedKey(seed));
          out.push(seed);
        }
      }
    }
    return out;
  }

  /** What the engines measure with: every window's own seeds, and all of
   *  them for work of no known window. */
  engineSeeds(): EngineSeedTable {
    return { seeds: this.allSeeds(), owners: [...this.reports.keys()].map((owner) => ({ owner, seeds: this.seedsFor(owner) })) };
  }

  /** Called with the owner whose seeds changed (null: a window went away). */
  onChange(listener: (owner: number | null) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

/** Parses a renderer's census message; null when it is malformed. */
export function parseUiFontCensus(json: unknown, limits = { uses: 4096, families: 32, chars: 256 }): UiFontCensus | null {
  if (typeof json !== 'string' || json.length > limits.uses * (limits.families * limits.chars)) return null;
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object') return null;
  const { dpr, uses } = value as { dpr?: unknown; uses?: unknown };
  if (typeof dpr !== 'number' || !Number.isFinite(dpr) || dpr <= 0 || !Array.isArray(uses) || uses.length > limits.uses) return null;
  const out: UiFontUse[] = [];
  for (const u of uses) {
    if (!u || typeof u !== 'object') return null;
    const { families, size, weight, italic, cacheKey } = u as Record<string, unknown>;
    if (!Array.isArray(families) || families.length > limits.families || !families.every((f) => typeof f === 'string' && f.length <= limits.chars)) return null;
    if (typeof size !== 'number' || typeof weight !== 'number' || typeof italic !== 'boolean') return null;
    if (cacheKey !== undefined && (typeof cacheKey !== 'string' || cacheKey.length > 4096)) return null;
    out.push({ families: families as string[], size, weight, italic, ...(cacheKey ? { cacheKey: cacheKey as string } : {}) });
  }
  return { dpr, uses: out };
}
