import { gradientColor, gradientPosition } from './typing.js';

export interface TextGeometry {
  text: string; font: string; weight: number; size: number; tracking: number;
  leading: number; align: string; italic: boolean; boxWidth: number;
  [key: string]: unknown;
}
export interface CursorSettings {
  [key: string]: number | string | boolean;
}
export interface Caret { x: number; baseline: number; size: number; rtl: boolean }

// Mirror the built-in canvas text editor's font, baseline, alignment and wrap
// settings. This hidden measurement element is never focused or displayed.
export function createCaretMeasure() {
  const root = document.createElement('div');
  root.setAttribute('aria-hidden', 'true');
  Object.assign(root.style, {
    position: 'fixed', left: '-100000px', top: '0', visibility: 'hidden',
    pointerEvents: 'none', padding: '0', margin: '0', border: '0',
    width: 'max-content', minWidth: '0', boxSizing: 'content-box',
    whiteSpace: 'pre', fontKerning: 'normal', fontVariantLigatures: 'normal'
  });
  document.body.append(root);
  const marker = () => {
    const el = document.createElement('span');
    Object.assign(el.style, { display: 'inline-block', width: '0', height: '0',
      padding: '0', margin: '0', border: '0', verticalAlign: 'baseline' });
    return el;
  };
  const memo = new Map<string, Caret>();
  return {
    dispose() { root.remove(); memo.clear(); },
    clear() { memo.clear(); },
    measure(d: TextGeometry, text: string): Caret {
      const signature = JSON.stringify([d, text]);
      const cached = memo.get(signature);
      if (cached) return cached;
      const size = Math.max(1, Number(d.size) || 64);
      Object.assign(root.style, {
        fontFamily: `"${String(d.font).replaceAll('"', '')}"`,
        fontWeight: String(d.weight || 400), fontStyle: d.italic ? 'italic' : 'normal',
        fontSize: `${size}px`, lineHeight: String(d.leading || 1.1),
        letterSpacing: `${Number(d.tracking) || 0}px`,
        fontVariationSettings: Object.keys(d).filter(k => k.startsWith('fontAxis.') && Number.isFinite(Number(d[k])))
          .map(k => `"${k.slice(9)}" ${Number(d[k])}`).join(',') || 'normal',
        textAlign: String(d.align || 'left'), whiteSpace: d.boxWidth ? 'pre-wrap' : 'pre',
        width: d.boxWidth ? `${Number(d.boxWidth)}px` : 'max-content'
      });
      root.dir = 'auto';
      const baselineMarker = marker();
      root.replaceChildren(document.createTextNode('Mg'), baselineMarker);
      const baseBaseline = baselineMarker.getBoundingClientRect().bottom - root.getBoundingClientRect().top;
      const caret = marker();
      root.replaceChildren(document.createTextNode(text), caret);
      const rect = root.getBoundingClientRect(), end = caret.getBoundingClientRect();
      const align = d.align === 'center' ? rect.width / 2 : d.align === 'right' ? rect.width : 0;
      const result = { x: end.left - rect.left - align,
        baseline: end.bottom - rect.top - baseBaseline + size * 0.82,
        size, rtl: getComputedStyle(root).direction === 'rtl' };
      memo.set(signature, result);
      if (memo.size > 128) memo.delete(memo.keys().next().value!);
      return result;
    }
  };
}

export function cursorBox(caret: Caret, settings: CursorSettings, offset: { x?: number; y?: number }) {
  const width = Math.max(0.5, Number(settings.cursorWidth) * caret.size / 100);
  const height = Math.max(0.5, Number(settings.cursorHeight) * caret.size * 0.82 / 100);
  const gap = Math.max(0, Number(settings.cursorOffsetX)) * caret.size / 100;
  const x = caret.x + (offset.x || 0) + (caret.rtl ? -gap - width : gap);
  const y = caret.baseline + (offset.y || 0) - height + Number(settings.cursorOffsetY) * caret.size / 100;
  return { x, y, width, height };
}

function rgb(value: unknown): number[] {
  const s = String(value);
  if (/^#[0-9a-f]{6}$/i.test(s)) return [1, 3, 5].map(i => parseInt(s.slice(i, i + 2), 16));
  if (/^#[0-9a-f]{3}$/i.test(s)) return [1, 2, 3].map(i => parseInt(s[i] + s[i], 16));
  return [255, 255, 255];
}

export function paintCursor(ctx: CanvasRenderingContext2D, box: ReturnType<typeof cursorBox>, settings: CursorSettings, time: number) {
  const { x, y, width, height } = box;
  const radians = Number(settings.gradAngle) * Math.PI / 180;
  const dx = Math.cos(radians), dy = -Math.sin(radians);
  // Use an ordinary gradient with densely sampled stops for offset/cycle. It
  // stays smooth at any export resolution without a separate pixel texture.
  const reach = Math.abs(dx) * width + Math.abs(dy) * height;
  const gradient = ctx.createLinearGradient(x + width / 2 - dx * reach / 2, y + height / 2 - dy * reach / 2,
    x + width / 2 + dx * reach / 2, y + height / 2 + dy * reach / 2);
  const colors = [rgb(settings.gradStart), rgb(settings.gradMid), rgb(settings.gradEnd)];
  for (let i = 0; i <= 128; i++) {
    const t = gradientPosition(i / 128, Number(settings.gradOffset), Number(settings.gradCycle), time);
    const color = gradientColor(t, colors, Number(settings.gradMidPos));
    gradient.addColorStop(i / 128, `rgb(${color.join(',')})`);
  }
  ctx.save();
  ctx.fillStyle = gradient;
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, Math.min(width, height) / 2 * Math.min(1, Math.max(0, Number(settings.cursorRound) / 100)));
  ctx.fill();
  ctx.restore();
}
