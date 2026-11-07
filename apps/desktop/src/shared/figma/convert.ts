import { BLEND_NAMES } from "../blend";
import {
  bounds,
  inverse,
  mul,
  type FigmaDoc,
  type FigmaNode,
  type Matrix,
  type Paint,
  type Rect,
} from "./doc";
import { parseSvgPathData } from "./paths";

export type Issue = { id: string; name: string; reason: string };
export type FigmaSource = {
  fileKey: string;
  nodeId: string;
  role: string;
  name: string;
  importedMatteId?: string | null;
  importedEffectIds?: string[];
  importedContentKeys?: string[];
};
/** Declarative edits: refs, parents and matte refs are plan-local. The host resolves
 * IDs/assets/channels and applies the complete list in ONE transaction. */
export type FigmaCommand = {
  type: "add_layer";
  ref: string;
  layerType: string;
  name: string;
  group: string | null;
  content: Record<string, any>;
  properties: Record<string, number>;
  source: FigmaSource;
  visible: boolean;
  locked: boolean;
  blend: string;
  blendVersion?: 2;
  effects: Array<{
    type: string;
    version: number;
    params: Record<string, number | string | boolean>;
  }>;
  matte?: string;
  matteMode?: string;
  mattes?: Array<{ ref: string; mode: string }>;
  imageHash?: string;
  raster?: FigmaNode["raster"];
};
export type FigmaPlan = {
  commands: FigmaCommand[];
  issues: Issue[];
  warnings?: Issue[];
  width: number;
  height: number;
  bounds: Rect;
};
export const containers = new Set([
  "GROUP",
  "FRAME",
  "COMPONENT",
  "COMPONENT_SET",
  "SYMBOL",
  "INSTANCE",
]);
const shapes = new Set(["RECTANGLE", "ROUNDED_RECTANGLE", "ELLIPSE"]);
const vectors = new Set([
  "VECTOR",
  "BOOLEAN_OPERATION",
  "LINE",
  "REGULAR_POLYGON",
  "STAR",
]);
export const blendModes: Record<string, string> = {
  NORMAL: "normal",
  PASS_THROUGH: "normal",
  MULTIPLY: "multiply",
  SCREEN: "screen",
  OVERLAY: "overlay",
  SOFT_LIGHT: "softlight",
  DIFFERENCE: "difference",
  LIGHTEN: "lighten",
  DARKEN: "darken",
  LINEAR_DODGE: "add",
  COLOR_BURN: "colorburn",
  COLOR_DODGE: "colordodge",
  HARD_LIGHT: "hardlight",
  EXCLUSION: "exclusion",
  HUE: "hue",
  SATURATION: "saturation",
  COLOR: "color",
  LUMINOSITY: "luminosity",
  LINEAR_BURN: "linearburn",
};
const nodePaints = (n: FigmaNode) => [
  ...n.fills,
  ...n.strokes,
  ...(n.text?.runs.flatMap((r) => r.fills) ?? []),
];
const paints = (v: Paint[]) =>
  v.filter((p) => p.visible !== false && (p.opacity ?? 1) > 0);
