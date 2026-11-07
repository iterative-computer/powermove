import { networkStrokePaths, parseSvgPathData } from '../../apps/desktop/src/shared/figma/paths';
import { figmaFileId } from './identity';
import { fallbackScopes } from '../../apps/desktop/src/shared/figma/convert';
import {
  inverse,
  mul,
  MAX_PAYLOAD,
  validateFigmaDoc,
  type FigmaDoc,
  type FigmaNode,
  type Matrix,
  type Paint,
} from '../../apps/desktop/src/shared/figma/doc';
figma.showUI(__html__, { width: 300, height: 230, themeColors: true });
const selectionChanged = () =>
  figma.ui.postMessage({ type: 'count', count: figma.currentPage.selection.length });
figma.on('selectionchange', selectionChanged);
selectionChanged();
const matrix = (m: Transform): Matrix => [m[0][0], m[1][0], m[0][1], m[1][1], m[0][2], m[1][2]];
const b64 = (bytes: Uint8Array) => figma.base64Encode(bytes);
let busy = false;
const status = (value: unknown) => figma.ui.postMessage(value);
figma.ui.onmessage = async (message) => {
  if (message.type === 'token') {
    if (typeof message.token === 'string' && /^[a-f0-9]{64}$/.test(message.token))
      await figma.clientStorage.setAsync('powermove-token', message.token);
    else if (!message.token) await figma.clientStorage.deleteAsync('powermove-token');
    return;
  }
  if (message.type === 'ready') {
    selectionChanged();
    void figma.clientStorage.getAsync('powermove-token').then((token) => status({ type: 'token', token }));
    return;
  }
  if (message.type !== 'send' || busy) return;
  busy = true;
  try {
    const doc = await serializeSelection();
    const payload = JSON.stringify(doc);
    if (payload.length > MAX_PAYLOAD) throw new Error('Select fewer layers (32 MB limit).');
    status({ type: 'selection', doc });
  } catch (e) {
    const text = e instanceof Error ? e.message : 'Could not send selection';
    figma.notify(text, { error: true });
    status({ type: 'error', text });
  } finally {
    busy = false;
  }
};
export async function serializeSelection(): Promise<FigmaDoc> {
  if (!figma.currentPage.selection.length) throw new Error('Select layers in Figma first.');
  const fileId = figmaFileId(figma.root);
  const images: Record<string, string> = {},
    seen = new Set<string>();
  let count = 0;
  const paint = async (p: FigmaPaint): Promise<Paint> => {
    const base: Paint = { type: p.type, visible: p.visible, opacity: p.opacity, blendMode: p.blendMode };
    if (p.type === 'SOLID') base.color = { ...p.color, a: 1 };
    else if (p.type === 'IMAGE') {
      base.imageHash = p.imageHash ?? undefined;
      base.scaleMode = p.scaleMode;
      if (p.imageTransform) base.imageTransform = matrix(p.imageTransform);
      base.scalingFactor = p.scalingFactor;
      base.rotation = p.rotation;
      if (p.imageHash && !seen.has(p.imageHash)) {
        seen.add(p.imageHash);
        const image = figma.getImageByHash(p.imageHash);
        if (image) images[p.imageHash] = b64(await image.getBytesAsync());
      }
    } else if ('gradientStops' in p) {
      base.gradientTransform = matrix(p.gradientTransform);
      base.gradientStops = p.gradientStops.map((s) => ({ position: s.position, color: s.color }));
    }
    return base;
  };
  const paints = async (ps: readonly FigmaPaint[] | PluginAPI['mixed'] | undefined) =>
    Array.isArray(ps) ? Promise.all(ps.map(paint)) : [];
  const serialize = async (node: SceneNode, depth = 0): Promise<FigmaNode> => {
    if (++count > 5000 || depth > 80) throw new Error('Select fewer layers.');
    const n = node as SceneNode & Record<string, any>;
    const out: FigmaNode = {
      id: n.id,
      fileKey: fileId,
      type: n.type,
      name: n.name,
      absoluteTransform: matrix(n.absoluteTransform),
      w: n.width,
      h: n.height,
      visible: n.visible,
      locked: n.locked,
      opacity: n.opacity,
      blendMode: n.blendMode,
      fills: await paints(n.fills),
      strokes: await paints(n.strokes),
      effects: (n.effects ?? []).map((e: any) => ({
        type: e.type,
        visible: e.visible,
        radius: e.radius,
        spread: e.spread,
        offset: e.offset,
        color: e.color,
        blendMode: e.blendMode,
        blurType: e.blurType,
        startRadius: e.startRadius,
        startOffset: e.startOffset,
        endOffset: e.endOffset,
        noiseType: e.noiseType,
        noiseSize: e.noiseSize,
        density: e.density,
        opacity: e.opacity,
        secondaryColor: e.secondaryColor,
        clipToShape: e.clipToShape,
        lightIntensity: e.lightIntensity,
        lightAngle: e.lightAngle,
        refraction: e.refraction,
        depth: e.depth,
        dispersion: e.dispersion,
      })),
      strokeWeight: typeof n.strokeWeight === 'number' ? n.strokeWeight : undefined,
      strokeAlign: n.strokeAlign,
      dashes: n.dashPattern,
      cap: typeof n.strokeCap === 'string' ? n.strokeCap : undefined,
      join: typeof n.strokeJoin === 'string' ? n.strokeJoin : undefined,
      miterLimit: n.strokeMiterLimit,
      ...(typeof n.strokeTopWeight === 'number'
        ? {
            strokeWeights: [n.strokeTopWeight, n.strokeRightWeight, n.strokeBottomWeight, n.strokeLeftWeight],
          }
        : {}),
      pointCount: n.pointCount,
      booleanOperation: n.booleanOperation,
      radii: [
        n.topLeftRadius ?? (typeof n.cornerRadius === 'number' ? n.cornerRadius : 0),
        n.topRightRadius ?? (typeof n.cornerRadius === 'number' ? n.cornerRadius : 0),
        n.bottomRightRadius ?? (typeof n.cornerRadius === 'number' ? n.cornerRadius : 0),
        n.bottomLeftRadius ?? (typeof n.cornerRadius === 'number' ? n.cornerRadius : 0),
      ],
      smoothing: n.cornerSmoothing,
      clipsContent: n.clipsContent,
      isMask: n.isMask,
      maskType: n.maskType,
      paths: (
        n.vectorPaths ??
        (['VECTOR', 'BOOLEAN_OPERATION', 'STAR', 'REGULAR_POLYGON'].includes(n.type)
          ? n.fillGeometry
          : undefined)
      )?.map((p: VectorPath) => ({ data: p.data, windingRule: p.windingRule })),
      children: [],
      unsupported: [],
    };
    if (n.arcData)
      out.arc = { start: n.arcData.startingAngle, end: n.arcData.endingAngle, inner: n.arcData.innerRadius };
    if (n.type === 'TEXT') {
      const segments = n.getStyledTextSegments([
        'fontName',
        'fontSize',
        'fontWeight',
        'fills',
        'textDecoration',
        'textCase',
        'letterSpacing',
        'lineHeight',
        'paragraphSpacing',
        'paragraphIndent',
        'listOptions',
        'listSpacing',
        'indentation',
      ]);
      const first = segments[0],
        size = first?.fontSize ?? 16;
      out.text = {
        characters: n.characters,
        runs: await Promise.all(
          segments.map(async (s: any) => ({
            start: s.start,
            end: s.end,
            font: s.fontName.family,
            style: s.fontName.style,
            weight: s.fontWeight,
            size: s.fontSize,
            fills: await paints(s.fills),
            decoration: s.textDecoration,
            textCase: s.textCase,
            tracking:
              s.letterSpacing?.unit === 'PERCENT'
                ? (s.fontSize * s.letterSpacing.value) / 100
                : s.letterSpacing?.value,
            leading:
              s.lineHeight?.unit === 'PIXELS'
                ? s.lineHeight.value
                : s.lineHeight?.unit === 'PERCENT'
                  ? (s.fontSize * s.lineHeight.value) / 100
                  : s.fontSize * 1.2,
            paragraphSpacing: s.paragraphSpacing,
            paragraphIndent: s.paragraphIndent,
            listSpacing: s.listSpacing,
            indentation: s.indentation,
            listType: s.listOptions?.type,
          })),
        ),
        align: n.textAlignHorizontal,
        autoResize: n.textAutoResize,
        verticalAlign: n.textAlignVertical,
        paragraphSpacing: typeof n.paragraphSpacing === 'number' ? n.paragraphSpacing : undefined,
        paragraphIndent: typeof n.paragraphIndent === 'number' ? n.paragraphIndent : undefined,
        truncation: n.textTruncation,
        maxLines: n.maxLines ?? undefined,
        leading:
          first?.lineHeight.unit === 'PIXELS'
            ? first.lineHeight.value
            : first?.lineHeight.unit === 'PERCENT'
              ? (size * first.lineHeight.value) / 100
              : size * 1.2,
        tracking:
          first?.letterSpacing.unit === 'PERCENT'
            ? (size * first.letterSpacing.value) / 100
            : (first?.letterSpacing.value ?? 0),
        advanced: !!(
          n.paragraphIndent ||
          n.paragraphSpacing ||
          n.textAlignVertical !== 'TOP' ||
          segments.some(
            (s: any) =>
              JSON.stringify(s.lineHeight) !== JSON.stringify(first.lineHeight) ||
              JSON.stringify(s.letterSpacing) !== JSON.stringify(first.letterSpacing),
          )
        ),
      };
    }
    if (n.vectorNetwork && !out.fills.length) out.paths = networkStrokePaths(n.vectorNetwork);
    const needsOutline =
      (typeof n.strokeWeight === 'symbol' && !out.strokeWeights) ||
      typeof n.strokeJoin === 'symbol' ||
      (n.variableWidthStrokeProperties && n.variableWidthStrokeProperties.widthProfile !== 'UNIFORM') ||
      (n.complexStrokeProperties?.type && n.complexStrokeProperties.type !== 'BASIC');
    if (needsOutline && out.strokes.length) {
      let outlined: any;
      try {
        outlined = n.outlineStroke?.();
        if (outlined) {
          const local = mul(inverse(out.absoluteTransform), matrix(outlined.absoluteTransform));
          const point = (x: number, y: number) => [
            local[0] * x + local[2] * y + local[4],
            local[1] * x + local[3] * y + local[5],
          ];
          out.strokePaths = (outlined.fillGeometry ?? outlined.vectorPaths ?? []).map((path: VectorPath) => ({
            windingRule: path.windingRule,
            data: parseSvgPathData(path.data)
              .map((p) => {
                const vs = p.vertices;
                if (!vs.length) return '';
                let d = 'M ' + point(vs[0]!.x, vs[0]!.y).join(' ');
                for (let i = 0; i < vs.length - (p.closed ? 0 : 1); i++) {
                  const a = vs[i]!,
                    b = vs[(i + 1) % vs.length]!;
                  d +=
                    ' C ' +
                    [
                      ...point(a.x + a.outX, a.y + a.outY),
                      ...point(b.x + b.inX, b.y + b.inY),
                      ...point(b.x, b.y),
                    ].join(' ');
                }
                return d + (p.closed ? ' Z' : '');
              })
              .join(' '),
          }));
        }
      } catch {
        // Failure to compute an outline is a per-node fallback, not a failed send.
      } finally {
        outlined?.remove();
      }
      if (!out.strokePaths?.length) out.unsupported!.push('missing computed stroke geometry');
    }
    // Boolean vectorPaths already contain the computed contour. Instances and
    // auto-layout expose resolved children at their final absolute positions.
    if ('children' in n && n.type !== 'BOOLEAN_OPERATION')
      for (const child of n.children) out.children.push(await serialize(child, depth + 1));
    return out;
  };
  // Remove descendants when their ancestor is also selected. Page order is paint order.
  const chosen = new Set(figma.currentPage.selection.map((n) => n.id));
  const roots = figma.currentPage.selection.filter((n) => {
    let p = n.parent;
    while (p) {
      if (chosen.has(p.id)) return false;
      p = p.parent;
    }
    return true;
  });
  const order = (n: BaseNode): number[] => {
    const path: number[] = [];
    let p = n.parent;
    while (p) {
      if ('children' in p) path.unshift(p.children.findIndex((c) => c.id === n.id));
      n = p;
      p = p.parent;
    }
    return path;
  };
  roots.sort((a, b) => {
    const aa = order(a),
      bb = order(b);
    for (let i = 0; i < Math.min(aa.length, bb.length); i++) if (aa[i] !== bb[i]) return aa[i] - bb[i];
    return aa.length - bb.length;
  });
  const nodes = await Promise.all(roots.map((n) => serialize(n)));
  // Export only the unsupported scopes, never their supported siblings or children.
  const raw = new Map<string, SceneNode>();
  const index = (n: SceneNode) => {
    raw.set(n.id, n);
    if ('children' in n) n.children.forEach(index);
  };
  roots.forEach(index);
  for (const { node: n } of fallbackScopes({ nodes, images })) {
    const source = raw.get(n.id)!;
    const b =
      ('absoluteRenderBounds' in source ? source.absoluteRenderBounds : null) ?? source.absoluteBoundingBox;
    if (!b) throw new Error(`No render bounds for ${n.name}`);
    n.raster = {
      png: b64(await source.exportAsync({ format: 'PNG', constraint: { type: 'SCALE', value: 2 } })),
      bounds: { x: b.x, y: b.y, w: b.width, h: b.height },
    };
  }
  return validateFigmaDoc({ format: 'powermove-figma', version: 1, source: 'plugin', nodes, images });
}
type FigmaPaint = globalThis.Paint;
