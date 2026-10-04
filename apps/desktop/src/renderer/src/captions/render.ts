/*
 * Captions rasterizer. The compositor treats the result exactly like a text
 * layer's bitmap: one texture placed by the layer transform, so opacity,
 * transforms, effects, masks, mattes and motion blur all apply, in preview
 * and in export alike.
 *
 * Layer space has its origin at the composition centre (the layer's default
 * position), so the anchor of the returned bitmap is where that origin falls
 * inside it.
 */
import { cueIndexAt, normalizeCaptionStyle, type CaptionCue, type CaptionStyle } from './model';
import { layoutCaption, placeBlock, stateKey, wordStates, type CaptionLayout, type WordState } from './layout';

export interface CaptionRasterDeps {
  canvas(width: number, height: number): HTMLCanvasElement;
  /** CSS font shorthand for a family/weight/size, matching text layers. */
  font(style: { font: string; weight: number; italic: boolean; size: number }): string;
  /** True when the font can paint now; false paints a retryable blank. */
  fontReady(font: string, text: string): boolean;
}

export interface CaptionFrame {
  key: string;
  cue: CaptionCue;
  style: CaptionStyle;
  layout: CaptionLayout;
  states: WordState[];
  block: { x0: number; y0: number; x1: number; y1: number };
  /** Room around the block for strokes and scaled words. */
  margin: number;
}

export interface CaptionRaster {
  cv: HTMLCanvasElement;
  w: number;
  h: number;
  anchorX: number;
  anchorY: number;
  selection: { x0: number; y0: number; x1: number; y1: number; w: number; h: number };
  blank: boolean;
}

const styleKeyCache = new WeakMap<object, string>();
const styleKey = (style: CaptionStyle) => {
  let key = styleKeyCache.get(style);
  if (!key) { key = JSON.stringify(style); styleKeyCache.set(style, key); }
  return key;
};

function hexToRgba(hex: string, opacity = 1): string {
  const value = hex.replace('#', '');
  const r = parseInt(value.slice(0, 2), 16), g = parseInt(value.slice(2, 4), 16), b = parseInt(value.slice(4, 6), 16);
  const a = value.length >= 8 ? parseInt(value.slice(6, 8), 16) / 255 : 1;
  return `rgba(${r},${g},${b},${Math.round(a * opacity * 1000) / 1000})`;
}

function roundedRect(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const radius = Math.max(0, Math.min(r, w / 2, h / 2));
  c.beginPath();
  c.moveTo(x + radius, y);
  c.arcTo(x + w, y, x + w, y + h, radius);
  c.arcTo(x + w, y + h, x, y + h, radius);
  c.arcTo(x, y + h, x, y, radius);
  c.arcTo(x, y, x + w, y, radius);
  c.closePath();
}

