/*
 * Captions model: presets, normalisation and the pure cue operations the
 * edit commands, the timeline and the Captions panel share. Every operation
 * returns a new, sorted, non-overlapping cue list; nothing here touches the
 * project, history or rendering.
 */
import {
  CAPTION_ALIGNS,
  CAPTION_PLACEMENTS,
  CAPTION_TEXT_CASES,
  CAPTION_WORD_ANIMATIONS,
  type CaptionCue,
  type CaptionStyle,
  type CaptionWord,
  type CaptionsContent
} from '../core/types/captions';

export type { CaptionCue, CaptionStyle, CaptionWord, CaptionsContent } from '../core/types/captions';

/** Shortest cue the editor keeps: one frame at 30 fps, rounded. */
export const MIN_CUE_DURATION = 0.04;
const EPS = 1e-6;

type UnknownRecord = Record<string, unknown>;
const isRecord = (value: unknown): value is UnknownRecord =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const finite = (value: unknown, fallback: number): number => {
  const number = typeof value === 'string' && value.trim() === '' ? NaN : Number(value);
  return Number.isFinite(number) ? number : fallback;
};
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
const round = (value: number) => Math.round(value * 1e6) / 1e6;
const HEX = /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i;
const color = (value: unknown, fallback: string) => typeof value === 'string' && HEX.test(value) ? value.toUpperCase() : fallback;
const oneOf = <T extends string>(values: readonly T[], value: unknown, fallback: T): T =>
  values.includes(value as T) ? value as T : fallback;

