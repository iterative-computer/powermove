import type { PMRegistry } from '../registry';
import { resolveContent } from './content-properties';

export type TextSplitMode = 'words' | 'characters' | 'lines';

/** Split the current text layout into editable runs; retain the source for recovery. */
export function splitTextLayers(PM: PMRegistry, ids: string[], mode: TextSplitMode): unknown {
  if (!['words', 'characters', 'lines'].includes(mode)) return false;
  const sources = ids.map(id => PM.L(id));
  if (!sources.length || sources.some((layer: any) => !layer || layer.type !== 'text' || layer.lock ||
    (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock))) return false;
  // pm-edit lays the pieces out in pm-wasm and builds the split layers; the
  // structure (insertion, hiding the source, grouping) stays here.
  const native = PM.EngineDoc?.edits?.splitText(sources, mode, PM.time, () => legacySplit(PM, sources, mode),
    (plans: any[][]) => applySplit(PM, sources, mode, plans));
  return native ?? legacySplit(PM, sources, mode);
}

/** Inserts the kernel's split layers before each source, hides the source, and groups them. */
function applySplit(PM: PMRegistry, sources: any[], mode: TextSplitMode, plans: any[][]): unknown {
  if (!plans.some(children => children.length)) return false;
  return PM.Edit.mutate(`Split text by ${mode}`, () => {
    const created: any[] = [], groups: any[] = [];
    sources.forEach((source: any, i: number) => {
      const children = plans[i] || [];
      if (!children.length) return;
      PM.proj.layers.splice(PM.proj.layers.indexOf(source), 0, ...children);
      source.on = PM.P(false);
      source.solo = false;
      created.push(...children);
      groups.push(PM.groupLayers([...children.map((layer: any) => layer.id), source.id], source.name));
    });
    PM.ProjectIndex?.invalidate();
    PM.selectLayers(groups.map(group => group.id));
    return created;
  }, { origin: 'command' });
}

function legacySplit(PM: PMRegistry, sources: any[], mode: TextSplitMode): unknown {
  const plans = sources.map((source: any) => {
    const layout = PM.textLayout(source, PM.time);
    return { source, layout, pieces: layout[mode] };
  });
  if (!plans.some(({ pieces }: any) => pieces.length)) return false;
  return PM.Edit.mutate(`Split text by ${mode}`, () => {
    const created: any[] = [], groups: any[] = [];
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
        // Each piece keeps its place in the source's sequence, so a split
        // word's first character keeps its original selector weight and
        // stagger timing instead of restarting the animation at character zero.
        const glyphs = layout.characters.filter((glyph: any) => glyph.sourceStart >= sourceStart && glyph.sourceStart < sourceStart + piece.text.length);
        child.d.animators = (source.d.animators || []).map((animator: any) => {
          const legacy = animator.p?.unit ? PM.evP(source, animator.p.unit, PM.time, `ta.${animator.id}.unit`) : null;
          const unit = [animator.unit, legacy].find((value: any) => ['characters', 'words', 'lines'].includes(value)) || 'characters';
          const first = glyphs[0];
          const local = unit === 'lines' ? first?.lineUnit ?? piece.line : unit === 'words' ? first?.word ?? 0 : first?.index ?? 0;
          const clone = JSON.parse(JSON.stringify(animator));
          delete clone.p.unit;
          return { ...clone, id: PM.uid('ta'), unit,
            unitOffset: Number(animator.unitOffset || 0) + local,
            unitTotal: Math.max(Number(animator.unitTotal || 0), layout[unit]?.length || 1) };
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
      // Keep every derivative beside its hidden, recoverable source. Grouping
      // directly inside this mutation preserves one-step undo/redo while the
      // identity group transform leaves each split layer's world pose intact.
      groups.push(PM.groupLayers([...children.map((layer: any) => layer.id), source.id], source.name));
    }
    PM.ProjectIndex?.invalidate();
    PM.selectLayers(groups.map(group => group.id));
    return created;
  }, { origin: 'command' });
}
