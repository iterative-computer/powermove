/**
 * Generates tests/vectors/caption-vectors.json.gz by running the captions
 * model, layout, and rasterizer frame (`captions/model.ts`, `layout.ts`,
 * `render.ts`) under bun over seeded random inputs. `tests/caption_parity.rs`
 * replays every case.
 *
 *   bun engine/crates/pm-eval/tests/vectors/generate-caption-vectors.ts
 *
 * Text is measured by a synthetic canvas whose `measureText` width depends on
 * every part of the font string and the letter spacing (see `synth`), so the
 * vectors check what the layout asks the canvas, not a font. The corpus test
 * checks real measurements. PM_CAPTION_SCALE multiplies the number of cases;
 * PM_CAPTION_SEED draws another set; PM_CAPTION_OUT writes elsewhere.
 */
import { writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { cueIndexAt, cueTokens, cueWords, normalizeCaptionStyle, presetStyle, textTokens, CAPTION_PRESETS } from '../../../../../apps/desktop/src/renderer/src/captions/model';
import { layoutCaption, placeBlock, wordStates } from '../../../../../apps/desktop/src/renderer/src/captions/layout';
import { createCaptionRasterizer } from '../../../../../apps/desktop/src/renderer/src/captions/render';

const SCALE = Number(process.env.PM_CAPTION_SCALE ?? 1);
const OUT = process.env.PM_CAPTION_OUT ?? new URL('./caption-vectors.json.gz', import.meta.url).pathname;

let seed = Number(process.env.PM_CAPTION_SEED ?? 0xca97);
function rand(): number {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = <T>(items: readonly T[]): T => items[Math.floor(rand() * items.length)]!;
const chance = (p: number) => rand() < p;
const range = (a: number, b: number) => a + (b - a) * rand();
const int = (a: number, b: number) => Math.floor(range(a, b + 1));

const view = new DataView(new ArrayBuffer(8));
function hex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}
function enc(v: unknown): any {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  if (Array.isArray(v)) return { a: v.map(enc) };
  if (typeof v === 'object') {
    const o: Record<string, unknown> = {};
    for (const k of Object.keys(v as object).sort()) o[k] = enc((v as any)[k]);
    return { o };
  }
  return { s: String(v) };
}
function run(fn: () => unknown): any {
  try { return enc(fn()); } catch (e) { return { throw: (e as Error)?.message ?? 'Error' }; }
}

/* ── synthetic canvas ───────────────────────────────────────────────────── */
/** `measureText(text).width` for a font string and letter spacing. */
function synth(font: string, letterSpacing: string, text: string): number {
  const m = /^(italic )?(\S+) (\S+)px "((?:[^"\\]|\\.)*)"/.exec(font);
  if (!m) throw new Error(`unexpected font ${font}`);
  const italic = !!m[1], weight = Number(m[2]), size = Number(m[3]), family = m[4]!.replace(/\\(.)/g, '$1');
  const ls = parseFloat(letterSpacing) || 0;
  const k = size / 16 * (italic ? 1.05 : 1) * (1 + (weight - 400) / 4000) * (1 + family.length / 100);
  let w = 0, n = 0;
  for (const ch of text) { w += ((ch.codePointAt(0)! % 13) + 4) * k; n++; }
  return w + ls * n;
}
const cssString = (value: unknown) => String(value ?? '').replace(/\\/g, '\\\\').replace(/"/g, '\\"');
/** raster.ts `fontStr` for a style without variation axes. */
const fontStr = (d: any) => {
  const families = [d.font, 'SF Pro Display'].filter(Boolean).map(family => `"${cssString(family)}"`).join(', ');
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px ${families}, -apple-system, sans-serif`;
};
function context() {
  return {
    font: '10px sans-serif',
    letterSpacing: '0px',
    textBaseline: 'alphabetic',
    measureText(text: string) { return { width: synth(this.font, this.letterSpacing, text) }; },
  };
}
const canvas = () => ({ width: 8, height: 8, getContext: () => context() }) as any;

/* ── random inputs ──────────────────────────────────────────────────────── */
const FONTS = ['SF Pro Display', 'Inter', 'Geist Mono', '', 'Noto Sans "CJK"', 'A\\B'];
const WORDS = ['the', 'quick', 'brown', 'fox', 'jumps', 'over', 'lazy', 'dog', 'Hello,', 'world!', 'iPhone', 'émigré', 'straße', 'ΣΊΣΥΦΟΣ', 'İstanbul', '👍🏽', '👨‍👩‍👧', 'á', '1,234.56', '—', '(quoted)', '"hi"'];
const CJK = ['日本語の', 'テキスト', '中文字幕', '「こんにちは」', '你好，世界。', 'ー', '、', '漢字', '𠀋𠂉', 'カタカナ！'];
const SPACES = [' ', ' ', ' ', '  ', '\t', '　', ' ', '﻿', ' ', ' ', '\u0085', '᠎'];
function phrase(): string {
  const n = int(0, 14);
  let s = '';
  for (let i = 0; i < n; i++) {
    const w = chance(0.25) ? pick(CJK) : pick(WORDS);
    s += (i && !(chance(0.3) && /[　-鿿]/.test(w)) ? pick(SPACES) : '') + w;
  }
  return s;
}
const cueText = () => Array.from({ length: int(1, chance(0.3) ? 3 : 1) }, phrase).join('\n');

function rawStyle(): any {
  const style: any = {};
  const maybe = (key: string, value: () => unknown) => { if (chance(0.55)) style[key] = value(); };
  maybe('preset', () => pick(['classic', 'boxed', 'pop', 'custom', 'nope', 3]));
  maybe('font', () => pick([...FONTS, '  ', 7, null]));
  maybe('weight', () => pick([100, 400, 600, 900, 1000, 1200, 0, 50.6, '700', ' ', 'bold', null, true]));
  maybe('italic', () => pick([true, false, 'true', 1]));
  maybe('size', () => pick([range(4, 120), 2, 2000, '48', '', 'x', null, [64]]));
  maybe('tracking', () => pick([0, range(-5, 12), -80, 300, '2', null]));
  maybe('leading', () => pick([range(0.8, 2), 0.1, 9, '1.4', 'a']));
  maybe('fill', () => pick(['#ffffff', '#12345678', '#FFF', 'red', 5]));
  maybe('stroke', () => pick([0, range(0, 10), -3, 500]));
  maybe('strokeColor', () => pick(['#000000', '#00000099', 'black']));
  maybe('box', () => pick([true, false, 'true', 1]));
  maybe('boxColor', () => pick(['#000000', '#ffffffaa', '#zzzzzz']));
  maybe('boxOpacity', () => pick([range(0, 100), -5, 150]));
  maybe('boxPadding', () => pick([range(0, 30), -2, 900, '12']));
  maybe('boxRadius', () => pick([range(0, 20), 1000]));
  maybe('placement', () => pick(['bottom', 'middle', 'top', 'side', null]));
  maybe('align', () => pick(['left', 'center', 'right', 'justify']));
  maybe('offsetX', () => pick([0, range(-300, 300), 1e6, '15']));
  maybe('offsetY', () => pick([0, range(-300, 300), -1e6, '-20']));
  maybe('safeMargin', () => pick([range(0, 20), 60, -1]));
  maybe('maxWidth', () => pick([range(20, 100), 5, 150, '80']));
  maybe('maxLines', () => pick([1, 2, 3, 6, 2.5, 0, 9, '2', null]));
  maybe('textCase', () => pick(['original', 'upper', 'lower', 'title']));
  maybe('highlight', () => pick([true, false, 'yes']));
  maybe('highlightColor', () => pick(['#FFD43B', '#24d5ff', 'gold']));
  maybe('highlightScale', () => pick([100, range(50, 300), 10, 400, '108']));
  maybe('wordAnimation', () => pick(['none', 'pop', 'fade', 'bounce', null]));
  return style;
}

/** A normalized style: a preset at a composition height, then random edits. */
function style(): any {
  const base = chance(0.4) ? presetStyle(pick(CAPTION_PRESETS).id, pick([720, 1080, 1920])) : normalizeCaptionStyle({});
  return normalizeCaptionStyle({ ...base, ...(chance(0.7) ? rawStyle() : {}) });
}

/**
 * A style the rasterizer uses as stored (it has `maxLines`): numbers within
 * what the canvas font accepts (weights 1 to 1000, positive sizes), and
 * booleans, enums, and numbers that normalization would change. Strings in
 * number fields concatenate where the legacy code adds them (see
 * docs/engine/pm-eval-layers.md), so they are left out.
 */
function storedStyle(): any {
  const s: any = { ...style() };
  const raw = rawStyle();
  for (const key of Object.keys(raw)) {
    const v = raw[key];
    if (['weight', 'size', 'tracking', 'leading', 'stroke', 'boxOpacity', 'boxPadding', 'boxRadius', 'offsetX', 'offsetY', 'safeMargin', 'maxWidth', 'maxLines', 'highlightScale'].includes(key)) {
      if (typeof v === 'number' && (key !== 'weight' || (v >= 1 && v <= 1000)) && (key !== 'size' || v > 0)) s[key] = v;
    } else if (key !== 'font' || typeof v === 'string') s[key] = v;
  }
  if (chance(0.2)) delete s.font;
  return s;
}

function words(text: string, start: number, end: number): any[] | undefined {
  const tokens = cueTokens(text).map(t => t.text);
  const mode = pick(['none', 'match', 'match', 'chars', 'mismatch', 'empty', 'garbage']);
  if (mode === 'none') return undefined;
  if (mode === 'empty') return [];
  if (mode === 'garbage') return pick([[null], [{ text: 5, start: 0, end: 1 }], 'abc', [{ start: 1 }]]) as any;
  let list: string[] = tokens;
  if (mode === 'chars') list = Array.from(text.replace(/\s+/g, ''));
  if (mode === 'mismatch') list = [...tokens, 'extra'];
  let t = start;
  const span = (end - start) / Math.max(1, list.length);
  return list.map(w => {
    const a = t + range(-0.2, 0.3) * span, b = a + range(0.2, 1.2) * span;
    t += span;
    return { text: chance(0.1) ? w.toUpperCase() : w, start: a, end: b };
  });
}

function cues(): any[] {
  const out: any[] = [];
  let t = range(-0.5, 1);
  for (let i = 0, n = int(0, 5); i < n; i++) {
    const start = t, end = t + range(0.04, 3);
    const text = cueText();
    const cue: any = { id: `c${i}`, start, end, text };
    const w = words(text, start, end);
    if (w !== undefined) cue.words = w;
    out.push(cue);
    t = end + (chance(0.4) ? 0 : range(0, 1));
  }
  if (chance(0.05) && out.length) out[int(0, out.length - 1)].text = pick([5, null, ['a']]);
  if (chance(0.03)) out.push(null);
  return out;
}

const cases: any[] = [];
const add = (op: string, input: Record<string, unknown>, fn: () => unknown) => cases.push({ op, ...input, out: run(fn) });

for (let i = 0; i < 400 * SCALE; i++) {
  const raw = chance(0.1) ? pick([null, 5, 'x', [1], []]) : rawStyle();
  add('style', { raw }, () => normalizeCaptionStyle(raw));
}
for (let i = 0; i < 300 * SCALE; i++) {
  const text = chance(0.1) ? pick(['', ' ', '　', 'a', '。', '「', '「a」', 'x、y', 'ab\ud800c', '\udc00']) : phrase();
  add('tokens', { text }, () => textTokens(text));
}
for (let i = 0; i < 300 * SCALE; i++) {
  const text = cueText(), start = range(0, 5), end = start + range(0.04, 4);
  const cue: any = { id: 'c', start, end, text };
  const w = words(text, start, end);
  if (w !== undefined) cue.words = w;
  add('words', { cue }, () => cueWords(cue));
}
for (let i = 0; i < 500 * SCALE; i++) {
  const s = style(), text = cueText(), start = range(0, 5), end = start + range(0.04, 4);
  const cue: any = { id: 'c', start, end, text };
  const w = words(text, start, end);
  if (w !== undefined && Array.isArray(w) && w.every(x => x && typeof x.text === 'string')) cue.words = w;
  const width = pick([1920, 1080, 720, range(100, 4000)]);
  const applyCase = pick([undefined, true, false]);
  const measure = (t: string, size: number) => synth(fontStr({ ...s, size }), `${s.tracking * size / s.size}px`, t);
  add('layout', { cue, style: s, width, applyCase: applyCase ?? null }, () => layoutCaption(cue, s, measure, { compositionWidth: width, applyCase }));
}
for (let i = 0; i < 300 * SCALE; i++) {
  const s = style(), layout = { width: range(1, 2000), height: range(1, 400) }, w = range(100, 4000), h = range(100, 4000);
  add('place', { style: s, layout, w, h }, () => placeBlock(layout, s, w, h));
}
for (let i = 0; i < 400 * SCALE; i++) {
  const s = style(), start = range(0, 3);
  const list = Array.from({ length: int(0, 6) }, (_, k) => ({ text: `w${k}`, start: start + k * range(0, 0.4), end: start + k * 0.4 + range(0, 0.6) }));
  const t = chance(0.2) ? pick([start, start + 0.18, start + 1 / 120, -1]) : start + range(-0.5, 3);
  add('states', { words: list, style: s, t: hex(t) }, () => wordStates(list, s, t));
}
for (let i = 0; i < 300 * SCALE; i++) {
  const list = cues().filter(c => c);
  const t = chance(0.3) && list.length ? pick(list.flatMap(c => [c.start, c.end])) : range(-1, 12);
  add('index', { cues: list, t: hex(t) }, () => cueIndexAt(list, t));
}
for (let i = 0; i < 800 * SCALE; i++) {
  const content: any = chance(0.03) ? pick([null, undefined, 5, { cues: 'x' }]) : { cues: cues() };
  if (content && typeof content === 'object' && Array.isArray(content.cues)) {
    const kind = pick(['normalized', 'normalized', 'stored', 'raw', 'missing']);
    if (kind === 'normalized') content.style = style();
    if (kind === 'stored') content.style = storedStyle();
    if (kind === 'raw') { content.style = rawStyle(); delete content.style.maxLines; }
  }
  const list = Array.isArray(content?.cues) ? content.cues.filter((c: any) => c && typeof c === 'object') : [];
  const local = chance(0.6) && list.length ? (() => { const c = pick(list); return c.start + range(-0.1, 1.05) * (c.end - c.start); })() : range(-1, 12);
  const w = pick([1920, 1080, 3840, range(200, 3000)]), h = pick([1080, 1920, 720, range(200, 3000)]);
  // A fresh rasterizer per case: the legacy layout cache keys a cue by its
  // word count, not its word times, so a shared one answers from history.
  const raster = createCaptionRasterizer({ canvas, font: fontStr, fontReady: () => true });
  add('frame', { content: content === undefined ? null : content, local: hex(local), w, h }, () => {
    const f = raster.frame(content, local, w, h);
    if (!f) return { frame: null, idle: raster.idleBounds(content, w, h) };
    return {
      frame: {
        cue: content.cues.indexOf(f.cue),
        layout: f.layout,
        states: f.states,
        block: f.block,
        margin: f.margin,
      },
    };
  });
}

const json = JSON.stringify({ cases });
writeFileSync(OUT, gzipSync(json));
console.log(`wrote ${OUT}: ${cases.length} cases`);