let counter = 0;
/** Cue ids only need to be unique within their layer. */
export function cueId(): string {
  counter = (counter + 1) % 1_679_616;
  return `c${Date.now().toString(36)}${counter.toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/* ── presets ─────────────────────────────────────────────── */

export const DEFAULT_CAPTION_STYLE: CaptionStyle = {
  preset: 'classic',
  font: 'SF Pro Display',
  weight: 600,
  italic: false,
  size: 54,
  tracking: 0,
  leading: 1.2,
  fill: '#FFFFFF',
  stroke: 3,
  strokeColor: '#000000',
  box: false,
  boxColor: '#000000',
  boxOpacity: 72,
  boxPadding: 14,
  boxRadius: 8,
  placement: 'bottom',
  align: 'center',
  offsetX: 0,
  offsetY: 0,
  safeMargin: 8,
  maxWidth: 80,
  maxLines: 2,
  textCase: 'original',
  highlight: false,
  highlightColor: '#FFD43B',
  highlightScale: 100,
  wordAnimation: 'none'
};

export interface CaptionPreset {
  id: string;
  label: string;
  description: string;
  style: Partial<CaptionStyle>;
}

/* Six looks that cover broadcast, streaming, short-form and editorial work.
   Each preset is a complete style so switching presets never keeps a stray
   value from the previous one. Sizes are for a 1080p frame and scale with
   the composition height when applied. */
export const CAPTION_PRESETS: readonly CaptionPreset[] = [
  {
    id: 'classic', label: 'Classic',
    description: 'White subtitles with a fine outline, two lines at the bottom.',
    style: {}
  },
  {
    id: 'boxed', label: 'Boxed',
    description: 'White text on a soft dark plate, as on streaming services.',
    style: { weight: 500, size: 48, stroke: 0, box: true, boxColor: '#000000', boxOpacity: 72, boxPadding: 14, boxRadius: 10 }
  },
  {
    id: 'whisper', label: 'Whisper',
    description: 'A small, light line in the lower third. Unobtrusive for landscape footage.',
    style: { weight: 400, size: 38, tracking: 0.5, leading: 1.3, stroke: 2, strokeColor: '#00000099', maxWidth: 70, maxLines: 1, safeMargin: 7 }
  },
  {
    id: 'spotlight', label: 'Spotlight',
    description: 'Bold centred line; the spoken word lights up.',
    style: { weight: 800, size: 64, leading: 1.1, stroke: 4, placement: 'middle', offsetY: 220, maxWidth: 70, highlight: true, highlightColor: '#24D5FF', highlightScale: 108 }
  },
  {
    id: 'pop', label: 'Pop',
    description: 'Heavy uppercase words that pop in as they are spoken. Made for vertical video.',
    style: { weight: 900, size: 82, leading: 1.05, stroke: 7, placement: 'middle', offsetY: 160, maxWidth: 76, maxLines: 2, textCase: 'upper', highlight: true, highlightColor: '#FFD43B', highlightScale: 116, wordAnimation: 'pop' }
  },
  {
    id: 'paper', label: 'Paper',
    description: 'Dark type on a white card. Calm and editorial.',
    style: { weight: 500, size: 46, leading: 1.25, fill: '#151515', stroke: 0, box: true, boxColor: '#FFFFFF', boxOpacity: 94, boxPadding: 16, boxRadius: 6 }
  }
];

export function captionPreset(id: string): CaptionPreset | null {
  return CAPTION_PRESETS.find(preset => preset.id === id) ?? null;
}

/** The complete style a preset applies, sized for a composition height. */
export function presetStyle(id: string, compositionHeight = 1080): CaptionStyle {
  const preset = captionPreset(id) ?? CAPTION_PRESETS[0]!;
  const scale = compositionHeight > 0 ? compositionHeight / 1080 : 1;
  const style = { ...DEFAULT_CAPTION_STYLE, ...preset.style, preset: preset.id };
  for (const key of ['size', 'stroke', 'boxPadding', 'boxRadius', 'offsetX', 'offsetY'] as const) {
    style[key] = Math.round(style[key] * scale * 100) / 100;
  }
  return style;
}

/** Where a caption block sits. A preset brings its own position, but moving
    a layer (a second language on top) keeps its look and preset label, and a
    moved layer stays where it is when it switches presets. */
export const CAPTION_POSITION_KEYS = ['placement', 'offsetX', 'offsetY'] as const;
const samePosition = (a: CaptionStyle, b: CaptionStyle) => CAPTION_POSITION_KEYS.every(key => a[key] === b[key]);

/** A style patch keeps its preset label only while its look still matches it. */
export function patchStyle(style: CaptionStyle, patch: Partial<CaptionStyle>, compositionHeight = 1080): CaptionStyle {
  if (typeof patch.preset === 'string' && patch.preset !== style.preset && captionPreset(patch.preset)) {
    const { preset, ...rest } = patch;
    const moved = !samePosition(style, presetStyle(style.preset, compositionHeight));
    const position = moved ? Object.fromEntries(CAPTION_POSITION_KEYS.map(key => [key, style[key]])) : {};
    return normalizeCaptionStyle({ ...presetStyle(preset, compositionHeight), ...position, ...rest });
  }
  const next = normalizeCaptionStyle({ ...style, ...patch });
  if (patch.preset === undefined && next.preset !== 'custom') {
    const reference = presetStyle(next.preset, compositionHeight);
    const position = new Set<string>(CAPTION_POSITION_KEYS);
    const changed = (Object.keys(patch) as Array<keyof CaptionStyle>)
      .some(key => key !== 'preset' && !position.has(key) && reference[key] !== next[key]);
    if (changed) next.preset = 'custom';
  }
  return next;
}

export function normalizeCaptionStyle(raw: unknown): CaptionStyle {
  const source = isRecord(raw) ? raw : {};
  const base = DEFAULT_CAPTION_STYLE;
  return {
    preset: typeof source.preset === 'string' && (source.preset === 'custom' || captionPreset(source.preset)) ? source.preset : base.preset,
    font: typeof source.font === 'string' && source.font.trim() ? source.font : base.font,
    weight: clamp(Math.round(finite(source.weight, base.weight)), 100, 1000),
    italic: source.italic === true,
    size: clamp(finite(source.size, base.size), 4, 1000),
    tracking: clamp(finite(source.tracking, base.tracking), -50, 200),
    leading: clamp(finite(source.leading, base.leading), 0.5, 4),
    fill: color(source.fill, base.fill),
    stroke: clamp(finite(source.stroke, base.stroke), 0, 100),
    strokeColor: color(source.strokeColor, base.strokeColor),
    box: source.box === true,
    boxColor: color(source.boxColor, base.boxColor),
    boxOpacity: clamp(finite(source.boxOpacity, base.boxOpacity), 0, 100),
    boxPadding: clamp(finite(source.boxPadding, base.boxPadding), 0, 400),
    boxRadius: clamp(finite(source.boxRadius, base.boxRadius), 0, 400),
    placement: oneOf(CAPTION_PLACEMENTS, source.placement, base.placement),
    align: oneOf(CAPTION_ALIGNS, source.align, base.align),
    offsetX: clamp(finite(source.offsetX, 0), -100_000, 100_000),
    offsetY: clamp(finite(source.offsetY, 0), -100_000, 100_000),
    safeMargin: clamp(finite(source.safeMargin, base.safeMargin), 0, 45),
    maxWidth: clamp(finite(source.maxWidth, base.maxWidth), 10, 100),
    maxLines: clamp(Math.round(finite(source.maxLines, base.maxLines)), 1, 6),
    textCase: oneOf(CAPTION_TEXT_CASES, source.textCase, base.textCase),
    highlight: source.highlight === true,
    highlightColor: color(source.highlightColor, base.highlightColor),
    highlightScale: clamp(finite(source.highlightScale, base.highlightScale), 50, 300),
    wordAnimation: oneOf(CAPTION_WORD_ANIMATIONS, source.wordAnimation, base.wordAnimation)
  };
}

/* ── cues ────────────────────────────────────────────────── */

function normalizeWords(raw: unknown, start: number, end: number): CaptionWord[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const words: CaptionWord[] = [];
  for (const item of raw) {
    if (!isRecord(item) || typeof item.text !== 'string' || !item.text.trim()) continue;
    let a = finite(item.start, NaN), b = finite(item.end, NaN);
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
    if (b < a) [a, b] = [b, a];
    a = clamp(a, start, end); b = clamp(b, a, end);
    words.push({ text: item.text.trim(), start: round(a), end: round(b) });
  }
  words.sort((x, y) => x.start - y.start);
  return words.length ? words : undefined;
}

/** Text normalisation shared by every importer: CRLF, stray whitespace and
    empty lines (an empty line ends an SRT block, so it can never be kept). */
export function cleanCueText(value: unknown): string {
  return String(value ?? '')
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map(line => line.replace(/[\t ]+/g, ' ').trim())
    .filter(Boolean)
    .join('\n');
}

/**
 * Sort, validate and de-overlap cues. Overlapping cues are resolved the way
 * an editor expects: a cue that starts inside the previous one shortens it;
 * cues starting at the same moment (two speakers) become one two-line cue.
 * Empty cues are dropped. `makeId` fills missing or duplicate ids.
 */
export function normalizeCues(raw: unknown, makeId: () => string = cueId): CaptionCue[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const cues: CaptionCue[] = [];
  for (const item of raw) {
    if (!isRecord(item)) continue;
    let start = finite(item.start, NaN), end = finite(item.end, NaN);
    if (!Number.isFinite(start) || !Number.isFinite(end)) continue;
    if (end < start) [start, end] = [end, start];
    start = Math.max(0, start); end = Math.max(0, end);
    if (end - start < MIN_CUE_DURATION - EPS) continue;
    const text = cleanCueText(item.text);
    if (!text) continue;
    let id = typeof item.id === 'string' && item.id ? item.id : '';
    if (!id || seen.has(id)) id = makeId();
    seen.add(id);
    const cue: CaptionCue = { id, start: round(start), end: round(end), text };
    const words = normalizeWords(item.words, cue.start, cue.end);
    if (words) cue.words = words;
    cues.push(cue);
  }
  // Stable: cues that start together keep their file order (speaker order).
  cues.sort((a, b) => a.start - b.start);
  const out: CaptionCue[] = [];
  for (const cue of cues) {
    const previous = out[out.length - 1];
    if (previous && cue.start < previous.end - EPS) {
      if (cue.start - previous.start < MIN_CUE_DURATION) {
        // Same start: one cue carrying both lines.
        previous.text = `${previous.text}\n${cue.text}`;
        previous.end = Math.max(previous.end, cue.end);
        if (previous.words || cue.words) {
          previous.words = [...(previous.words ?? []), ...(cue.words ?? [])].sort((a, b) => a.start - b.start);
        }
        continue;
      }
      previous.end = cue.start;
      if (previous.words) previous.words = previous.words.filter(word => word.start < previous.end).map(word => ({ ...word, end: Math.min(word.end, previous.end) }));
      if (previous.words && !previous.words.length) delete previous.words;
    }
    out.push(cue);
  }
  return out;
}

export function normalizeCaptionsContent(raw: unknown, makeId: () => string = cueId): CaptionsContent {
  const source = isRecord(raw) ? raw : {};
  const content: CaptionsContent = {
    ...source,
    cues: normalizeCues(source.cues, makeId),
    style: normalizeCaptionStyle(source.style)
  };
  if (typeof source.language === 'string' && /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/.test(source.language)) content.language = source.language;
  else delete content.language;
  return content;
}

/** Index of the cue showing at layer time `t`, or -1. Binary search. */
export function cueIndexAt(cues: readonly CaptionCue[], t: number): number {
  let lo = 0, hi = cues.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const cue = cues[mid]!;
    if (t < cue.start) hi = mid - 1;
    else if (t >= cue.end) lo = mid + 1;
    else return mid;
  }
  return -1;
}

/** First cue index whose end is after `t` (cues are sorted and disjoint). */
export function firstCueEndingAfter(cues: readonly CaptionCue[], t: number): number {
  let lo = 0, hi = cues.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid]!.end <= t) lo = mid + 1; else hi = mid;
  }
  return lo;
}

/** Half-open index range of the cues that intersect [t0, t1). */
export function cueRange(cues: readonly CaptionCue[], t0: number, t1: number): [number, number] {
  const first = firstCueEndingAfter(cues, t0);
  let lo = first, hi = cues.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid]!.start < t1) lo = mid + 1; else hi = mid;
  }
  return [first, lo];
}

export function captionsEnd(cues: readonly CaptionCue[]): number {
  return cues.length ? cues[cues.length - 1]!.end : 0;
}

export function applyTextCase(text: string, textCase: CaptionStyle['textCase']): string {
  if (textCase === 'upper') return text.toLocaleUpperCase();
  if (textCase === 'lower') return text.toLocaleLowerCase();
  return text;
}

/* ── text units ──────────────────────────────────────────
   Chinese and Japanese are written without spaces between words: transcript
   tokens join without a space, a line may break between any two characters,
   and each character takes about two Latin characters of line length.
   (Korean is written with spaces and follows the Latin rules.) */

const CJK = /[\u2E80-\u2FFF\u3000-\u303F\u3040-\u30FF\u31F0-\u31FF\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF\uFF00-\uFFEF\u{20000}-\u{2FFFF}]/u;
/** Punctuation that may not start a line; it stays with the character before. */
const CJK_CLOSING = /[、。，．！？：；」』）】〕〉》〙〗〛…‥・ーぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々〻!?.,;:)\]}%'"”’]/u;
/** Punctuation that may not end a line; it stays with the character after. */
const CJK_OPENING = /[「『（【〔〈《〘〖〚(\[{“‘]/u;

export const isCjkChar = (char: string | undefined) => !!char && CJK.test(char);

/** Words as cue text: spaced, except where Chinese or Japanese meet. */
export function joinWords(texts: readonly string[]): string {
  let out = '';
  for (const raw of texts) {
    const text = raw.trim();
    if (!text) continue;
    if (out && !isCjkChar(Array.from(out).at(-1)) && !isCjkChar(Array.from(text)[0])) out += ' ';
    out += text;
  }
  return out;
}

/** Line length for segmentation: CJK characters count double. */
export function displayLength(text: string): number {
  let length = 0;
  for (const char of text) length += isCjkChar(char) ? 2 : 1;
  return length;
}

export interface TextToken {
  text: string;
  /** A space separates this token from the previous one on the same line. */
  gap: boolean;
}

/** Units a line may break between: space-separated words, and single
    Chinese/Japanese characters (closing punctuation stays with the
    character before it, opening punctuation with the one after). */
export function textTokens(paragraph: string): TextToken[] {
  const tokens: TextToken[] = [];
  for (const chunk of paragraph.split(/\s+/).filter(Boolean)) {
    let current = '', gap = tokens.length > 0, opening = false;
    const push = () => { if (current) { tokens.push({ text: current, gap }); gap = false; current = ''; } };
    for (const char of chunk) {
      const cjk = isCjkChar(char), last = Array.from(current).at(-1);
      if (!current || opening) current += char;
      else if (CJK_CLOSING.test(char) && (cjk || isCjkChar(last))) current += char;
      else if (cjk || isCjkChar(last)) { push(); current = char; }
      else current += char;
      opening = CJK_OPENING.test(char);
    }
    push();
  }
  return tokens;
}

/** Every token of cue text, paragraph after paragraph, as layout numbers them. */
export function cueTokens(text: string): TextToken[] {
  return text.split('\n').flatMap(textTokens);
}

/**
 * Word timings for highlight and word animation, one per layout token.
 * Transcribed cues carry real timings: they are used as they are when they
 * line up with the tokens, and spread over the tokens by character position
 * when they do not (Chinese/Japanese words cover several characters).
 * Otherwise each token gets a share of the cue proportional to its length,
 * which reads naturally for imported subtitles.
 */
export function cueWords(cue: CaptionCue): CaptionWord[] {
  const tokens = cueTokens(cue.text).map(token => token.text);
  if (cue.words?.length && cue.words.length === tokens.length) {
    return cue.words.map((word, index) => ({ ...word, text: tokens[index]! }));
  }
  const mapped = cue.words?.length ? wordsByCharacter(cue.words, tokens) : null;
  if (mapped) return mapped;
  const total = tokens.reduce((sum, token) => sum + token.length + 1, 0) || 1;
  const span = cue.end - cue.start;
  let at = cue.start;
  return tokens.map(token => {
    const length = span * (token.length + 1) / total;
    const word = { text: token, start: round(at), end: round(at + length) };
    at += length;
    return word;
  });
}

/** Token timings from word timings that spell the same characters. */
function wordsByCharacter(words: readonly CaptionWord[], tokens: readonly string[]): CaptionWord[] | null {
  const squash = (text: string) => Array.from(text.replace(/\s+/g, '').toLocaleLowerCase());
  const owner: number[] = [];
  words.forEach((word, index) => { for (const _ of squash(word.text)) owner.push(index); });
  const characters = tokens.flatMap(token => squash(token));
  if (characters.length !== owner.length || squash(words.map(word => word.text).join('')).join('') !== characters.join('')) return null;
  // Characters of one word share its time evenly.
  const share = new Map<number, number>();
  for (const index of owner) share.set(index, (share.get(index) ?? 0) + 1);
  const seen = new Map<number, number>();
  let position = 0;
  return tokens.map(token => {
    const count = squash(token).length;
    let start = Infinity, end = -Infinity;
    for (let i = 0; i < count; i++, position++) {
      const index = owner[position]!, word = words[index]!;
      const n = share.get(index)!, k = seen.get(index) ?? 0;
      seen.set(index, k + 1);
      const step = (word.end - word.start) / n;
      start = Math.min(start, word.start + step * k);
      end = Math.max(end, word.start + step * (k + 1));
    }
    return { text: token, start: round(start), end: round(end) };
  });
}

/* ── operations ──────────────────────────────────────────── */

export interface CuePatch {
  id: string;
  start?: number;
  end?: number;
  text?: string;
  words?: CaptionWord[] | null;
}

/** Apply patches, then keep every untouched neighbour where it was: a patched
    cue is clamped between its neighbours instead of pushing them. */
export function updateCues(cues: readonly CaptionCue[], patches: readonly CuePatch[]): CaptionCue[] {
  const byId = new Map(patches.map(patch => [patch.id, patch]));
  /* Untouched cues are shared, not copied: cue objects are never mutated in
     place, and a drag on a long transcript patches one or two of thousands. */
  const out = cues.slice();
  const touched: number[] = [];
  for (let index = 0; index < out.length; index++) {
    const patch = byId.get(out[index]!.id);
    if (!patch) continue;
    const source = out[index]!;
    const cue: CaptionCue = { ...source, ...(source.words ? { words: source.words.map(word => ({ ...word })) } : {}) };
    out[index] = cue;
    touched.push(index);
    if (patch.text !== undefined) {
      const text = cleanCueText(patch.text);
      if (text !== cue.text) {
        cue.text = text;
        // Edited words no longer match their timings.
        if (patch.words === undefined) delete cue.words;
      }
    }
    if (patch.words === null) delete cue.words;
    const previous = out[index - 1], next = out[index + 1];
    const lower = previous && !byId.has(previous.id) ? previous.end : 0;
    const upper = next && !byId.has(next.id) ? next.start : Infinity;
    let start = patch.start !== undefined ? finite(patch.start, cue.start) : cue.start;
    let end = patch.end !== undefined ? finite(patch.end, cue.end) : cue.end;
    start = clamp(start, lower, upper - MIN_CUE_DURATION);
    end = clamp(end, start + MIN_CUE_DURATION, Math.max(start + MIN_CUE_DURATION, upper));
    const shift = Math.max(0, start) - cue.start;
    // A cue moved as a whole carries its word timings with it.
    if (cue.words && patch.words === undefined && shift && Math.abs(end - cue.end - shift) < 1e-6) {
      cue.words = cue.words.map(word => ({ ...word, start: word.start + shift, end: word.end + shift }));
    }
    cue.start = round(Math.max(0, start));
    cue.end = round(end);
    if (Array.isArray(patch.words)) cue.words = patch.words.map(word => ({ ...word }));
    if (cue.words) {
      const words = normalizeWords(cue.words, cue.start, cue.end);
      if (words) cue.words = words; else delete cue.words;
    }
  }
  return stillNormal(out, touched) ? out : normalizeCues(out.filter(cue => cue.text), cueId);
}

/** Whether the cues patched at `touched` still leave the list sorted,
    disjoint and valid, so the full normalisation pass can be skipped. */
function stillNormal(cues: readonly CaptionCue[], touched: readonly number[]): boolean {
  for (const index of touched) {
    const cue = cues[index]!, previous = cues[index - 1], next = cues[index + 1];
    if (!cue.text || cue.end - cue.start < MIN_CUE_DURATION - EPS) return false;
    if (previous && previous.end > cue.start + EPS) return false;
    if (next && cue.end > next.start + EPS) return false;
  }
  return true;
}

/** Add cues without moving existing ones; new cues are clipped to the gaps. */
export function insertCues(cues: readonly CaptionCue[], added: readonly Partial<CaptionCue>[], makeId: () => string = cueId): CaptionCue[] {
  const ids = new Set(cues.map(cue => cue.id));
  const fresh = normalizeCues(added.map(cue => ({ ...cue, id: cue.id && !ids.has(cue.id) ? cue.id : makeId() })), makeId);
  const out = cues.map(cue => ({ ...cue }));
  for (const cue of fresh) {
    const [first, last] = cueRange(out, cue.start, cue.end);
    // Fit the new cue into the gap around its start.
    const before = out.slice(0, first).at(-1);
    let start = Math.max(cue.start, before?.end ?? 0);
    let end = cue.end;
    if (first < last) {
      const overlap = out[first]!;
      if (overlap.start <= start + EPS) start = overlap.end;
      const next = out.slice(first).find(item => item.start >= start - EPS);
      if (next) end = Math.min(end, next.start);
    }
    if (end - start < MIN_CUE_DURATION - EPS) continue;
    const placed: CaptionCue = { ...cue, start: round(start), end: round(end) };
    if (placed.words) {
      const words = normalizeWords(placed.words, placed.start, placed.end);
      if (words) placed.words = words; else delete placed.words;
    }
    const at = firstCueEndingAfter(out, placed.start);
    out.splice(at, 0, placed);
  }
  return normalizeCues(out, makeId);
}

export function deleteCues(cues: readonly CaptionCue[], ids: readonly string[]): CaptionCue[] {
  const remove = new Set(ids);
  return cues.filter(cue => !remove.has(cue.id)).map(cue => ({ ...cue }));
}

/** Split one cue at layer time `at`. Text divides at word timings when
    present, otherwise at the word boundary nearest the time proportion. */
export function splitCue(cues: readonly CaptionCue[], id: string, at: number, makeId: () => string = cueId): { cues: CaptionCue[]; tailId: string | null } {
  const index = cues.findIndex(cue => cue.id === id);
  const cue = cues[index];
  if (!cue || at <= cue.start + MIN_CUE_DURATION - EPS || at >= cue.end - MIN_CUE_DURATION + EPS) {
    return { cues: cues.map(item => ({ ...item })), tailId: null };
  }
  const words = cueWords(cue);
  let cut = words.findIndex(word => (word.start + word.end) / 2 >= at);
  if (cut === -1) cut = words.length;
  // Each half keeps at least one word when the cue has two or more.
  if (words.length > 1) cut = clamp(cut, 1, words.length - 1);
  const head = words.slice(0, cut), tail = words.slice(cut);
  const lines = cue.text.split('\n');
  const join = (part: CaptionWord[]) => joinWords(part.map(word => word.text));
  const headText = words.length > 1 ? join(head) : lines[0] ?? cue.text;
  const tailText = words.length > 1 ? join(tail) : joinWords(lines.slice(1)) || cue.text;
  const tailId = makeId();
  const first: CaptionCue = { id: cue.id, start: cue.start, end: round(at), text: headText };
  const second: CaptionCue = { id: tailId, start: round(at), end: cue.end, text: tailText };
  if (cue.words?.length) {
    const realHead = cue.words.filter(word => (word.start + word.end) / 2 < at);
    const realTail = cue.words.filter(word => (word.start + word.end) / 2 >= at);
    const fit = (list: CaptionWord[], a: number, b: number) => normalizeWords(list, a, b);
    const headWords = fit(realHead, first.start, first.end), tailWords = fit(realTail, second.start, second.end);
    if (headWords) first.words = headWords;
    if (tailWords) second.words = tailWords;
  }
  const out = cues.map(item => ({ ...item }));
  out.splice(index, 1, first, second);
  return { cues: out, tailId };
}

/** Merge the run of cues from the first to the last listed id into one. */
export function mergeCues(cues: readonly CaptionCue[], ids: readonly string[]): { cues: CaptionCue[]; id: string | null } {
  const wanted = new Set(ids);
  const indices = cues.map((cue, index) => wanted.has(cue.id) ? index : -1).filter(index => index >= 0);
  if (indices.length < 2) return { cues: cues.map(cue => ({ ...cue })), id: null };
  const first = indices[0]!, last = indices[indices.length - 1]!;
  const run = cues.slice(first, last + 1);
  const merged: CaptionCue = {
    id: run[0]!.id,
    start: run[0]!.start,
    end: run[run.length - 1]!.end,
    text: joinWords(run.map(cue => joinWords(cue.text.split('\n'))))
  };
  if (run.every(cue => cue.words?.length)) merged.words = run.flatMap(cue => cue.words!.map(word => ({ ...word })));
  const out = cues.map(cue => ({ ...cue }));
  out.splice(first, run.length, merged);
  return { cues: out, id: merged.id };
}

/** The shift a group of cues may move by without crossing an unmoved
    neighbour or layer time zero: [min, max]. */
export function moveLimits(cues: readonly CaptionCue[], ids: readonly string[]): [number, number] {
  const moving = new Set(ids);
  let min = -Infinity, max = Infinity;
  cues.forEach((cue, index) => {
    if (!moving.has(cue.id)) return;
    min = Math.max(min, -cue.start);
    for (let j = index - 1; j >= 0; j--) if (!moving.has(cues[j]!.id)) { min = Math.max(min, cues[j]!.end - cue.start); break; }
    for (let j = index + 1; j < cues.length; j++) if (!moving.has(cues[j]!.id)) { max = Math.min(max, cues[j]!.start - cue.end); break; }
  });
  return [min, max];
}

export function moveCues(cues: readonly CaptionCue[], ids: readonly string[], by: number): CaptionCue[] {
  const [min, max] = moveLimits(cues, ids);
  const dt = clamp(by, Math.min(min, 0), Math.max(max, 0));
  const moving = new Set(ids);
  return cues.map(cue => {
    if (!moving.has(cue.id) || !dt) return { ...cue };
    const moved: CaptionCue = { ...cue, start: round(cue.start + dt), end: round(cue.end + dt) };
    if (cue.words) moved.words = cue.words.map(word => ({ ...word, start: round(word.start + dt), end: round(word.end + dt) }));
    return moved;
  });
}

/** Retime one edge of a cue, clamped between its neighbours. */
export function trimCue(cues: readonly CaptionCue[], id: string, edge: 'start' | 'end', time: number): CaptionCue[] {
  return updateCues(cues, [{ id, [edge]: time }]);
}

/** Shift every cue (all of them, so nothing collides) by `by` seconds. */
export function shiftAllCues(cues: readonly CaptionCue[], by: number): CaptionCue[] {
  return normalizeCues(cues.map(cue => ({
    ...cue, start: cue.start + by, end: cue.end + by,
    ...(cue.words ? { words: cue.words.map(word => ({ ...word, start: word.start + by, end: word.end + by })) } : {})
  })));
}

/**
 * Cues for a clip whose In point moves by `delta` seconds (trim-in, the tail
 * of a split). Cues keep their composition time: everything shifts by
 * -delta, cues that end before the new In point are dropped and the cue
 * spanning it is clipped. A negative delta (extending the In point) shifts
 * the cues later, so they stay where they were heard.
 */
export function rebaseCuesForStart(cues: readonly CaptionCue[], delta: number): CaptionCue[] {
  if (!Number.isFinite(delta) || !delta) return cues.map(cue => ({ ...cue }));
  return shiftAllCues(cues, -delta);
}

/** Cues for the head of a split at layer time `end`: later cues belong to the
    tail, and the cue spanning the cut is clipped to it. */
export function clipCuesToEnd(cues: readonly CaptionCue[], end: number): CaptionCue[] {
  if (!Number.isFinite(end)) return cues.map(cue => ({ ...cue }));
  return normalizeCues(cues.filter(cue => cue.start < end - EPS).map(cue => cue.end <= end ? cue : { ...cue, end }));
}