export function createCaptionRasterizer(deps: CaptionRasterDeps) {
  const measureContext = () => {
    const context = deps.canvas(8, 8).getContext('2d')!;
    context.textBaseline = 'alphabetic';
    return context;
  };
  let measurer: CanvasRenderingContext2D | null = null;
  const layouts = new Map<string, CaptionLayout>();

  function layoutFor(cue: CaptionCue, style: CaptionStyle, compositionWidth: number): CaptionLayout {
    const key = `${cue.id}|${cue.text}|${cue.start}|${cue.end}|${cue.words?.length ?? 0}|${styleKey(style)}|${compositionWidth}`;
    const cached = layouts.get(key);
    if (cached) { layouts.delete(key); layouts.set(key, cached); return cached; }
    measurer ??= measureContext();
    const context = measurer;
    let font = '';
    const measure = (text: string, size: number) => {
      const next = deps.font({ ...style, size });
      if (next !== font) { context.font = font = next; if ('letterSpacing' in context) (context as any).letterSpacing = `${style.tracking * size / style.size}px`; }
      return context.measureText(text).width;
    };
    const layout = layoutCaption(cue, style, measure, { compositionWidth });
    layouts.set(key, layout);
    if (layouts.size > 256) layouts.delete(layouts.keys().next().value!);
    return layout;
  }

  /** What the layer shows at layer-local time `local`, or null between cues. */
  function frame(content: any, local: number, compositionWidth: number, compositionHeight: number): CaptionFrame | null {
    const cues: CaptionCue[] = Array.isArray(content?.cues) ? content.cues : [];
    const index = cueIndexAt(cues, local);
    if (index < 0) return null;
    const cue = cues[index]!;
    const style = content.style && typeof content.style === 'object' && 'maxLines' in content.style ? content.style as CaptionStyle : normalizeCaptionStyle(content.style);
    const layout = layoutFor(cue, style, compositionWidth);
    const animated = style.highlight || style.wordAnimation !== 'none';
    const states = animated ? wordStates(layout.words, style, local) : [];
    const block = placeBlock(layout, style, compositionWidth, compositionHeight);
    const growth = Math.max(1, style.highlightScale / 100, style.wordAnimation === 'pop' ? 1.1 : 1);
    const margin = Math.ceil(style.stroke + layout.size * (growth - 1) + 4);
    const key = `${cue.id}|${styleKey(style)}|${compositionWidth}x${compositionHeight}|${layoutKey(layout)}|${animated ? stateKey(states) : ''}`;
    return { key, cue, style, layout, states, block, margin };
  }

  function draw(f: CaptionFrame, scale: number): CaptionRaster {
    const { style, layout, block, margin } = f;
    const w = block.x1 - block.x0 + margin * 2, h = block.y1 - block.y0 + margin * 2;
    const density = Math.max(0.05, Math.min(scale, 8192 / w, 8192 / h));
    const cv = deps.canvas(Math.ceil(w * density), Math.ceil(h * density));
    const c = cv.getContext('2d')!;
    c.scale(density, density);
    c.translate(margin, margin);
    const pad = style.box ? style.boxPadding : 0;
    if (style.box && style.boxOpacity > 0) {
      c.fillStyle = hexToRgba(style.boxColor, style.boxOpacity / 100);
      roundedRect(c, 0, 0, block.x1 - block.x0, block.y1 - block.y0, style.boxRadius);
      c.fill();
    }
    const font = deps.font({ ...style, size: layout.size });
    c.font = font;
    if ('letterSpacing' in c) (c as any).letterSpacing = `${style.tracking * layout.size / style.size}px`;
    c.textBaseline = 'alphabetic';
    c.lineJoin = 'round';
    c.miterLimit = 2;
    const ascent = layout.size * 0.82 + (layout.lineHeight - layout.size) / 2;
    const stroke = style.stroke > 0 ? style.stroke * layout.size / style.size : 0;
    for (const line of layout.lines) {
      for (const word of line.words) {
        const state = f.states[word.index] ?? { alpha: 1, scale: 1, highlighted: false };
        if (state.alpha <= 0) continue;
        c.save();
        c.globalAlpha = state.alpha;
        const x = pad + word.x, y = pad + line.y + ascent;
        if (state.scale !== 1) {
          const cx = x + word.width / 2, cy = pad + line.y + layout.lineHeight / 2;
          c.translate(cx, cy); c.scale(state.scale, state.scale); c.translate(-cx, -cy);
        }
        if (stroke > 0) {
          c.strokeStyle = hexToRgba(style.strokeColor);
          c.lineWidth = stroke * 2;
          c.strokeText(word.text, x, y);
        }
        c.fillStyle = state.highlighted ? style.highlightColor : style.fill;
        c.fillText(word.text, x, y);
        c.restore();
      }
    }
    const text = layout.lines.map(line => line.words.map(word => word.text).join(' ')).join(' ');
    const blank = !deps.fontReady(font, text);
    const selection = { x0: block.x0, y0: block.y0, x1: block.x1, y1: block.y1, w: block.x1 - block.x0, h: block.y1 - block.y0 };
    return { cv, w, h, anchorX: margin - block.x0, anchorY: margin - block.y0, selection, blank };
  }

  /** Bounds for selection and snapping when no cue is showing: the column
      the next caption will occupy, one line tall. */
  function idleBounds(content: any, compositionWidth: number, compositionHeight: number) {
    const style = normalizeCaptionStyle(content?.style);
    const lineHeight = style.size * style.leading;
    const block = placeBlock({ width: compositionWidth * style.maxWidth / 100 - (style.box ? style.boxPadding * 2 : 0), height: lineHeight }, style, compositionWidth, compositionHeight);
    return { x0: block.x0, y0: block.y0, x1: block.x1, y1: block.y1, w: block.x1 - block.x0, h: block.y1 - block.y0, ax: 0, ay: 0 };
  }

  function clear() { layouts.clear(); }

  return { frame, draw, idleBounds, clear };
}

function layoutKey(layout: CaptionLayout): string {
  return `${layout.size.toFixed(2)}:${layout.lines.length}:${layout.width.toFixed(1)}`;
}
