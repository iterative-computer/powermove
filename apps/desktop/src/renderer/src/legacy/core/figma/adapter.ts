import type { NodeChange } from '@open-pencil/kiwi/fig/codec';
import { validateFigmaDoc, mul, type FigmaDoc, type FigmaNode, type Matrix } from '../../../../../shared/figma/doc';
import type { FigmaSelection } from './clipboard';
import { readGeometry } from './geometry';
const id = (guid: NodeChange['guid']) => (guid ? `${guid.sessionID}:${guid.localID}` : '');
const matrix = (n: NodeChange): Matrix =>
  n.transform
    ? [n.transform.m00, n.transform.m10, n.transform.m01, n.transform.m11, n.transform.m02, n.transform.m12]
    : [1, 0, 0, 1, 0, 0];
/** Clipboard-only private-format adapter. Shared conversion never imports kiwi. */
export function clipboardDoc(selection: FigmaSelection): FigmaDoc {
  const entries = new Map<string, { raw: NodeChange; children: string[] }>();
  for (const raw of selection.nodes) {
    if (
      !raw.guid ||
      raw.phase === 'REMOVED' ||
      ['DOCUMENT', 'CANVAS', 'STYLE', 'VARIABLE', 'VARIABLE_SET', 'SLICE'].includes(raw.type ?? '')
    )
      continue;
    const key = id(raw.guid);
    if (entries.has(key)) throw new Error('Duplicate Figma layer');
    entries.set(key, { raw, children: [] });
  }
  const roots: string[] = [];
  for (const [key, { raw }] of entries) {
    const parent = entries.get(id(raw.parentIndex?.guid));
    if (parent) parent.children.push(key);
    else roots.push(key);
  }
  const seen = new Set<string>();
  const sorted = (ids: string[]) =>
    ids.sort((a, b) =>
      (entries.get(a)!.raw.parentIndex?.position ?? '') < (entries.get(b)!.raw.parentIndex?.position ?? '') ? -1 : 1,
    );
  const build = (key: string, parent?: Matrix, depth = 0): FigmaNode => {
    if (seen.has(key) || depth > 90) throw new Error('Invalid Figma hierarchy');
    seen.add(key);
    const { raw: n, children } = entries.get(key)!;
    const abs = parent ? mul(parent, matrix(n)) : matrix(n),
      unsupported: string[] = [];
    const paths: FigmaNode['paths'] = [];
    for (const g of n.fillGeometry ?? []) {
      try {
        const blob = selection.blobs[g.commandsBlob!];
        if (!blob) throw 0;
        const contours = readGeometry(blob);
        for (const c of contours) {
          const vs = c.vertices;
          const first = vs[0];
          if (!first) continue;
          let d = `M ${first.x} ${first.y}`;
          for (let i = 1; i < vs.length; i++) {
            const a = vs[i - 1]!,
              b = vs[i]!;
            d += ` C ${a.x + a.outX} ${a.y + a.outY} ${b.x + b.inX} ${b.y + b.inY} ${b.x} ${b.y}`;
          }
          if (c.closed) {
            const a = vs.at(-1)!,
              b = first;
            d += ` C ${a.x + a.outX} ${a.y + a.outY} ${b.x + b.inX} ${b.y + b.inY} ${b.x} ${b.y} Z`;
          }
          paths.push({ data: d, windingRule: g.windingRule ?? 'NONZERO' });
        }
        if (g.styleID) unsupported.push('mixed vector fills');
      } catch {
        unsupported.push('complex vector geometry');
      }
    }
    const convertPaint = (p: any) => {
      const out: any = { type: p.type, visible: p.visible, opacity: p.opacity, color: p.color, blendMode: p.blendMode };
      if (p.stops) out.gradientStops = p.stops;
      if (p.transform)
        out.gradientTransform = [
          p.transform.m00,
          p.transform.m10,
          p.transform.m01,
          p.transform.m11,
          p.transform.m02,
          p.transform.m12,
        ];
      return out;
    };
    const size = n.fontSize ?? 16,
      baseline = n.derivedTextData?.baselines?.[0];
    if (n.textData?.characterStyleIDs?.some((v) => v !== 0)) unsupported.push('mixed clipboard text styles');
    const leading =
      n.lineHeight?.units === 'PIXELS'
        ? n.lineHeight.value
        : n.lineHeight?.units === 'PERCENT'
          ? (size * n.lineHeight.value) / 100
          : baseline?.lineHeight;
    const arc = n.arcData as any;
    return {
      id: key,
      fileKey: 'clipboard',
      type: n.type ?? 'UNKNOWN',
      name: n.name ?? 'Figma layer',
      absoluteTransform: abs,
      w: n.size?.x ?? 0,
      h: n.size?.y ?? 0,
      visible: n.visible,
      locked: n.locked,
      opacity: n.opacity,
      blendMode: n.blendMode,
      fills: (n.fillPaints ?? n.backgroundPaints ?? []).map(convertPaint),
      strokes: (n.strokePaints ?? []).map(convertPaint),
      effects: (n.effects ?? []).map((e: any) => ({
        type: e.type,
        visible: e.visible,
        radius: e.radius,
        spread: e.spread,
        offset: e.offset,
        color: e.color,
        blendMode: e.blendMode,
      })),
      strokeWeight: n.strokeWeight,
      strokeAlign: (['CENTER', 'INSIDE', 'OUTSIDE'].includes(n.strokeAlign ?? '')
        ? n.strokeAlign
        : undefined) as FigmaNode['strokeAlign'],
      dashes: n.dashPattern,
      cap: n.strokeCap,
      join: n.strokeJoin,
      radii: [
        n.rectangleTopLeftCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleTopRightCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleBottomRightCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleBottomLeftCornerRadius ?? n.cornerRadius ?? 0,
      ],
      smoothing: n.cornerSmoothing,
      clipsContent: ['FRAME', 'SYMBOL', 'INSTANCE'].includes(n.type ?? '') && n.frameMaskDisabled !== true,
      isMask: n.mask,
      paths,
      ...(arc
        ? { arc: { start: arc.startingAngle ?? 0, end: arc.endingAngle ?? Math.PI * 2, inner: arc.innerRadius ?? 0 } }
        : {}),
      ...(n.type === 'TEXT'
        ? {
            text: {
              characters: n.textData?.characters ?? '',
              runs: [
                {
                  start: 0,
                  end: n.textData?.characters?.length ?? 0,
                  font: n.fontName?.family ?? 'Arial',
                  style: n.fontName?.style ?? 'Regular',
                  weight: n.fontWeight,
                  size,
                  fills: (n.fillPaints ?? []).map(convertPaint),
                },
              ],
              align: n.textAlignHorizontal,
              autoResize: n.textAutoResize,
              leading,
              tracking:
                n.letterSpacing?.units === 'PERCENT' ? (size * n.letterSpacing.value) / 100 : n.letterSpacing?.value,
              baseline: baseline ? baseline.position.y - size * 0.82 : 0,
              advanced: !!(
                n.paragraphIndent ||
                n.paragraphSpacing ||
                n.fontVariations?.length ||
                (n.textDecoration && n.textDecoration !== 'NONE') ||
                (n.textCase && n.textCase !== 'ORIGINAL')
              ),
            },
          }
        : {}),
      unsupported,
      children: sorted(children).map((c) => build(c, abs, depth + 1)),
    };
  };
  const nodes = sorted(roots).map((key) => build(key));
  if (!nodes.length || seen.size !== entries.size) throw new Error('Invalid Figma hierarchy');
  return validateFigmaDoc({ format: 'powermove-figma', version: 1, source: 'clipboard', nodes, images: {} });
}
