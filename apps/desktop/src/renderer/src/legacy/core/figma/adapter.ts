import type { NodeChange } from "@open-pencil/kiwi/fig/codec";
import {
  validateFigmaDoc,
  mul,
  type FigmaDoc,
  type FigmaNode,
  type Matrix,
} from "../../../../../shared/figma/doc";
import type { FigmaSelection } from "./clipboard";
import { readGeometry } from "./geometry";
const id = (guid: NodeChange["guid"]) =>
  guid ? `${guid.sessionID}:${guid.localID}` : "";
const matrix = (n: NodeChange): Matrix =>
  n.transform
    ? [
        n.transform.m00,
        n.transform.m10,
        n.transform.m01,
        n.transform.m11,
        n.transform.m02,
        n.transform.m12,
      ]
    : [1, 0, 0, 1, 0, 0];
/** Clipboard-only private-format adapter. Shared conversion never imports kiwi. */
export function clipboardDoc(selection: FigmaSelection): FigmaDoc {
  const images: Record<string, string> = {};
  const entries = new Map<string, { raw: NodeChange; children: string[] }>();
  for (const raw of selection.nodes) {
    if (
      !raw.guid ||
      raw.phase === "REMOVED" ||
      [
        "DOCUMENT",
        "CANVAS",
        "STYLE",
        "VARIABLE",
        "VARIABLE_SET",
        "SLICE",
      ].includes(raw.type ?? "")
    )
      continue;
    const key = id(raw.guid);
    if (entries.has(key)) throw new Error("Duplicate Figma layer");
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
      (entries.get(a)!.raw.parentIndex?.position ?? "") <
      (entries.get(b)!.raw.parentIndex?.position ?? "")
        ? -1
        : 1,
    );
  const build = (key: string, parent?: Matrix, depth = 0): FigmaNode => {
    if (seen.has(key) || depth > 90) throw new Error("Invalid Figma hierarchy");
    seen.add(key);
    const { raw: n, children } = entries.get(key)!;
    const abs = parent ? mul(parent, matrix(n)) : matrix(n),
      unsupported: string[] = [];
    const geometry = (records: any[], styled = true) => {
      const paths: NonNullable<FigmaNode["paths"]> = [];
      for (const g of records) {
        try {
          const blob = selection.blobs[g.commandsBlob!];
          if (!blob) throw 0;
          const contours = readGeometry(blob),
            data: string[] = [];
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
            data.push(d);
          }
          if (data.length)
            paths.push({
              data: data.join(" "),
              windingRule:
                g.windingRule === "ODD"
                  ? "EVENODD"
                  : (g.windingRule ?? "NONZERO"),
            });
          if (g.styleID && styled) unsupported.push("mixed vector fills");
        } catch {
          unsupported.push("complex vector geometry");
        }
      }
      return paths;
    };
    const paths = geometry(n.fillGeometry ?? []);
    const strokePaths =
      n.strokePaints?.length &&
      n.strokeWeight == null &&
      n.strokeGeometry?.length
        ? geometry(n.strokeGeometry, false)
        : undefined;
    const convertPaint = (p: any) => {
      const out: any = {
        type: p.type,
        visible: p.visible,
        opacity: p.opacity,
        color: p.color,
        blendMode: p.blendMode,
      };
      if (p.type === "IMAGE") {
        const hash =
          typeof p.image?.hash === "string"
            ? p.image.hash
            : Array.from(p.image?.hash ?? [])
                .map((v) => Number(v).toString(16).padStart(2, "0"))
                .join("");
        const bytes =
          selection.blobs[p.image?.dataBlob] ?? selection.images?.[hash];
        out.imageHash = hash || undefined;
        if (hash && bytes?.length) {
          let binary = "";
          for (let i = 0; i < bytes.length; i += 32768)
            binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
          images[hash] = btoa(binary);
        }
        out.scaleMode =
          p.imageScaleMode === "STRETCH" ? "CROP" : p.imageScaleMode;
        out.scalingFactor = p.scale;
        if (p.transform)
          out.imageTransform = [
            p.transform.m00,
            p.transform.m10,
            p.transform.m01,
            p.transform.m11,
            p.transform.m02,
            p.transform.m12,
          ];
      }
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
    const characters = n.textData?.characters ?? "",
      styleIDs = n.textData?.characterStyleIDs ?? [],
      table = n.textData?.styleOverrideTable ?? [];
    const runs: any[] = [];
    let start = 0;
    for (let end = 1; end <= characters.length; end++)
      if (
        end === characters.length ||
        (styleIDs[end] ?? 0) !== (styleIDs[start] ?? 0) ||
        characters[end - 1] === "\n"
      ) {
        const styleID = styleIDs[start] ?? 0,
          override = styleID
            ? (table.find((v: any) => v.styleID === styleID) ??
              table[styleID] ??
              table[styleID - 1])
            : undefined;
        if (styleID && !override)
          unsupported.push("missing clipboard text style");
        const r = { ...n, ...override },
          fontSize = r.fontSize ?? 16;
        const paragraphIndex =
            characters.slice(0, start).split("\n").length - 1,
          line = n.textData?.lines?.[paragraphIndex] as any;
        const spacing = (v: any) =>
          v?.units === "PERCENT"
            ? (fontSize * v.value) / 100
            : v?.units === "PIXELS"
              ? v.value
              : undefined;
        runs.push({
          start,
          end,
          font: r.fontName?.family ?? "Arial",
          style: r.fontName?.style ?? "Regular",
          weight: r.fontWeight,
          size: fontSize,
          fills: (r.fillPaints ?? []).map(convertPaint),
          decoration: r.textDecoration,
          textCase: r.textCase,
          tracking: spacing(r.letterSpacing),
          leading: spacing(r.lineHeight),
          paragraphSpacing: r.paragraphSpacing,
          paragraphIndent: r.paragraphIndent,
          listType:
            line?.lineType === "ORDERED_LIST"
              ? "ORDERED"
              : line?.lineType === "UNORDERED_LIST"
                ? "UNORDERED"
                : "NONE",
          indentation: line?.indentationLevel,
        });
        start = end;
      }
    if (!runs.length)
      runs.push({
        start: 0,
        end: 0,
        font: n.fontName?.family ?? "Arial",
        style: n.fontName?.style ?? "Regular",
        size,
        fills: (n.fillPaints ?? []).map(convertPaint),
      });
    const leading =
      n.lineHeight?.units === "PIXELS"
        ? n.lineHeight.value
        : n.lineHeight?.units === "PERCENT"
          ? (size * n.lineHeight.value) / 100
          : baseline?.lineHeight;
    const arc = n.arcData as any;
    return {
      id: key,
      fileKey: "clipboard",
      type: n.type ?? "UNKNOWN",
      name: n.name ?? "Figma layer",
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
        type:
          e.type === "FOREGROUND_BLUR"
            ? "LAYER_BLUR"
            : e.type === "GRAIN"
              ? "TEXTURE"
              : e.type,
        visible: e.visible,
        radius: e.radius,
        spread: e.spread,
        offset: e.offset,
        color: e.color,
        blendMode: e.blendMode,
        blurType: e.blurOpType,
        startRadius: e.startRadius,
        startOffset: e.startOffset,
        endOffset: e.endOffset,
        noiseType: e.noiseType,
        noiseSize:
          typeof e.noiseSize === "number" ? e.noiseSize : e.noiseSize?.x,
        density: e.density,
        opacity: e.opacity,
        secondaryColor: e.secondaryColor,
        clipToShape: e.clipToShape,
        lightAngle: e.specularAngle,
        lightIntensity: e.specularIntensity,
        refraction: e.refractionIntensity,
        depth: e.refractionRadius,
        dispersion: e.chromaticAberration,
      })),
      strokeWeight: n.strokeWeight,
      strokeAlign: (["CENTER", "INSIDE", "OUTSIDE"].includes(
        n.strokeAlign ?? "",
      )
        ? n.strokeAlign
        : undefined) as FigmaNode["strokeAlign"],
      dashes: n.dashPattern,
      miterLimit: (n as any).miterLimit,
      pointCount: n.count,
      booleanOperation: n.booleanOperation,
      ...((n as any).borderStrokeWeightsIndependent
        ? {
            strokeWeights: [
              (n as any).borderTopWeight ?? n.strokeWeight ?? 0,
              (n as any).borderRightWeight ?? n.strokeWeight ?? 0,
              (n as any).borderBottomWeight ?? n.strokeWeight ?? 0,
              (n as any).borderLeftWeight ?? n.strokeWeight ?? 0,
            ] as [number, number, number, number],
          }
        : {}),
      cap: n.strokeCap,
      join: n.strokeJoin,
      radii: [
        n.rectangleTopLeftCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleTopRightCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleBottomRightCornerRadius ?? n.cornerRadius ?? 0,
        n.rectangleBottomLeftCornerRadius ?? n.cornerRadius ?? 0,
      ],
      smoothing: n.cornerSmoothing,
      clipsContent:
        ["FRAME", "SYMBOL", "INSTANCE"].includes(n.type ?? "") &&
        n.frameMaskDisabled !== true,
      isMask: n.mask,
      maskType: n.maskType,
      paths,
      strokePaths,
      ...(arc
        ? {
            arc: {
              start: arc.startingAngle ?? 0,
              end: arc.endingAngle ?? Math.PI * 2,
              inner: arc.innerRadius ?? 0,
            },
          }
        : {}),
      ...(n.type === "TEXT"
        ? {
            text: {
              characters: n.textData?.characters ?? "",
              runs,
              verticalAlign: n.textAlignVertical,
              paragraphSpacing: n.paragraphSpacing,
              paragraphIndent: n.paragraphIndent,
              truncation: (n as any).textTruncation,
              maxLines: (n as any).maxLines,
              align: n.textAlignHorizontal,
              autoResize: n.textAutoResize,
              leading,
              tracking:
                n.letterSpacing?.units === "PERCENT"
                  ? (size * n.letterSpacing.value) / 100
                  : n.letterSpacing?.value,
              baseline: baseline ? baseline.position.y - size * 0.82 : 0,
              advanced: !!(
                n.paragraphIndent ||
                n.paragraphSpacing ||
                n.fontVariations?.length ||
                (n.textDecoration && n.textDecoration !== "NONE") ||
                (n.textCase && n.textCase !== "ORIGINAL")
              ),
            },
          }
        : {}),
      unsupported,
      children: (() => {
        const cs = sorted(children).map((c) => build(c, abs, depth + 1));
        return n.type === "BOOLEAN_OPERATION" ? [] : cs;
      })(),
    };
  };
  const nodes = sorted(roots).map((key) => build(key));
  if (!nodes.length || seen.size !== entries.size)
    throw new Error("Invalid Figma hierarchy");
  return validateFigmaDoc({
    format: "powermove-figma",
    version: 1,
    source: "clipboard",
    nodes,
    images,
  });
}
