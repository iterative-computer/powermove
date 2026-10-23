import type { NodeChange, Paint } from '@open-pencil/kiwi/fig/codec';
import type { PMRegistry } from '../../registry';
import type { FigmaSelection } from './clipboard';
import { readGeometry } from './geometry';
import { materializeSvgPaths, type ImportedSvgPath } from '../svg-import';

type Matrix = [number, number, number, number, number, number];
type Entry = { node: NodeChange; children: Entry[] };
export type ImportIssue = { name: string; reason: string };
export type FigmaPlan = { layers: any[]; roots: string[]; issues: ImportIssue[]; width: number; height: number };
const containers = new Set(['GROUP', 'FRAME', 'SYMBOL', 'INSTANCE']);
const nonvisual = new Set(['DOCUMENT', 'CANVAS', 'STYLE', 'VARIABLE', 'VARIABLE_SET']);
const identity: Matrix = [1, 0, 0, 1, 0, 0];
const key = (guid: NodeChange['guid']) => guid ? `${guid.sessionID}:${guid.localID}` : '';
const visiblePaints = (paints?: Paint[]) => (paints ?? []).filter(paint => paint.visible !== false && (paint.opacity ?? 1) > 0);
const point = (m: Matrix, x: number, y: number) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
const transform = (node: NodeChange): Matrix => node.transform
  ? [node.transform.m00, node.transform.m10, node.transform.m01, node.transform.m11, node.transform.m02, node.transform.m12] : [...identity];
const color = (paint?: Paint) => {
  const c = paint?.color ?? { r: 0, g: 0, b: 0, a: 0 };
  return '#' + [c.r, c.g, c.b, (c.a ?? 1) * (paint?.opacity ?? 1)]
    .map(value => Math.round(Math.max(0, Math.min(1, value)) * 255).toString(16).padStart(2, '0')).join('').toUpperCase();
};