export const hex = (c: NonNullable<Paint["color"]>, opacity = 1) =>
  "#" +
  [c.r, c.g, c.b, (c.a ?? 1) * opacity]
    .map((v) =>
      Math.round(Math.max(0, Math.min(1, v)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")
    .toUpperCase();
const color = (p?: Paint) =>
  p?.color ? hex(p.color, p.opacity ?? 1) : "#00000000";
export function gradient(p: Paint) {
  const type = (
    {
      GRADIENT_LINEAR: "linear",
      GRADIENT_RADIAL: "radial",
      GRADIENT_ANGULAR: "angular",
      GRADIENT_DIAMOND: "diamond",
    } as Record<string, string>
  )[p.type];
  if (!type || !p.gradientStops?.length || !p.gradientTransform)
    return undefined;
  try {
    inverse(p.gradientTransform);
  } catch {
    return undefined;
  }
  return {
    type,
    transform: p.gradientTransform,
    opacity: p.opacity ?? 1,
    stops: p.gradientStops.map((s) => ({
      position: s.position,
      color: hex(s.color),
    })),
  };
}
/** Single capability authority for clipboard, plugin exports and host. Missing
 * source information and genuinely unavailable rendering are explicit fallbacks. */
export function unsupportedReasons(
  node: FigmaNode,
  _inheritedMatte = false,
): string[] {
  const reasons = [...(node.unsupported ?? [])],
    container = containers.has(node.type);
  const m = node.absoluteTransform,
    det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-10) reasons.push("singular transform");
  if (
    !container &&
    !shapes.has(node.type) &&
    !vectors.has(node.type) &&
    node.type !== "TEXT"
  )
    reasons.push(`unsupported ${node.type.toLowerCase()}`);
  if (!blendModes[node.blendMode ?? "NORMAL"])
    reasons.push("unknown blend mode");
  if (
    node.isMask &&
    node.maskType &&
    !["ALPHA", "LUMINANCE", "VECTOR"].includes(node.maskType)
  )
    reasons.push("unsupported mask type");
  if (
    node.join &&
    !["MITER", "BEVEL", "ROUND"].includes(node.join.toUpperCase())
  )
    reasons.push("unsupported stroke join");
  for (const cap of [
    node.cap,
    node.startCap,
    node.endCap,
    ...(node.paths ?? []).flatMap((p) => [p.startCap, p.endCap]),
  ])
    if (
      cap &&
      ![
        "NONE",
        "BUTT",
        "ROUND",
        "SQUARE",
        "ARROW_LINES",
        "ARROW_EQUILATERAL",
        "ARROW_TRIANGLE",
        "DIAMOND_FILLED",
        "CIRCLE_FILLED",
        "TRIANGLE_FILLED",
      ].includes(cap.toUpperCase())
    )
      reasons.push("unsupported stroke cap");
  for (const p of paints(nodePaints(node))) {
    if (p.blendMode && !blendModes[p.blendMode])
      reasons.push("unknown paint blend mode");
    if (p.type === "IMAGE") {
      if (!p.imageHash) reasons.push("missing image bytes");
      if (p.scaleMode && !["FILL", "FIT", "CROP", "TILE"].includes(p.scaleMode))
        reasons.push("unsupported image scale mode");
    } else if (p.type !== "SOLID" && !gradient(p))
      reasons.push("unsupported paint " + p.type.toLowerCase());
  }
  for (const e of node.effects.filter((e) => e.visible !== false)) {
    if (["TEXTURE", "GLASS"].includes(e.type))
      reasons.push(e.type.toLowerCase() + " optical model");
    else if (
      ![
        "DROP_SHADOW",
        "INNER_SHADOW",
        "LAYER_BLUR",
        "BACKGROUND_BLUR",
        "NOISE",
      ].includes(e.type)
    )
      reasons.push("unsupported effect " + e.type.toLowerCase());
  }
  if (vectors.has(node.type)) {
    try {
      const paths = node.paths?.flatMap((p) => parseSvgPathData(p.data));
      if (
        !paths?.length &&
        (!node.strokePaths?.length || paints(node.fills).length) &&
        !["STAR", "REGULAR_POLYGON", "LINE"].includes(node.type)
      )
        reasons.push("missing vector geometry");
      if (
        paths?.some((p) =>
          p.vertices.some((v) =>
            Object.values(v).some(
              (x) => !Number.isFinite(x) || Math.abs(x) > 1e7,
            ),
          ),
        ) ||
        (paths?.reduce((n, p) => n + p.vertices.length, 0) ?? 0) > 50000
      )
        reasons.push("invalid vector geometry");
    } catch {
      reasons.push("invalid vector geometry");
    }
  }
  if (node.type === "TEXT" && !node.text?.runs.length)
    reasons.push("missing text styles");
  if (!container && !vectors.has(node.type) && node.children.length)
    reasons.push("unsupported layer hierarchy");
  return [...new Set(reasons)];
}
/** The same contextual scopes are used before PNG export and before conversion. */
export function fallbackScopes(
  doc: Pick<FigmaDoc, "nodes"> & Partial<Pick<FigmaDoc, "images">>,
): Array<{ node: FigmaNode; reasons: string[] }> {
  const result: Array<{ node: FigmaNode; reasons: string[] }> = [];
  const siblings = (nodes: FigmaNode[], inherited = false) => {
    let active = inherited;
    for (const node of nodes) {
      if (node.visible === false) continue;
      const reasons = unsupportedReasons(node, node.isMask ? false : active);
      if (doc.images)
        for (const p of paints(nodePaints(node)))
          if (p.type === "IMAGE" && p.imageHash && !doc.images[p.imageHash])
            reasons.push("missing image bytes");
      if (reasons.length) result.push({ node, reasons });
      else if (node.type !== "BOOLEAN_OPERATION")
        siblings(node.children, !!node.clipsContent || active);
      if (node.isMask) active = true;
    }
  };
  siblings(doc.nodes);
  return result;
}
const overlap = (a: Rect, b: Rect) =>
  a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
export function selectionBounds(nodes: FigmaNode[]): Rect {
  const bs = nodes.map(bounds),
    x = Math.min(...bs.map((b) => b.x)),
    y = Math.min(...bs.map((b) => b.y));
  return {
    x,
    y,
    w: Math.max(1, ...bs.map((b) => b.x + b.w - x)),
    h: Math.max(1, ...bs.map((b) => b.y + b.h - y)),
  };
}
/** Clipboard PNG has only composite pixels. Promote an unsafe crop to its lowest
 * enclosing subtree. With overlapping roots the only safe scope is all roots. */
export function clipboardFallbacks(doc: FigmaDoc): {
  nodes: FigmaNode[];
  whole: boolean;
} {
  const copy = structuredClone(doc.nodes);
  const flat: FigmaNode[] = [];
  const parents = new Map<FigmaNode, FigmaNode>();
  const visit = (n: FigmaNode) => {
    flat.push(n);
    for (const c of n.children) {
      parents.set(c, n);
      visit(c);
    }
  };
  copy.forEach(visit);
  for (const { node, reasons } of fallbackScopes({
    nodes: copy,
    images: doc.images,
  }))
    node.unsupported = reasons;
  const descendants = (a: FigmaNode, b: FigmaNode): boolean =>
    a === b || a.children.some((c) => descendants(c, b));
  let whole = false;
  for (const n of flat) {
    if (!unsupportedReasons(n).length) continue;
    let scope = n;
    while (true) {
      const b = bounds(scope);
      const conflicts = flat.filter(
        (other) =>
          other.visible !== false &&
          !descendants(scope, other) &&
          (!descendants(other, scope) ||
            paints(other.fills).length > 0 ||
            paints(other.strokes).length > 0) &&
          overlap(b, bounds(other)),
      );
      if (!conflicts.length) break;
      const parent = parents.get(scope);
      if (!parent) {
        whole = true;
        break;
      }
      scope = parent;
    }
    if (scope !== n)
      scope.unsupported = [
        ...(scope.unsupported ?? []),
        "overlapping clipboard preview",
      ];
  }
  return { nodes: copy, whole };
}
function blurParameters(
  e: FigmaNode["effects"][number],
): Record<string, number | boolean> {
  return {
    radius: e.radius ?? 0,
    progressive: e.blurType === "PROGRESSIVE",
    startRadius: e.startRadius ?? 0,
    startX: e.startOffset?.x ?? 0,
    startY: e.startOffset?.y ?? 0,
    endX: e.endOffset?.x ?? 1,
    endY: e.endOffset?.y ?? 0,
  };
}
export function convertFigma(
  doc: FigmaDoc,
  composition: { w: number; h: number },
): FigmaPlan {
  const box = selectionBounds(doc.nodes),
    commands: FigmaCommand[] = [],
    issues: Issue[] = [];
  const offset = {
    x: (composition.w - box.w) / 2 - box.x,
    y: (composition.h - box.h) / 2 - box.y,
  };
  type Matte = { ref: string; mode: string };
  const build = (
    node: FigmaNode,
    parent: FigmaNode | null,
    group: string | null,
    inherited: Matte[] = [],
  ): FigmaCommand | undefined => {
    if (node.visible === false) return;
    const reasons = unsupportedReasons(node),
      m = parent
        ? mul(inverse(parent.absoluteTransform), node.absoluteTransform)
        : node.absoluteTransform;
    for (const paint of nodePaints(node))
      if (
        paint.type === "IMAGE" &&
        paint.imageHash &&
        !doc.images[paint.imageHash]
      )
        reasons.push("missing image bytes");
    const sx = Math.hypot(m[0], m[1]),
      sy = (m[0] * m[3] - m[1] * m[2]) / (sx || 1);
    const props: Record<string, number> = {
      "position.x": m[4] + (group ? 0 : offset.x),
      "position.y": m[5] + (group ? 0 : offset.y),
      "anchor.x": 0,
      "anchor.y": 0,
      "scale.x": sx * 100,
      "scale.y": sy * 100,
      rotation: (Math.atan2(m[1], m[0]) * 180) / Math.PI,
      skew:
        (Math.atan((m[0] * m[2] + m[1] * m[3]) / (sx * sy)) * 180) / Math.PI ||
        0,
      opacity: (node.opacity ?? 1) * 100,
    };
    const local = {
      ...props,
      "position.x": 0,
      "position.y": 0,
      "scale.x": 100,
      "scale.y": 100,
      rotation: 0,
      skew: 0,
      opacity: 100,
    };
    const add = (
      layerType: string,
      content: Record<string, any>,
      role = "node",
      p = props,
      owner = group,
      mattes = inherited,
    ): FigmaCommand => {
      const c: FigmaCommand = {
        type: "add_layer",
        ref: node.id + ":" + role,
        layerType,
        name: role === "node" ? node.name : node.name + " " + role,
        group: owner,
        content:
          layerType === "group"
            ? { isolate: node.blendMode !== "PASS_THROUGH", ...content }
            : content,
        properties: { ...p },
        source: {
          fileKey: node.fileKey,
          nodeId: node.id,
          role,
          name: role === "node" ? node.name : node.name + " " + role,
        },
        visible: !node.isMask && role !== "clip" && !role.endsWith("-clip"),
        locked: !!node.locked,
        blend: blendModes[node.blendMode ?? "NORMAL"] ?? "normal",
        blendVersion: 2,
        effects: [],
      };
      if ((layerType === "shape" && !content.paths) || layerType === "image") {
        c.properties["anchor.x"] = -node.w / 2;
        c.properties["anchor.y"] = -node.h / 2;
      }
      if (mattes.length) {
        c.matte = mattes[0]!.ref;
        c.matteMode = mattes[0]!.mode;
        c.mattes = mattes;
      }
      commands.push(c);
      return c;
    };
    if (reasons.length) {
      issues.push({
        id: node.id,
        name: node.name,
        reason: [...new Set(reasons)].join(", "),
      });
      const b = node.raster?.bounds ?? bounds(node),
        rm: Matrix = [1, 0, 0, 1, b.x, b.y],
        lm = parent ? mul(inverse(parent.absoluteTransform), rm) : rm;
      const c = add("image", { w: b.w, h: b.h, fit: "stretch" }, "node", {
        "position.x": lm[4] + (group ? 0 : offset.x),
        "position.y": lm[5] + (group ? 0 : offset.y),
        "anchor.x": -b.w / 2,
        "anchor.y": -b.h / 2,
        "scale.x": Math.hypot(lm[0], lm[1]) * 100,
        "scale.y":
          ((lm[0] * lm[3] - lm[1] * lm[2]) / Math.hypot(lm[0], lm[1])) * 100,
        rotation: (Math.atan2(lm[1], lm[0]) * 180) / Math.PI,
        skew:
          (Math.atan(
            (lm[0] * lm[2] + lm[1] * lm[3]) / (lm[0] * lm[3] - lm[1] * lm[2]),
          ) *
            180) /
          Math.PI,
        opacity: 100,
      });
      c.properties["anchor.x"] = -b.w / 2;
      c.properties["anchor.y"] = -b.h / 2;
      c.raster = node.raster;
      return c;
    }
    if (node.strokePaths?.length && paints(node.strokes).length) {
      const fill: FigmaNode = {
        ...node,
        id: node.id + "~fill",
        name: node.name + " fill",
        strokes: [],
        strokePaths: undefined,
        opacity: 1,
        blendMode: "NORMAL",
        effects: [],
        isMask: false,
      };
      const stroke: FigmaNode = {
        ...node,
        id: node.id + "~outline",
        name: node.name + " stroke",
        type: "VECTOR",
        fills: node.strokes,
        strokes: [],
        paths: node.strokePaths,
        strokePaths: undefined,
        children: [],
        opacity: 1,
        blendMode: "NORMAL",
        effects: [],
        isMask: false,
        unsupported: [],
      };
      const root: FigmaNode = {
        ...node,
        type: "GROUP",
        fills: [],
        strokes: [],
        paths: undefined,
        strokePaths: undefined,
        children: [
          ...(paints(node.fills).length || node.children.length ? [fill] : []),
          stroke,
        ],
        unsupported: [],
      };
      const start = commands.length;
      const result = build(root, parent, group, inherited);
      for (const c of commands.slice(start))
        if (c.source.nodeId.startsWith(node.id + "~")) {
          c.source.role =
            c.source.nodeId.slice(node.id.length + 1) + ":" + c.source.role;
          c.source.nodeId = node.id;
        }
      return result;
    }
    const fills = paints(node.fills),
      strokes = paints(node.strokes),
      r = node.radii ?? [0, 0, 0, 0];
    const geometry = (fill?: Paint, stroke?: Paint): Record<string, any> => {
      const d: Record<string, any> = {
        shape:
          node.type === "ELLIPSE"
            ? "ellipse"
            : node.type === "STAR"
              ? "star"
              : node.type === "REGULAR_POLYGON"
                ? "polygon"
                : node.type === "LINE"
                  ? "line"
                  : "rect",
        w: node.w,
        h: node.h,
        color: color(fill),
        stroke: stroke
          ? (node.strokeWeight ??
            (node.strokeWeights ? Math.max(...node.strokeWeights) : 1))
          : 0,
        strokeColor: color(stroke),
        radius: r[0],
        independentCorners: new Set(r).size > 1,
        radiusTL: r[0],
        radiusTR: r[1],
        radiusBR: r[2],
        radiusBL: r[3],
        smoothing: (node.smoothing ?? 0) * 100,
        points: node.pointCount ?? 5,
      };
      if (node.arc) d.arc = node.arc;
      if (fill && gradient(fill)) d.fillGradient = gradient(fill);
      if (stroke && gradient(stroke)) d.strokeGradient = gradient(stroke);
      if (stroke)
        d.strokeStyle = {
          align: (node.strokeAlign ?? "CENTER").toLowerCase(),
          dashes: node.dashes ?? [],
          dashOffset: 0,
          cap: node.cap ?? "NONE",
          startCap: node.startCap ?? node.cap ?? "NONE",
          endCap: node.endCap ?? node.cap ?? "NONE",
          join: (node.join ?? "MITER").toLowerCase(),
          miterLimit: node.miterLimit ?? 4,
          // Figma reports all four side weights even for a uniform stroke.
          // Retain independent sides only when they actually differ.
          ...(node.strokeWeights && new Set(node.strokeWeights).size > 1
            ? { weights: node.strokeWeights }
            : {}),
        };
      if (node.paths?.length) {
        d.compound = true;
        d.fillRule =
          node.paths[0]!.windingRule === "EVENODD" ? "evenodd" : "nonzero";
        d.paths = node.paths
          .flatMap((p, groupIndex) =>
            parseSvgPathData(p.data).map((path) => ({
              ...path,
              startCap: p.startCap,
              endCap: p.endCap,
              fillGroup: String(groupIndex),
              fillRule: p.windingRule === "EVENODD" ? "evenodd" : "nonzero",
            })),
          )
          .map((path) => ({
            ...path,
            name: node.name,
            fill: color(fill).slice(0, 7),
            fillEnabled: !!fill,
            fillOpacity: 100,
            stroke: color(stroke).slice(0, 7),
            strokeWidth: stroke ? (node.strokeWeight ?? 1) : 0,
            strokeOpacity: 100,
          }));
      }
      return d;
    };
    const effectCommands = (c: FigmaCommand) => {
      for (const e of node.effects.filter((e) => e.visible !== false)) {
        if (e.type === "DROP_SHADOW" || e.type === "INNER_SHADOW")
          c.effects.push({
            type: "figmaShadow",
            version: 1,
            params: {
              color: hex(e.color ?? { r: 0, g: 0, b: 0 }).slice(0, 7),
              opacity: (e.color?.a ?? 1) * 100,
              offsetX: e.offset?.x ?? 0,
              offsetY: e.offset?.y ?? 0,
              radius: e.radius ?? 0,
              spread: e.spread ?? 0,
              inner: e.type === "INNER_SHADOW",
              blendMode: Math.max(
                0,
                e.blendMode === "SOFT_LIGHT"
                  ? 18
                  : BLEND_NAMES.indexOf(
                      (blendModes[e.blendMode ?? "NORMAL"] ?? "normal") as any,
                    ),
              ),
            },
          });
        else if (e.type === "LAYER_BLUR" && e.blurType === "PROGRESSIVE")
          c.effects.push({
            type: "figmaLayerBlur",
            version: 1,
            params: blurParameters(e),
          });
        else if (e.type === "LAYER_BLUR")
          c.effects.push({
            type: "blur",
            version: 2,
            params: {
              amount: e.radius ?? 0,
              dimensions: 0,
              repeatEdgePixels: false,
            },
          });
        else if (e.type === "NOISE")
          c.effects.push({
            type: "figmaNoise",
            version: 1,
            params: {
              size: e.noiseSize ?? 1,
              density: e.density ?? 1,
              color: hex(e.color ?? { r: 0, g: 0, b: 0 }).slice(0, 7),
              secondaryColor: hex(
                e.secondaryColor ?? e.color ?? { r: 0, g: 0, b: 0 },
              ).slice(0, 7),
              opacity: (e.opacity ?? e.color?.a ?? 1) * 100,
              blendMode: Math.max(
                0,
                e.blendMode === "SOFT_LIGHT"
                  ? 18
                  : BLEND_NAMES.indexOf(
                      (blendModes[e.blendMode ?? "NORMAL"] ?? "normal") as any,
                    ),
              ),
              noiseType:
                e.noiseType === "DUOTONE"
                  ? 1
                  : e.noiseType === "MULTITONE"
                    ? 2
                    : 0,
            },
          });
      }
    };
    const textContent = () => {
      const t = node.text!,
        run = t.runs[0]!;
      const weight = (r: typeof run) =>
        r.weight ??
        (/black|heavy/i.test(r.style)
          ? 900
          : /extra.?bold/i.test(r.style)
            ? 800
            : /semi.?bold|demi/i.test(r.style)
              ? 600
              : /bold/i.test(r.style)
                ? 700
                : /medium/i.test(r.style)
                  ? 500
                  : /light/i.test(r.style)
                    ? 300
                    : 400);
      return {
        text: t.characters,
        font: run.font,
        size: run.size,
        weight: weight(run),
        italic: /italic|oblique/i.test(run.style),
        color: color(run.fills[0]),
        align: (t.align ?? "LEFT")
          .toLowerCase()
          .replace("justified", "justify"),
        boxWidth: t.autoResize === "WIDTH_AND_HEIGHT" ? 0 : node.w,
        boxHeight: ["WIDTH_AND_HEIGHT", "HEIGHT"].includes(t.autoResize ?? "")
          ? 0
          : node.h,
        tracking: t.tracking ?? run.tracking ?? 0,
        leading: (t.leading ?? run.leading ?? run.size * 1.2) / run.size,
        verticalAlign: (t.verticalAlign ?? "TOP").toLowerCase(),
        paragraphSpacing: t.paragraphSpacing ?? 0,
        paragraphIndent: t.paragraphIndent ?? 0,
        truncation:
          t.truncation ?? (t.autoResize === "TRUNCATE" ? "ENDING" : "DISABLED"),
        maxLines: t.maxLines ?? 0,
        textStroke: strokes.length ? (node.strokeWeight ?? 1) : 0,
        textStrokeColor: color(strokes[0]),
        strokeGradient: strokes[0] && gradient(strokes[0]),
        strokeStyle: {
          align: (node.strokeAlign ?? "CENTER").toLowerCase(),
          dashes: node.dashes ?? [],
          join: (node.join ?? "MITER").toLowerCase(),
          miterLimit: node.miterLimit ?? 4,
        },
        richText: true,
        runs: t.runs.map((r) => ({
          start: r.start,
          end: r.end,
          font: r.font,
          size: r.size,
          weight: weight(r),
          italic: /italic|oblique/i.test(r.style),
          color: color(r.fills.find((p) => p.type !== "IMAGE")),
          tracking: r.tracking ?? t.tracking ?? 0,
          leading: r.leading ?? t.leading ?? r.size * 1.2,
          fillGradient:
            r.fills.find((p) => p.type !== "IMAGE") &&
            gradient(r.fills.find((p) => p.type !== "IMAGE")!),
          paints:
            r.fills.filter((p) => p.type !== "IMAGE").length > 1
              ? r.fills
                  .filter((p) => p.type !== "IMAGE")
                  .map((p) => ({
                    color: color(p),
                    gradient: gradient(p),
                    blend: blendModes[p.blendMode ?? "NORMAL"],
                  }))
              : undefined,
          decoration: r.decoration ?? "NONE",
          textCase: r.textCase ?? "ORIGINAL",
          paragraphSpacing: r.paragraphSpacing ?? t.paragraphSpacing ?? 0,
          paragraphIndent: r.paragraphIndent ?? t.paragraphIndent ?? 0,
          listSpacing: r.listSpacing ?? 0,
          indentation: r.indentation ?? 0,
          listType: r.listType ?? "NONE",
        })),
      };
    };
    const paintLayer = (
      paint: Paint | undefined,
      stroke: Paint | undefined,
      role: string,
      owner: string | null,
      p: Record<string, number>,
      mattes: Matte[] = [],
    ) => {
      const image =
        paint?.type === "IMAGE"
          ? paint
          : stroke?.type === "IMAGE"
            ? stroke
            : undefined;
      if (image || (node.type === "TEXT" && paint && gradient(paint))) {
        const mask = add(
          node.type === "TEXT" ? "text" : "shape",
          node.type === "TEXT"
            ? {
                ...textContent(),
                color: "#FFFFFFFF",
                textPaint: stroke ? "stroke" : "fill",
                textStroke: stroke ? (node.strokeWeight ?? 1) : 0,
                textStrokeColor: "#FFFFFFFF",
                strokeGradient: undefined,
                runs: textContent().runs.map((r) => ({
                  ...r,
                  color: "#FFFFFFFF",
                  fillGradient: undefined,
                  paints: [{ color: "#FFFFFFFF" }],
                })),
              }
            : geometry(
                paint
                  ? { type: "SOLID", color: { r: 1, g: 1, b: 1 } }
                  : undefined,
                stroke
                  ? { type: "SOLID", color: { r: 1, g: 1, b: 1 } }
                  : undefined,
              ),
          role + "-clip",
          p,
          owner,
          [],
        );
        mask.blend = "normal";
        if (node.type === "TEXT") {
          mask.properties["position.x"]! +=
            node.text?.align === "CENTER"
              ? node.w / 2
              : node.text?.align === "RIGHT"
                ? node.w
                : 0;
          mask.properties["position.y"]! += node.text?.baseline ?? 0;
        }
        if (!image) {
          const c = add("shape", geometry(paint), role, p, owner, [
            ...mattes,
            { ref: mask.ref, mode: "alpha" },
          ]);
          c.blend = blendModes[paint?.blendMode ?? "NORMAL"] ?? "normal";
          return c;
        }
        const c = add(
          "image",
          {
            w: node.w,
            h: node.h,
            fit: "stretch",
            imageMapping: {
              mode: (image.scaleMode ?? "FILL").toLowerCase(),
              transform: image.imageTransform,
              scale: image.scalingFactor ?? 1,
              rotation: image.rotation ?? 0,
            },
          },
          role,
          p,
          owner,
          [...mattes, { ref: mask.ref, mode: "alpha" }],
        );
        c.imageHash = image.imageHash;
        c.properties.opacity = (image.opacity ?? 1) * 100;
        c.blend = blendModes[image.blendMode ?? "NORMAL"] ?? "normal";
        return c;
      }
      const c = add(
        node.type === "TEXT" ? "text" : "shape",
        node.type === "TEXT"
          ? {
              ...textContent(),
              ...(stroke
                ? {
                    textPaint: "stroke",
                    textStrokeColor: color(stroke),
                    strokeGradient: gradient(stroke),
                  }
                : {}),
              ...(paint
                ? {
                    color: color(paint),
                    textStroke: 0,
                    runs: textContent().runs.map((r) => ({
                      ...r,
                      color: color(paint),
                      textStroke: 0,
                      paints: [{ color: color(paint) }],
                      fillGradient: undefined,
                    })),
                  }
                : {}),
            }
          : geometry(paint, stroke),
        role,
        p,
        owner,
        mattes,
      );
      c.blend =
        blendModes[(paint ?? stroke)?.blendMode ?? "NORMAL"] ?? "normal";
      return c;
    };
    if (containers.has(node.type)) {
      const c = add("group", {});
      effectCommands(c);
      let clip: Matte[] = [];
      if (node.clipsContent) {
        const mask = add(
          "shape",
          geometry({ type: "SOLID", color: { r: 1, g: 1, b: 1 } }),
          "clip",
          local,
          c.ref,
          [],
        );
        mask.blend = "normal";
        clip = [{ ref: mask.ref, mode: "alpha" }];
      }
      siblings(node.children, node, c.ref, clip);
      for (let i = 0; i < strokes.length; i++)
        paintLayer(undefined, strokes[i], `stroke-${i}`, c.ref, local);
      for (let i = 0; i < fills.length; i++)
        paintLayer(fills[i], undefined, `fill-${i}`, c.ref, local);
      for (const [i, e] of node.effects.entries())
        if (e.visible !== false && e.type === "BACKGROUND_BLUR") {
          const bg = add(
            "shape",
            geometry({ type: "SOLID", color: { r: 1, g: 1, b: 1 } }),
            `backdrop-${i}`,
            local,
            c.ref,
            [],
          );
          bg.blend = "normal";
          bg.effects = [
            {
              type: "figmaBackdropBlur",
              version: 2,
              params: blurParameters(e),
            },
          ];
        }
      return c;
    }
    const hasBackdrop = node.effects.some(
      (e) => e.visible !== false && e.type === "BACKGROUND_BLUR",
    );
    const stacked =
      hasBackdrop ||
      (node.type === "TEXT" && !!fills[0] && !!gradient(fills[0])) ||
      fills.length > 1 ||
      strokes.length > 1 ||
      [...fills, ...strokes].some((p) => p.type === "IMAGE") ||
      (fills[0]?.blendMode ?? "NORMAL") !== "NORMAL";
    let c: FigmaCommand;
    const runImages =
      node.type === "TEXT"
        ? node.text!.runs.flatMap((r, index) =>
            r.fills
              .filter((p) => p.type === "IMAGE")
              .map((p) => ({ paint: p, index })),
          )
        : [];
    if (runImages.length) {
      c = add("group", {});
      const content = textContent();
      const body = add("text", content, "text", local, c.ref, []);
      body.blend = "normal";
      body.properties["position.x"]! +=
        node.text?.align === "CENTER"
          ? node.w / 2
          : node.text?.align === "RIGHT"
            ? node.w
            : 0;
      body.properties["position.y"]! += node.text?.baseline ?? 0;
      for (const [i, item] of runImages.entries()) {
        paintLayer(item.paint, undefined, `text-image-${i}`, c.ref, local);
        const mask = commands.find(
          (x) => x.ref === node.id + `:text-image-${i}-clip`,
        )!;
        mask.content = {
          ...content,
          runs: content.runs.map((r, index) => ({
            ...r,
            color: index === item.index ? "#FFFFFFFF" : "#00000000",
            fillGradient: undefined,
            paints: [
              { color: index === item.index ? "#FFFFFFFF" : "#00000000" },
            ],
          })),
        };
      }
    } else if (stacked) {
      c = add("group", {});
      for (let i = 0; i < strokes.length; i++)
        paintLayer(undefined, strokes[i], `stroke-${i}`, c.ref, local);
      for (let i = 0; i < fills.length; i++)
        paintLayer(fills[i], undefined, `fill-${i}`, c.ref, local);
    } else
      c = add(
        node.type === "TEXT" ? "text" : "shape",
        node.type === "TEXT" ? textContent() : geometry(fills[0], strokes[0]),
      );
    if (node.type === "TEXT" && !stacked && !runImages.length) {
      const align = node.text?.align,
        ox = align === "CENTER" ? node.w / 2 : align === "RIGHT" ? node.w : 0;
      c.properties["position.x"]! += m[0] * ox;
      c.properties["position.x"]! += m[2] * (node.text?.baseline ?? 0);
      c.properties["position.y"]! +=
        m[1] * ox + m[3] * (node.text?.baseline ?? 0);
    }
    if (node.isMask && node.maskType === "VECTOR") {
      c.content.color = "#FFFFFFFF";
      if (c.content.fillGradient) delete c.content.fillGradient;
    }
    for (const [i, e] of node.effects.entries())
      if (e.visible !== false && e.type === "BACKGROUND_BLUR") {
        const bg = add(
          "shape",
          geometry({ type: "SOLID", color: { r: 1, g: 1, b: 1 } }),
          `backdrop-${i}`,
          local,
          c.ref,
          [],
        );
        bg.blend = "normal";
        bg.effects = [
          { type: "figmaBackdropBlur", version: 2, params: blurParameters(e) },
        ];
      }
    effectCommands(c);
    return c;
  };
  const siblings = (
    nodes: FigmaNode[],
    parent: FigmaNode | null,
    group: string | null,
    clips: Matte[] = [],
  ) => {
    const active = new Map<FigmaNode, Matte[]>();
    let mask: Matte | undefined;
    for (const node of nodes) {
      if (node.isMask && node.visible !== false)
        mask = {
          ref: node.id + ":node",
          mode:
            node.maskType === "LUMINANCE"
              ? "luma"
              : node.maskType === "VECTOR"
                ? "vector"
                : "alpha",
        };
      else active.set(node, [...clips, ...(mask ? [mask] : [])]);
    }
    for (const node of [...nodes].reverse())
      build(node, parent, group, node.isMask ? [] : active.get(node));
  };
  siblings(doc.nodes, null, null);
  return { commands, issues, width: box.w, height: box.h, bounds: box };
}
