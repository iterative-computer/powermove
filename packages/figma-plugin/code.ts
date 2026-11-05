import { figmaFileId } from './identity';
import { fallbackScopes } from '../../apps/desktop/src/shared/figma/convert';
import {
  MAX_PAYLOAD,
  validateFigmaDoc,
  type FigmaDoc,
  type FigmaNode,
  type Matrix,
  type Paint,
} from '../../apps/desktop/src/shared/figma/doc';
figma.showUI(__html__, { width: 300, height: 230, themeColors: true });
const selectionChanged = () => figma.ui.postMessage({ type: 'count', count: figma.currentPage.selection.length });
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
void figma.clientStorage.getAsync('powermove-token').then((token) => status({ type: 'token', token }));
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
      })),
      strokeWeight: typeof n.strokeWeight === 'number' ? n.strokeWeight : undefined,
      strokeAlign: n.strokeAlign,
      dashes: n.dashPattern,
      cap: typeof n.strokeCap === 'string' ? n.strokeCap : undefined,
      join: typeof n.strokeJoin === 'string' ? n.strokeJoin : undefined,
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
      paths: n.vectorPaths?.map((p: VectorPath) => ({ data: p.data, windingRule: p.windingRule })),
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
          })),
        ),
        align: n.textAlignHorizontal,
        autoResize: n.textAutoResize,
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
    if (typeof n.strokeWeight === 'symbol' || typeof n.strokeCap === 'symbol' || typeof n.strokeJoin === 'symbol')
      out.unsupported!.push('mixed strokes');
    // Instances expose resolved children, including their overrides, directly.
    if ('children' in n) for (const child of n.children) out.children.push(await serialize(child, depth + 1));
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
  for (const { node: n } of fallbackScopes({ nodes })) {
    const source = raw.get(n.id)!;
    const b = ('absoluteRenderBounds' in source ? source.absoluteRenderBounds : null) ?? source.absoluteBoundingBox;
    if (!b) throw new Error(`No render bounds for ${n.name}`);
    n.raster = {
      png: b64(await source.exportAsync({ format: 'PNG', constraint: { type: 'SCALE', value: 2 } })),
      bounds: { x: b.x, y: b.y, w: b.width, h: b.height },
    };
  }
  return validateFigmaDoc({ format: 'powermove-figma', version: 1, source: 'plugin', nodes, images });
}
type FigmaPaint = globalThis.Paint;