/** Convert first, without touching the document, so fallback decisions are atomic. */
export function planFigmaImport(PM: PMRegistry, selection: FigmaSelection, at: number): FigmaPlan {
  const entries = new Map<string, Entry>();
  for (const node of selection.nodes) {
    if (!node.guid || node.phase === 'REMOVED' || nonvisual.has(node.type ?? '') || node.type === 'SLICE') continue;
    const id = key(node.guid);
    if (entries.has(id)) throw new Error('The copied Figma selection contains duplicate layers.');
    entries.set(id, { node, children: [] });
  }
  const roots: Entry[] = [];
  for (const entry of entries.values()) {
    const parent = entries.get(key(entry.node.parentIndex?.guid));
    if (parent) parent.children.push(entry); else roots.push(entry);
  }
  if (!roots.length) throw new Error('No importable layers were found in the Figma selection.');
  const issues: ImportIssue[] = [], layers: any[] = [], rootIds: string[] = [];
  let vertexCount = 0;
  const issue = (node: NodeChange, reason: string) => {
    if (issues.length < 30 && !issues.some(value => value.name === node.name && value.reason === reason)) issues.push({ name: node.name || node.type || 'Layer', reason });
  };
  const visited = new Set<Entry>();
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const { node } of roots) {
    const m = transform(node), w = node.size?.x ?? 0, h = node.size?.y ?? 0;
    for (const [x, y] of [[0, 0], [w, 0], [0, h], [w, h]]) {
      const p = point(m, x!, y!); x0 = Math.min(x0, p[0]!); y0 = Math.min(y0, p[1]!); x1 = Math.max(x1, p[0]!); y1 = Math.max(y1, p[1]!);
    }
  }
  if (![x0, y0, x1, y1].every(Number.isFinite)) throw new Error('The copied Figma selection has invalid bounds.');
  const width = Math.max(1, x1 - x0), height = Math.max(1, y1 - y0);
  const offsetX = (PM.proj.w - width) / 2 - x0, offsetY = (PM.proj.h - height) / 2 - y0;
  const from = PM.snapF(at, PM.proj.fps), dur = Math.max(1 / PM.proj.fps, PM.proj.dur - from);
  const build = (entry: Entry, group: string | null, depth: number) => {
    if (visited.has(entry) || depth > 100) throw new Error('The copied Figma layer hierarchy is invalid.');
    visited.add(entry);
    const node = entry.node;
    const m = transform(node);
    if (m.some(value => !Number.isFinite(value) || Math.abs(value) > 1e7)) throw new Error('A copied Figma layer has an invalid transform.');
    const w = node.size?.x ?? 0, h = node.size?.y ?? 0;
    if (![w, h].every(value => Number.isFinite(value) && value >= 0 && value <= 1e6)) throw new Error('A copied Figma layer has an invalid size.');
    for (const value of [node.opacity, node.strokeWeight, node.cornerRadius, node.cornerSmoothing,
      node.rectangleTopLeftCornerRadius, node.rectangleTopRightCornerRadius, node.rectangleBottomLeftCornerRadius, node.rectangleBottomRightCornerRadius]) {
      if (value != null && (!Number.isFinite(value) || value < 0)) throw new Error('A copied Figma layer has invalid properties.');
    }
    const sx = Math.hypot(m[0], m[1]), det = m[0] * m[3] - m[1] * m[2];
    const sy = sx ? det / sx : 0;
    const skew = sy ? Math.atan((m[0] * m[2] + m[1] * m[3]) / (sx * sy)) * 180 / Math.PI : 0;
    if (!sx || !sy || Math.abs(skew) > 80) issue(node, 'unsupported transform');
    const p = { 'position.x': m[4] + (group ? 0 : offsetX), 'position.y': m[5] + (group ? 0 : offsetY),
      'anchor.x': 0, 'anchor.y': 0, 'scale.x': sx * 100, 'scale.y': sy * 100,
      rotation: Math.atan2(m[1], m[0]) * 180 / Math.PI, skew, opacity: (node.opacity ?? 1) * 100 };
    const create = (type: string, d: any, props = p, name = node.name || node.type || 'Figma layer') => {
      // Primitive shapes render around their centre; Figma's origin is the
      // top-left corner. Editable vector paths already use authored coordinates.
      const adjusted = type === 'shape' && !d.paths?.length
        ? { ...props, 'anchor.x': -w / 2, 'anchor.y': -h / 2 } : props;
      const layer = PM.mkLayer(type, { name, from, dur, d, p: adjusted });
      layer.group = group; layer.on = node.visible !== false; layer.lock = !!node.locked;
      layer.collapsed = false;
      return layer;
    };
    if (node.mask) issue(node, 'mask');
    if (node.effects?.some(effect => effect.visible !== false)) issue(node, 'effects');
    if (node.blendMode && !['NORMAL', 'PASS_THROUGH'].includes(node.blendMode)) issue(node, 'blend mode');
    const fills = visiblePaints(node.fillPaints ?? node.backgroundPaints), strokes = visiblePaints(node.strokePaints);
    if (fills.length > 1 || strokes.length > 1) issue(node, 'multiple fills or strokes');
    for (const paint of [...fills, ...strokes]) {
      if ([paint.opacity, paint.color?.r, paint.color?.g, paint.color?.b, paint.color?.a].some(value => value != null && !Number.isFinite(value))) throw new Error('A Figma layer has an invalid fill.');
      if (paint.type !== 'SOLID') issue(node, paint.type === 'IMAGE' ? 'image fill' : 'gradient or special fill');
      if (paint.blendMode && paint.blendMode !== 'NORMAL') issue(node, 'fill blend mode');
    }
    if (node.dashPattern?.length) issue(node, 'dashed stroke');
    if (strokes.length && node.strokeCap && node.strokeCap !== 'NONE') issue(node, 'stroke end caps');
    if (strokes.length && node.strokeJoin && node.strokeJoin !== (node.type === 'VECTOR' ? 'MITER' : 'ROUND')) issue(node, 'stroke joins');
    if (node.type === 'INSTANCE') issue(node, 'component instance');
    const shapeContent = {
      shape: node.type === 'ELLIPSE' ? 'ellipse' : 'rect', w, h, color: color(fills[0]),
      radius: node.cornerRadius ?? 0, smoothing: (node.cornerSmoothing ?? 0) * 100,
      independentCorners: !!node.rectangleCornerRadiiIndependent,
      radiusTL: node.rectangleTopLeftCornerRadius ?? node.cornerRadius ?? 0,
      radiusTR: node.rectangleTopRightCornerRadius ?? node.cornerRadius ?? 0,
      radiusBR: node.rectangleBottomRightCornerRadius ?? node.cornerRadius ?? 0,
      radiusBL: node.rectangleBottomLeftCornerRadius ?? node.cornerRadius ?? 0,
      stroke: strokes.length ? node.strokeWeight ?? 1 : 0, strokeColor: color(strokes[0]),
    };
    if (strokes.length && node.strokeAlign !== 'CENTER') issue(node, 'inside or outside stroke');
    if (containers.has(node.type ?? '')) {
      if (strokes.length) issue(node, 'frame stroke');
      const layer = create('group', {}); layers.push(layer);
      if (!group) rootIds.push(layer.id);
      const children = entry.children.sort((a, b) => (a.node.parentIndex?.position ?? '') < (b.node.parentIndex?.position ?? '') ? -1 : 1);
      // Figma siblings paint bottom to top; Powermove's stack is top to bottom.
      for (const child of [...children].reverse()) build(child, layer.id, depth + 1);
      if (node.type !== 'GROUP' && (fills.length || strokes.length)) {
        const background = create('shape', shapeContent, { ...p, 'position.x': 0, 'position.y': 0, 'scale.x': 100, 'scale.y': 100, rotation: 0, skew: 0, opacity: 100 }, `${node.name || 'Frame'} background`);
        background.group = layer.id; background.on = true; layers.push(background);
      }
      if (node.type !== 'GROUP' && node.frameMaskDisabled !== true && children.length) {
        // No mask is needed when every descendant lies inside a square frame.
        // Rounded clipping and overflowing children require the image fallback.
        if (shapeContent.radius || shapeContent.independentCorners || children.some(child => {
          const cm = transform(child.node), cw = child.node.size?.x ?? 0, ch = child.node.size?.y ?? 0;
          return [[0, 0], [cw, 0], [0, ch], [cw, ch]].some(([x, y]) => {
            const v = point(cm, x!, y!); return v[0]! < 0 || v[1]! < 0 || v[0]! > w || v[1]! > h;
          }) || child.children.length > 0 || visiblePaints(child.node.strokePaints).length > 0;
        })) issue(node, 'clipped frame');
      }
    } else {
      let layer: any;
      if (node.type === 'TEXT') {
        const text = node.textData?.characters ?? '';
        const size = node.fontSize ?? 16, font = node.fontName?.family || 'Arial';
        if (!Number.isFinite(size) || size <= 0 || size > 16384) throw new Error('A Figma text layer has an invalid font size.');
        if (PM.Fonts?.families?.length && !PM.Fonts.families.includes(font)) issue(node, `missing font: ${font}`);
        if (node.textData?.characterStyleIDs?.some(id => id !== 0)) issue(node, 'mixed text styles');
        if ((node.textDecoration && node.textDecoration !== 'NONE') || (node.textCase && node.textCase !== 'ORIGINAL')
          || node.paragraphSpacing || node.paragraphIndent || node.fontVariations?.length || node.textData?.lines?.some(line => line.lineType && line.lineType !== 'PLAIN')) issue(node, 'advanced text formatting');
        if (!['LEFT', 'CENTER', 'RIGHT', undefined].includes(node.textAlignHorizontal)) issue(node, 'justified text');
        const baseline = node.derivedTextData?.baselines?.[0];
        const lineHeight = node.lineHeight?.units === 'PIXELS' ? node.lineHeight.value
          : node.lineHeight?.units === 'PERCENT' ? size * node.lineHeight.value / 100 : baseline?.lineHeight ?? size * 1.2;
        if (!Number.isFinite(lineHeight) || lineHeight <= 0) throw new Error('A Figma text layer has invalid line spacing.');
        if (node.letterSpacing && !Number.isFinite(node.letterSpacing.value)) throw new Error('A Figma text layer has invalid letter spacing.');
        const align = (node.textAlignHorizontal ?? 'LEFT').toLowerCase();
        const ox = align === 'center' ? w / 2 : align === 'right' ? w : 0;
        const oy = baseline ? baseline.position.y - size * .82 : 0;
        const origin = point(m, ox, oy);
        const textProps = { ...p, 'position.x': origin[0]! + (group ? 0 : offsetX), 'position.y': origin[1]! + (group ? 0 : offsetY) };
        const style = node.fontName?.style ?? '';
        const weights: Array<[RegExp, number]> = [[/thin/i, 100], [/extra.?light/i, 200], [/light/i, 300], [/medium/i, 500], [/semi.?bold|demi/i, 600], [/extra.?bold|ultra.?bold/i, 800], [/black|heavy/i, 900], [/bold/i, 700]];
        const weight = node.fontWeight ?? weights.find(([pattern]) => pattern.test(style))?.[1] ?? 400;
        layer = create('text', { text, font, size, weight, italic: /italic|oblique/i.test(style), color: color(fills[0]), align,
          boxWidth: node.textAutoResize === 'WIDTH_AND_HEIGHT' ? 0 : w,
          boxHeight: node.textAutoResize === 'NONE' ? h : 0, tracking: node.letterSpacing?.units === 'PERCENT' ? size * node.letterSpacing.value / 100 : node.letterSpacing?.value ?? 0,
          leading: lineHeight / size }, textProps);
        if (strokes.length) issue(node, 'text stroke');
      } else if (['RECTANGLE', 'ROUNDED_RECTANGLE', 'ELLIPSE'].includes(node.type ?? '')) {
        const arc = node.arcData as { startingAngle?: number; endingAngle?: number; innerRadius?: number } | undefined;
        if (arc && ((arc.innerRadius ?? 0) > 0 || Math.abs((arc.endingAngle ?? Math.PI * 2) - (arc.startingAngle ?? 0) - Math.PI * 2) > 1e-5)) issue(node, 'partial ellipse');
        layer = create('shape', shapeContent);
      } else if (['VECTOR', 'BOOLEAN_OPERATION', 'LINE', 'REGULAR_POLYGON', 'STAR'].includes(node.type ?? '')) {
        const paths: ImportedSvgPath[] = [];
        try {
          for (const geometry of node.fillGeometry ?? []) {
            const blob = geometry.commandsBlob == null ? undefined : selection.blobs[geometry.commandsBlob];
            if (!blob) throw new Error('Missing vector data');
            const contours = readGeometry(blob);
            vertexCount += contours.reduce((sum, contour) => sum + contour.vertices.length, 0);
            if (vertexCount > 50000) throw new Error('Too many editable vector points');
            // Powermove fills each editable path separately. Compound contours
            // can contain holes, so do not silently fill those holes in.
            if (fills.length && contours.length > 1) throw new Error('Compound vector');
            if (fills.length && contours.some(contour => !contour.closed)) throw new Error('Open vector fill');
            if (geometry.styleID) throw new Error('Mixed vector fills');
            for (const contour of contours) paths.push({ name: node.name || 'Path', ...contour,
              fill: color(fills[0]).slice(0, 7), fillEnabled: fills.length > 0, fillOpacity: (fills[0]?.opacity ?? 1) * (fills[0]?.color?.a ?? 1) * 100,
              stroke: color(strokes[0]).slice(0, 7), strokeWidth: strokes.length ? node.strokeWeight ?? 1 : 0,
              strokeOpacity: (strokes[0]?.opacity ?? 1) * (strokes[0]?.color?.a ?? 1) * 100 });
          }
          if (!paths.length) throw new Error('Missing editable vector paths');
        } catch { issue(node, 'complex vector geometry'); }
        layer = create('shape', { paths: materializeSvgPaths(PM, { width: w, height: h, paths, warnings: [] }) });
      } else { issue(node, `unsupported ${node.type?.toLowerCase() || 'layer'}`); layer = create('group', {}); }
      layers.push(layer); if (!group) rootIds.push(layer.id);
      if (entry.children.length) issue(node, 'unsupported layer hierarchy');
    }
  };
  roots.sort((a, b) => (a.node.parentIndex?.position ?? '') < (b.node.parentIndex?.position ?? '') ? 1 : -1);
  for (const entry of roots) build(entry, null, 0);
  if (visited.size !== entries.size) throw new Error('The copied Figma layer hierarchy is invalid.');
  return { layers, roots: rootIds, issues, width, height };
}

export function applyFigmaPlan(PM: PMRegistry, plan: FigmaPlan): void {
  if (plan.issues.length) throw new Error('This Figma selection needs a fallback.');
  const result = PM.Edit.mutate('Paste Figma layers', () => {
    PM.proj.layers.unshift(...plan.layers);
    PM.ProjectIndex?.invalidate();
    PM.selectLayers(plan.roots);
  }, { origin: 'import' });
  if (!result.ok) throw new Error(result.message);
}
