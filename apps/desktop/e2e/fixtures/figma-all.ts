import { parseSvgPathData } from '../../src/shared/figma/paths';
import type { FigmaDoc, FigmaNode } from '../../src/shared/figma/doc';
import { figmaHtml, figmaNode, guid } from '../../src/renderer/src/legacy/core/figma/__fixtures__/clipboard';
export { allFeatures, referenceSVG } from '../../src/shared/figma/__fixtures__/all-features';
export async function clipboardFixture(doc: FigmaDoc): Promise<string> {
  const blobs: Uint8Array[] = [],
    raw: any[] = [];
  const geometry = (data: string) => {
    const bytes: number[] = [];
    const point = (x: number, y: number) => {
      const buf = new Uint8Array(8);
      new DataView(buf.buffer).setFloat32(0, x, true);
      new DataView(buf.buffer).setFloat32(4, y, true);
      bytes.push(...buf);
    };
    for (const path of parseSvgPathData(data)) {
      const vs = path.vertices;
      if (!vs.length) continue;
      bytes.push(1);
      point(vs[0]!.x, vs[0]!.y);
      for (let i = 1; i < vs.length; i++) {
        const a = vs[i - 1]!,
          b = vs[i]!;
        bytes.push(4);
        point(a.x + a.outX, a.y + a.outY);
        point(b.x + b.inX, b.y + b.inY);
        point(b.x, b.y);
      }
      if (path.closed) {
        const a = vs.at(-1)!,
          b = vs[0]!;
        bytes.push(4);
        point(a.x + a.outX, a.y + a.outY);
        point(b.x + b.inX, b.y + b.inY);
        point(b.x, b.y);
        bytes.push(0);
      }
    }
    const index = blobs.length;
    blobs.push(Uint8Array.from(bytes));
    return index;
  };
  let serial = 0;
  const visit = (n: FigmaNode, parent?: { n: FigmaNode; id: number }, index = 0) => {
    const id = ++serial,
      m = n.absoluteTransform,
      p = parent?.n.absoluteTransform;
    const paint = (v: any) => {
      const m = v.gradientTransform ?? v.imageTransform;
      let imageIndex: number | undefined;
      if (v.type === 'IMAGE' && doc.images[v.imageHash]) {
        imageIndex = blobs.length;
        blobs.push(Uint8Array.from(Buffer.from(doc.images[v.imageHash]!, 'base64')));
      }
      return {
        type: v.type,
        color: v.color,
        visible: v.visible,
        opacity: v.opacity,
        blendMode: v.blendMode,
        stops: v.gradientStops,
        scale: v.scalingFactor,
        transform: m
          ? {
              m00: m[0],
              m10: m[1],
              m01: m[2],
              m11: m[3],
              m02: m[4],
              m12: m[5],
            }
          : undefined,
        ...(v.type === 'IMAGE'
          ? {
              image: { hash: Uint8Array.from([1, 2, 3]), dataBlob: imageIndex },
              imageScaleMode: v.scaleMode === 'CROP' ? 'STRETCH' : v.scaleMode,
            }
          : {}),
      };
    };
    const node: any = figmaNode(id, n.type === 'BOOLEAN_OPERATION' ? 'BOOLEAN_OPERATION' : n.type, {
      name: n.name,
      size: { x: n.w, y: n.h },
      transform: {
        m00: m[0],
        m10: m[1],
        m01: m[2],
        m11: m[3],
        m02: m[4] - (p?.[4] ?? 0),
        m12: m[5] - (p?.[5] ?? 0),
      },
      fillPaints: n.fills.map(paint),
      strokePaints: n.strokes.map(paint),
      strokeWeight: n.strokeWeight,
      miterLimit: n.miterLimit,
      ...(n.strokeWeights
        ? {
            borderStrokeWeightsIndependent: true,
            borderTopWeight: n.strokeWeights[0],
            borderRightWeight: n.strokeWeights[1],
            borderBottomWeight: n.strokeWeights[2],
            borderLeftWeight: n.strokeWeights[3],
          }
        : {}),
      strokeAlign: n.strokeAlign,
      dashPattern: n.dashes,
      strokeCap: n.cap === 'ARROW_TRIANGLE' ? 'ARROW_EQUILATERAL' : n.cap,
      strokeJoin: n.join,
      blendMode: n.blendMode,
      effects: n.effects.map((e) => ({
        ...e,
        noiseSize: (e as any).noiseSize ? { x: (e as any).noiseSize, y: (e as any).noiseSize } : undefined,
        type: e.type === 'LAYER_BLUR' ? 'FOREGROUND_BLUR' : e.type === 'TEXTURE' ? 'GRAIN' : e.type,
      })),
      cornerRadius: n.radii?.[0],
      cornerSmoothing: n.smoothing,
      frameMaskDisabled: !n.clipsContent,
      mask: n.isMask,
      maskType: n.maskType,
      arcData: n.arc
        ? { startingAngle: n.arc.start, endingAngle: n.arc.end, innerRadius: n.arc.inner }
        : undefined,
    } as any);
    if (parent) node.parentIndex = { guid: guid(parent.id), position: String.fromCharCode(33 + index) };
    if (n.paths?.length)
      node.fillGeometry = n.paths.map((path) => ({
        commandsBlob: geometry(path.data),
        windingRule: path.windingRule === 'EVENODD' ? 'ODD' : path.windingRule,
      }));
    if (n.text) {
      const t = n.text,
        r = t.runs[0]!;
      Object.assign(node, {
        textData: {
          characters: t.characters,
          characterStyleIDs: Array.from({ length: t.characters.length }, (_, i) =>
            Math.max(
              0,
              t.runs.findIndex((r) => i >= r.start && i < r.end),
            ),
          ),
          styleOverrideTable: t.runs
            .slice(1)
            .map((r, index) => ({
              styleID: index + 1,
              fontName: { family: r.font, style: r.style, postscript: r.font },
              fontSize: r.size,
              fontWeight: r.weight,
              fillPaints: r.fills.map(paint),
              textDecoration: r.decoration,
              textCase: r.textCase,
              letterSpacing: r.tracking === undefined ? undefined : { units: 'PIXELS', value: r.tracking },
              lineHeight: r.leading === undefined ? undefined : { units: 'PIXELS', value: r.leading },
              paragraphSpacing: r.paragraphSpacing,
              paragraphIndent: r.paragraphIndent,
            })),
        },
        fontName: { family: r.font, style: r.style, postscript: r.font },
        fontSize: r.size,
        textAlignHorizontal: t.align,
        textAlignVertical: (t as any).verticalAlign,
        textAutoResize: t.autoResize,
        textTruncation: t.truncation,
        maxLines: t.maxLines,
        letterSpacing: t.tracking === undefined ? undefined : { units: 'PIXELS', value: t.tracking },
        lineHeight: t.leading === undefined ? undefined : { units: 'PIXELS', value: t.leading },
        paragraphSpacing: (t as any).paragraphSpacing,
        paragraphIndent: (t as any).paragraphIndent,
      });
    }
    raw.push(node);
    n.children.forEach((c, i) => visit(c, { n, id }, i));
  };
  doc.nodes.forEach((n, i) => visit(n, undefined, i));
  return figmaHtml(raw, blobs);
}
