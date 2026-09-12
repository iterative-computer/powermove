import type { PMRegistry } from '../registry';
import { resolveContent } from './content-properties';

export type TextSplitMode = 'words' | 'characters' | 'lines';

/** Split the current text layout into editable runs; retain the source for recovery. */
export function splitTextLayers(PM: PMRegistry, ids: string[], mode: TextSplitMode): unknown {
  if (!['words', 'characters', 'lines'].includes(mode)) return false;
  const sources = ids.map(id => PM.L(id));
  if (!sources.length || sources.some((layer: any) => !layer || layer.type !== 'text' || layer.lock ||
    (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock))) return false;
  const plans = sources.map((source: any) => {
    const layout = PM.textLayout(source, PM.time);
    return { source, layout, pieces: layout[mode] };
  });
  if (!plans.some(({ pieces }: any) => pieces.length)) return false;
  return PM.Edit.mutate(`Split text by ${mode}`, () => {
    const created: any[] = [];
    for (const { source, pieces, layout } of plans) {
      if (!pieces.length) continue;
      const content = resolveContent(PM, source, PM.time);
      const fontOffset = PM.raster(source, 1, PM.time)?.fontOffset || { x: 0, y: 0 };
      const children = pieces.map((piece: any, index: number) => {
        const child = PM.cloneLayer(source);
        child.name = `${source.name} · ${index + 1} · ${piece.text}`;
        child.d = { ...child.d, ...content, text: piece.text, align: 'left', paragraph: false,
          fontAnchorBounds: null };
        // A split fixes the authored text and typography at the playhead. Its
        // transform channels, timing, effects and stack membership stay editable.
        const sourceStart = piece.sourceStart ?? String(content.text).indexOf(piece.text);
        child.d.styles = (child.d.styles || []).flatMap((style: any) => {
          const start = Math.max(style.start, sourceStart), end = Math.min(style.end, sourceStart + piece.text.length);
          return end > start ? [{ ...style, id: PM.uid('ts'), start: start - sourceStart, end: end - sourceStart }] : [];
        });
        // Remap range selectors so a word's first character keeps its original
        // selector weight instead of restarting the animation at character zero.
        const glyphs = layout.characters.filter((glyph: any) => glyph.sourceStart >= sourceStart && glyph.sourceStart < sourceStart + piece.text.length);
        child.d.animators = (source.d.animators || []).map((animator: any) => {
          const values: any = Object.fromEntries(Object.entries(animator.p).map(([key, prop]) =>
            [key, PM.evP(source, prop, PM.time, `ta.${animator.id}.${key}`)]));
          const unit = values.unit || 'characters';
          const first = glyphs[0];
          const base = unit === 'lines' ? piece.line : unit === 'words' ? first?.word ?? 0 : first?.index ?? 0;
          const count = unit === 'lines' ? 1 : unit === 'words' ? new Set(glyphs.map((g: any) => g.word)).size : glyphs.length;
          const total = layout[unit]?.length || 1, scale = total / Math.max(1, count);
          values.start = (Number(values.start) + Number(values.offset || 0) - base / total * 100) * scale;
          values.end = (Number(values.end) + Number(values.offset || 0) - base / total * 100) * scale;
          values.offset = 0;
          values.x = Number(values.x || 0) + Number(values.tracking || 0) * (first?.index || 0);
          return { ...animator, id: PM.uid('ta'), p: Object.fromEntries(Object.entries(values).map(([key, value]) => [key, PM.P(value)])) };
        });
        for (const [axis, offset] of [['x', piece.x + fontOffset.x], ['y', piece.y + fontOffset.y]] as const) {
          const prop = child.p[`anchor.${axis}`];
          // Moving the local origin preserves rotation, scale, skew and parenting.
          prop.v -= offset;
          for (const key of prop.kf) key.v -= offset;
          if (prop.expr) prop.expr = `(${prop.expr}) - (${offset})`;
        }
        return child;
      });
      PM.proj.layers.splice(PM.proj.layers.indexOf(source), 0, ...children);
      source.on = PM.P(false);
      source.solo = false;
      created.push(...children);
    }
    PM.ProjectIndex?.invalidate();
    PM.selectLayers(created.map(layer => layer.id));
    return created;
  }, { origin: 'command' });
}
