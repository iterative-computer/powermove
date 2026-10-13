/*
 * Caption block layout: line breaking at the style's maximum width and line
 * count, balanced lines, and word positions for highlight and animation.
 *
 * Pure apart from the injected `measure`, which must use the same canvas
 * font settings the rasterizer paints with, so preview and export agree.
 */
import { applyTextCase, cueWords, textTokens, type CaptionCue, type CaptionStyle, type CaptionWord } from './model';

export type Measure = (text: string, size: number) => number;

export interface LaidWord {
  /** Index into the cue's word list (timings). */
  index: number;
  text: string;
  /** Left edge relative to the block's left edge. */
  x: number;
  width: number;
}

export interface LaidLine { words: LaidWord[]; width: number; y: number }

export interface CaptionLayout {
  size: number;
  lineHeight: number;
  lines: LaidLine[];
  /** Text block, excluding the box padding. */
  width: number;
  height: number;
  words: CaptionWord[];
}

/** `gap`: a space precedes the token on its line (none between CJK characters). */
interface Token { index: number; text: string; width: number; breakBefore: boolean; gap: boolean }

function wrap(tokens: Token[], space: number, limit: number): Token[][] {
  const lines: Token[][] = [];
  let line: Token[] = [], width = 0;
  for (const token of tokens) {
    const next = line.length ? width + (token.gap ? space : 0) + token.width : token.width;
    if (line.length && (token.breakBefore || next > limit)) {
      lines.push(line); line = [token]; width = token.width;
    } else { line.push(token); width = next; }
  }
  if (line.length) lines.push(line);
  return lines;
}

const lineWidth = (line: Token[], space: number) => line.reduce((sum, token, index) => sum + token.width + (index && token.gap ? space : 0), 0);

export interface LayoutOptions {
  compositionWidth: number;
  /** Text case already applied to `text` when false. */
  applyCase?: boolean;
}

export function layoutCaption(cue: CaptionCue, style: CaptionStyle, measure: Measure, options: LayoutOptions): CaptionLayout {
  const text = options.applyCase === false ? cue.text : applyTextCase(cue.text, style.textCase);
  const words = cueWords({ ...cue, text });
  const paragraphs = text.split('\n');
  const padding = style.box ? style.boxPadding * 2 : 0;
  const limit = Math.max(1, options.compositionWidth * style.maxWidth / 100 - padding);

  const build = (size: number) => {
    let index = 0;
    const tokens: Token[] = [];
    for (const paragraph of paragraphs) {
      textTokens(paragraph).forEach((token, position) => {
        tokens.push({ index: index++, text: token.text, width: measure(token.text, size), breakBefore: position === 0 && tokens.length > 0, gap: token.gap });
      });
    }
    return { tokens, space: measure(' ', size) };
  };

  // Shrink to fit the line budget, as broadcast subtitlers do, never below
  // 60% of the styled size.
  let size = style.size;
  let { tokens, space } = build(size);
  let lines = wrap(tokens, space, limit);
  for (let step = 0; lines.length > style.maxLines && step < 8; step++) {
    size = Math.max(style.size * 0.6, size * 0.94);
    ({ tokens, space } = build(size));
    lines = wrap(tokens, space, limit);
    if (size <= style.size * 0.6) break;
  }

  // Balance: the narrowest width that keeps the same number of lines, so a
  // two-line caption reads as two even lines rather than one long, one short.
  if (lines.length > 1 && !tokens.some(token => token.breakBefore)) {
    let lo = Math.max(...tokens.map(token => token.width)), hi = limit;
    for (let step = 0; step < 12 && hi - lo > 1; step++) {
      const mid = (lo + hi) / 2;
      if (wrap(tokens, space, mid).length <= lines.length) hi = mid; else lo = mid;
    }
    lines = wrap(tokens, space, hi);
  }

  const lineHeight = size * style.leading;
  const widths = lines.map(line => lineWidth(line, space));
  const width = Math.max(1, ...widths);
  const laid: LaidLine[] = lines.map((line, row) => {
    const w = widths[row]!;
    let x = style.align === 'left' ? 0 : style.align === 'right' ? width - w : (width - w) / 2;
    const placed = line.map((token, position) => {
      if (position && token.gap) x += space;
      const word = { index: token.index, text: token.text, x, width: token.width };
      x += token.width;
      return word;
    });
    return { words: placed, width: w, y: row * lineHeight };
  });
  return { size, lineHeight, lines: laid, width, height: Math.max(lineHeight, lines.length * lineHeight), words };
}

export interface BlockPlacement {
  /** Block rectangle in layer space (origin at the composition centre), including box padding. */
  x0: number; y0: number; x1: number; y1: number;
}

/** Where the caption block sits: inside the title-safe area at the bottom,
    middle or top, aligned within the max-width column, then offset. */
export function placeBlock(layout: { width: number; height: number }, style: CaptionStyle, compositionWidth: number, compositionHeight: number): BlockPlacement {
  const padding = style.box ? style.boxPadding : 0;
  const w = layout.width + padding * 2, h = layout.height + padding * 2;
  const column = compositionWidth * style.maxWidth / 100;
  const safe = compositionHeight * style.safeMargin / 100;
  const x0 = style.align === 'left' ? -column / 2 : style.align === 'right' ? column / 2 - w : -w / 2;
  const y0 = style.placement === 'top' ? -compositionHeight / 2 + safe
    : style.placement === 'middle' ? -h / 2
    : compositionHeight / 2 - safe - h;
  return { x0: x0 + style.offsetX, y0: y0 + style.offsetY, x1: x0 + style.offsetX + w, y1: y0 + style.offsetY + h };
}

/* ── word state at a moment ─────────────────────────────── */

export interface WordState { alpha: number; scale: number; highlighted: boolean }

const POP = 0.18, FADE = 0.16, HIGHLIGHT_IN = 0.08;
const easeOutBack = (t: number) => { const c = 1.6; return 1 + (c + 1) * Math.pow(t - 1, 3) + c * Math.pow(t - 1, 2); };

/** Per-word appearance at layer time `t`. Quantised to 1/60 s so the
    rasterizer can cache identical frames. */
export function wordStates(words: readonly CaptionWord[], style: CaptionStyle, t: number): WordState[] {
  const time = Math.round(t * 60) / 60;
  return words.map(word => {
    let alpha = 1, scale = 1;
    const since = time - word.start;
    if (style.wordAnimation === 'fade') alpha = since < 0 ? 0 : Math.min(1, since / FADE);
    if (style.wordAnimation === 'pop') {
      alpha = since < 0 ? 0 : Math.min(1, since / (POP / 2));
      scale = since < 0 ? 0.7 : since >= POP ? 1 : 0.7 + 0.3 * easeOutBack(since / POP);
    }
    const highlighted = style.highlight && time >= word.start && time < word.end;
    if (highlighted && style.highlightScale !== 100) {
      const k = Math.min(1, Math.max(0, since / HIGHLIGHT_IN));
      scale *= 1 + (style.highlightScale / 100 - 1) * k;
    }
    return { alpha: Math.round(alpha * 100) / 100, scale: Math.round(scale * 1000) / 1000, highlighted };
  });
}

/** A short signature of the word states, for raster cache keys. */
export function stateKey(states: readonly WordState[]): string {
  let key = '';
  for (const state of states) key += `${state.highlighted ? 'h' : ''}${state.alpha === 1 ? '' : state.alpha}${state.scale === 1 ? '' : `x${state.scale}`};`;
  return key;
}
