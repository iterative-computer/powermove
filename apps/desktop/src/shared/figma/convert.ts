import { bounds, inverse, mul, type FigmaDoc, type FigmaNode, type Matrix, type Paint, type Rect } from './doc';
import { parseSvgPathData } from './paths';

export type Issue = { id: string; name: string; reason: string };
export type FigmaSource = {
  fileKey: string;
  nodeId: string;
  role: string;
  name: string;
  importedMatteId?: string | null;
  importedEffectIds?: string[];
};
/** Declarative edits: refs, parents and matte refs are plan-local. The host resolves
 * IDs/assets/channels and applies the complete list in ONE transaction. */
export type FigmaCommand = {
  type: 'add_layer';
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
  effects: Array<{ type: string; version: number; params: Record<string, number | string | boolean> }>;
  matte?: string;
  raster?: FigmaNode['raster'];
};
export type FigmaPlan = { commands: FigmaCommand[]; issues: Issue[]; width: number; height: number; bounds: Rect };
export const containers = new Set(['GROUP', 'FRAME', 'COMPONENT', 'COMPONENT_SET', 'SYMBOL', 'INSTANCE']);
const shapes = new Set(['RECTANGLE', 'ROUNDED_RECTANGLE', 'ELLIPSE']);
const vectors = new Set(['VECTOR', 'BOOLEAN_OPERATION', 'LINE', 'REGULAR_POLYGON', 'STAR']);
export const blendModes: Record<string, string> = {
  NORMAL: 'normal',
  PASS_THROUGH: 'normal',
  MULTIPLY: 'multiply',
  SCREEN: 'screen',
  OVERLAY: 'overlay',
  SOFT_LIGHT: 'softlight',
  DIFFERENCE: 'difference',
  LIGHTEN: 'lighten',
  DARKEN: 'darken',
  LINEAR_DODGE: 'add',
};
const paints = (v: Paint[]) => v.filter((p) => p.visible !== false && (p.opacity ?? 1) > 0);
export const hex = (c: NonNullable<Paint['color']>, opacity = 1) =>
  '#' +
  [c.r, c.g, c.b, (c.a ?? 1) * opacity]
    .map((v) =>
      Math.round(Math.max(0, Math.min(1, v)) * 255)
        .toString(16)
        .padStart(2, '0'),
    )
    .join('')
    .toUpperCase();
const color = (p?: Paint) => (p?.color ? hex(p.color, p.opacity ?? 1) : '#00000000');
function gradient(p: Paint, node: FigmaNode) {
  if (!['GRADIENT_LINEAR', 'GRADIENT_RADIAL'].includes(p.type) || !p.gradientTransform || p.gradientStops?.length !== 2)
    return false;
  const [a, b] = p.gradientStops;
  if (a!.position !== 0 || b!.position !== 1 || [a!, b!].some((s) => (s.color.a ?? 1) !== 1) || (p.opacity ?? 1) !== 1)
    return false;
  try {
    const m = inverse(p.gradientTransform);
    const sx = m[0] * 0 + m[2] * 0.5 + m[4],
      sy = m[1] * 0 + m[3] * 0.5 + m[5];
    const ex = m[0] + m[2] * 0.5 + m[4],
      ey = m[1] + m[3] * 0.5 + m[5];
    // The radial ramp supports circles; affine elliptical/skewed ramps need PNG.
    if (
      p.type === 'GRADIENT_RADIAL' &&
      (Math.abs(Math.hypot(m[0] * node.w, m[1] * node.h) - Math.hypot(m[2] * node.w, m[3] * node.h)) > 0.01 ||
        Math.abs(m[0] * m[2] * node.w ** 2 + m[1] * m[3] * node.h ** 2) > 0.01)
    )
      return false;
    return {
      type: 'gradient',
      version: 2,
      params: {
        startX: (p.type === 'GRADIENT_RADIAL' ? (sx + ex) / 2 : sx) * node.w,
        startY: (p.type === 'GRADIENT_RADIAL' ? (sy + ey) / 2 : sy) * node.h,
        endX: ex * node.w,
        endY: ey * node.h,
        startColor: hex(a!.color).slice(0, 7),
        endColor: hex(b!.color).slice(0, 7),
        rampShape: p.type === 'GRADIENT_RADIAL' ? 1 : 0,
        rampScatter: 0,
        blendWithOriginal: 0,
      },
    };
  } catch {
    return false;
  }
}
/** Shared by the plugin BEFORE exportAsync, and by every host. Never silently
 * approximate unsupported drawing styles. A reason means rasterize this subtree. */
export function unsupportedReasons(node: FigmaNode, inheritedMatte = false): string[] {
  const reasons = [...(node.unsupported ?? [])],
    fills = paints(node.fills),
    strokes = paints(node.strokes),
    container = containers.has(node.type);
  if (inheritedMatte && (node.clipsContent || node.children.some((child) => child.isMask)))
    reasons.push('nested clipping');
  const m = node.absoluteTransform,
    sx = Math.hypot(m[0], m[1]),
    det = m[0] * m[3] - m[1] * m[2];
  if (!sx || Math.abs(det) < 1e-8 || Math.abs((Math.atan((m[0] * m[2] + m[1] * m[3]) / det) * 180) / Math.PI) > 80)
    reasons.push('unsupported transform');
  if (!container && !shapes.has(node.type) && !vectors.has(node.type) && node.type !== 'TEXT')
    reasons.push(`unsupported ${node.type.toLowerCase()}`);
  if (!blendModes[node.blendMode ?? 'NORMAL']) reasons.push('blend mode');
  for (const p of [...fills, ...strokes]) {
    if (p.blendMode && !['NORMAL', 'PASS_THROUGH'].includes(p.blendMode)) reasons.push('fill blend mode');
    if (
      p.type !== 'SOLID' &&
      (!fills.includes(p) ||
        !gradient(p, node) ||
        fills.length > 1 ||
        strokes.length ||
        container ||
        node.type === 'TEXT' ||
        vectors.has(node.type))
    )
      reasons.push(p.type === 'IMAGE' ? 'image fill' : 'complex gradient');
  }
  if ((fills.length > 1 || strokes.length > 1) && node.effects.some((e) => e.visible !== false))
    reasons.push('effects on stacked paints');
  if (node.effects.some((e) => e.visible !== false)) {
    if (container) reasons.push('group effects');
    for (const e of node.effects.filter((e) => e.visible !== false)) {
      if (
        !['DROP_SHADOW', 'LAYER_BLUR'].includes(e.type) ||
        (e.spread ?? 0) !== 0 ||
        (e.blendMode && !['NORMAL', 'MULTIPLY'].includes(e.blendMode)) ||
        e.radius == null ||
        (e.type === 'DROP_SHADOW' && (!e.color || !e.offset))
      )
        reasons.push(
          e.type === 'BACKGROUND_BLUR'
            ? 'background blur'
            : e.type === 'INNER_SHADOW'
              ? 'inner shadow'
              : 'unsupported effect',
        );
    }
  }
  if (strokes.length) {
    if (node.type === 'TEXT') reasons.push('text stroke');
    // The engine has centred strokes; avoid changing vector contours to fake alignment.
    if (node.strokeAlign && node.strokeAlign !== 'CENTER') reasons.push('inside or outside stroke');
    if (node.dashes?.length) reasons.push('dashed stroke');
    if (node.cap && !['NONE', 'BUTT'].includes(node.cap)) reasons.push('stroke end caps');
    if (node.join && node.join !== (vectors.has(node.type) ? 'MITER' : 'ROUND')) reasons.push('stroke joins');
  }
  if (node.arc && ((node.arc.inner ?? 0) > 0 || Math.abs(node.arc.end - node.arc.start - Math.PI * 2) > 1e-5))
    reasons.push('partial ellipse');
  if (node.isMask && container) reasons.push('group mask');
  if (node.isMask && node.maskType && node.maskType !== 'ALPHA') reasons.push('non-alpha mask');
  if (vectors.has(node.type)) {
    if (fills.length > 1 || strokes.length > 1) reasons.push('multiple vector paints');
    try {
      const contours = node.paths?.flatMap((p) => parseSvgPathData(p.data));
      if (
        !contours?.length ||
        contours.some((p) =>
          p.vertices.some((v) => Object.values(v).some((x) => !Number.isFinite(x) || Math.abs(x) > 1e7)),
        ) ||
        contours.reduce((sum, p) => sum + p.vertices.length, 0) > 50000 ||
        (fills.length && (contours.length > 1 || contours.some((p) => !p.closed)))
      )
        reasons.push('complex vector geometry');
    } catch {
      reasons.push('complex vector geometry');
    }
  }
  if (node.type === 'TEXT') {
    if (!node.text?.runs.length) reasons.push('missing text styles');
    if (
      node.text?.advanced ||
      node.text?.runs.some(
        (r) => (r.decoration && r.decoration !== 'NONE') || (r.textCase && r.textCase !== 'ORIGINAL'),
      )
    )
      reasons.push('advanced text formatting');
    if (node.text?.align && !['LEFT', 'CENTER', 'RIGHT'].includes(node.text.align)) reasons.push('justified text');
    // Range colors are supported by text styles; layout-changing mixed metrics need PNG.
    const first = node.text?.runs[0];
    if (
      node.text?.runs.some(
        (r) =>
          r.font !== first?.font ||
          r.size !== first?.size ||
          r.style !== first?.style ||
          r.fills.length !== 1 ||
          r.fills[0]?.type !== 'SOLID',
      )
    )
      reasons.push('mixed text metrics');
  }
  if (!container && node.children.length) reasons.push('unsupported layer hierarchy');
  // A single matte channel cannot intersect nested clips/masks.
  if (
    node.clipsContent &&
    node.children.some(function nested(n): boolean {
      return !!n.clipsContent || !!n.isMask || n.children.some(nested);
    })
  )
    reasons.push('nested clipping');
  return [...new Set(reasons)];
}
/** The same contextual scopes are used before PNG export and before conversion. */
export function fallbackScopes(doc: Pick<FigmaDoc, 'nodes'>): Array<{ node: FigmaNode; reasons: string[] }> {
  const result: Array<{ node: FigmaNode; reasons: string[] }> = [];
  const siblings = (nodes: FigmaNode[], inherited = false) => {
    let active = inherited;
    for (const node of nodes) {
      if (node.visible === false) continue;
      const reasons = unsupportedReasons(node, node.isMask ? false : active);
      if (reasons.length) result.push({ node, reasons });
      else siblings(node.children, !!node.clipsContent || active);
      if (node.isMask) active = true;
    }
  };
  siblings(doc.nodes);
  return result;
}
const overlap = (a: Rect, b: Rect) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
export function selectionBounds(nodes: FigmaNode[]): Rect {
  const bs = nodes.map(bounds),
    x = Math.min(...bs.map((b) => b.x)),
    y = Math.min(...bs.map((b) => b.y));
  return { x, y, w: Math.max(1, ...bs.map((b) => b.x + b.w - x)), h: Math.max(1, ...bs.map((b) => b.y + b.h - y)) };
}
/** Clipboard PNG has only composite pixels. Promote an unsafe crop to its lowest
 * enclosing subtree. With overlapping roots the only safe scope is all roots. */
export function clipboardFallbacks(doc: FigmaDoc): { nodes: FigmaNode[]; whole: boolean } {
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
  for (const { node, reasons } of fallbackScopes({ nodes: copy })) node.unsupported = reasons;
  const descendants = (a: FigmaNode, b: FigmaNode): boolean => a === b || a.children.some((c) => descendants(c, b));
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
          (!descendants(other, scope) || paints(other.fills).length > 0 || paints(other.strokes).length > 0) &&
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
    if (scope !== n) scope.unsupported = [...(scope.unsupported ?? []), 'overlapping clipboard preview'];
  }
  return { nodes: copy, whole };
}
export function convertFigma(doc: FigmaDoc, composition: { w: number; h: number }): FigmaPlan {
  const box = selectionBounds(doc.nodes),
    commands: FigmaCommand[] = [],
    issues: Issue[] = [];
  const offset = { x: (composition.w - box.w) / 2 - box.x, y: (composition.h - box.h) / 2 - box.y };
  const build = (node: FigmaNode, parent: FigmaNode | null, group: string | null, matte?: string) => {
    if (node.visible === false) return;
    const reasons = unsupportedReasons(node, !!matte),
      m = parent ? mul(inverse(parent.absoluteTransform), node.absoluteTransform) : node.absoluteTransform;
    const sx = Math.hypot(m[0], m[1]),
      sy = (m[0] * m[3] - m[1] * m[2]) / (sx || 1);
    const p = {
      'position.x': m[4] + (group ? 0 : offset.x),
      'position.y': m[5] + (group ? 0 : offset.y),
      'anchor.x': 0,
      'anchor.y': 0,
      'scale.x': sx * 100,
      'scale.y': sy * 100,
      rotation: (Math.atan2(m[1], m[0]) * 180) / Math.PI,
      skew: (Math.atan((m[0] * m[2] + m[1] * m[3]) / (sx * sy)) * 180) / Math.PI || 0,
      opacity: (node.opacity ?? 1) * 100,
    };
    const add = (
      layerType: string,
      content: Record<string, any>,
      role = 'node',
      props = p,
      owner = group,
    ): FigmaCommand => {
      const ref = node.id + ':' + role;
      const command: FigmaCommand = {
        type: 'add_layer',
        ref,
        layerType,
        name: role === 'node' ? node.name : `${node.name} ${role}`,
        group: owner,
        content,
        properties: { ...props },
        visible: true,
        locked: !!node.locked,
        blend: blendModes[node.blendMode ?? 'NORMAL'] ?? 'normal',
        effects: [],
        source: {
          fileKey: node.fileKey,
          nodeId: node.id,
          role,
          name: role === 'node' ? node.name : `${node.name} ${role}`,
        },
        ...(matte ? { matte } : {}),
      };
      if (layerType === 'shape' && !content.paths) {
        command.properties['anchor.x'] = -node.w / 2;
        command.properties['anchor.y'] = -node.h / 2;
      }
      commands.push(command);
      return command;
    };
    if (reasons.length) {
      issues.push({ id: node.id, name: node.name, reason: reasons.join(', ') });
      const raster = node.raster,
        b = raster?.bounds ?? bounds(node);
      // Raster is already transformed and contains opacity/effects. Place its
      // absolute render bounds back into the parent's coordinate system.
      const rm: Matrix = [1, 0, 0, 1, b.x, b.y],
        local = parent ? mul(inverse(parent.absoluteTransform), rm) : rm;
      const r = add('image', { w: b.w, h: b.h, fit: 'stretch' }, 'node', {
        'position.x': local[4] + (group ? 0 : offset.x),
        'position.y': local[5] + (group ? 0 : offset.y),
        'anchor.x': 0,
        'anchor.y': 0,
        'scale.x': Math.hypot(local[0], local[1]) * 100,
        'scale.y': ((local[0] * local[3] - local[1] * local[2]) / Math.hypot(local[0], local[1])) * 100,
        rotation: (Math.atan2(local[1], local[0]) * 180) / Math.PI,
        skew:
          (Math.atan((local[0] * local[2] + local[1] * local[3]) / (local[0] * local[3] - local[1] * local[2])) * 180) /
          Math.PI,
        opacity: 100,
      });
      r.properties['anchor.x'] = -b.w / 2;
      r.properties['anchor.y'] = -b.h / 2;
      r.raster = raster;
      return r.ref;
    }
    const fills = paints(node.fills),
      strokes = paints(node.strokes),
      r = node.radii ?? [0, 0, 0, 0];
    const shape = (fill?: Paint, stroke?: Paint) => ({
      shape: node.type === 'ELLIPSE' ? 'ellipse' : 'rect',
      w: node.w,
      h: node.h,
      color: fill?.type.startsWith('GRADIENT') ? '#FFFFFFFF' : color(fill),
      radius: r[0],
      independentCorners: new Set(r).size > 1,
      radiusTL: r[0],
      radiusTR: r[1],
      radiusBR: r[2],
      radiusBL: r[3],
      smoothing: (node.smoothing ?? 0) * 100,
      stroke: stroke ? (node.strokeWeight ?? 1) : 0,
      strokeColor: color(stroke),
    });
    const effects = (command: FigmaCommand) => {
      for (const e of node.effects.filter((e) => e.visible !== false))
        command.effects.push(
          e.type === 'DROP_SHADOW'
            ? {
                type: 'shadow',
                version: 2,
                params: {
                  color: hex(e.color!).slice(0, 7),
                  opacity: (e.color!.a ?? 1) * 100,
                  angle: (Math.atan2(e.offset!.x, -e.offset!.y) * 180) / Math.PI,
                  distance: Math.hypot(e.offset!.x, e.offset!.y),
                  softness: e.radius ?? 0,
                  shadowOnly: false,
                },
              }
            : { type: 'blur', version: 2, params: { amount: e.radius ?? 0, dimensions: 0, repeatEdgePixels: false } },
        );
    };
    if (containers.has(node.type)) {
      const c = add('group', {});
      let clip = matte;
      if (node.clipsContent) {
        const mask = add(
          'shape',
          shape({ type: 'SOLID', color: { r: 1, g: 1, b: 1 } }),
          'clip',
          {
            ...p,
            'position.x': 0,
            'position.y': 0,
            'scale.x': 100,
            'scale.y': 100,
            rotation: 0,
            skew: 0,
            opacity: 100,
          },
          c.ref,
        );
        clip = mask.ref;
      }
      // Figma mask applies to subsequent siblings. Walk bottom-to-top to resolve,
      // then emit top-to-bottom without changing the authored sibling order.
      const masks = new Map<FigmaNode, string | undefined>();
      let active = clip;
      for (const child of node.children) {
        if (child.isMask && child.visible !== false) active = child.id + ':node';
        else masks.set(child, active);
      }
      for (const child of [...node.children].reverse())
        build(child, node, c.ref, child.isMask ? undefined : masks.get(child));
      for (let i = strokes.length - 1; i >= 0; i--)
        add(
          'shape',
          shape(undefined, strokes[i]),
          `stroke-${i}`,
          {
            ...p,
            'position.x': 0,
            'position.y': 0,
            'scale.x': 100,
            'scale.y': 100,
            rotation: 0,
            skew: 0,
            opacity: 100,
          },
          c.ref,
        );
      for (let i = 0; i < fills.length; i++)
        add(
          'shape',
          shape(fills[i]),
          `fill-${i}`,
          {
            ...p,
            'position.x': 0,
            'position.y': 0,
            'scale.x': 100,
            'scale.y': 100,
            rotation: 0,
            skew: 0,
            opacity: 100,
          },
          c.ref,
        );
      return c.ref;
    }
    let c: FigmaCommand;
    if (node.type === 'TEXT') {
      const text = node.text!,
        run = text.runs[0]!,
        size = run.size,
        style = run.style;
      const weights: Array<[RegExp, number]> = [
        [/thin/i, 100],
        [/extra.?light/i, 200],
        [/light/i, 300],
        [/medium/i, 500],
        [/semi.?bold|demi/i, 600],
        [/extra.?bold/i, 800],
        [/black|heavy/i, 900],
        [/bold/i, 700],
      ];
      const align = (text.align ?? 'LEFT').toLowerCase(),
        ox = align === 'center' ? node.w / 2 : align === 'right' ? node.w : 0,
        oy = text.baseline ?? 0;
      c = add('text', {
        text: text.characters,
        font: run.font,
        size,
        weight: run.weight ?? weights.find(([re]) => re.test(style))?.[1] ?? 400,
        italic: /italic|oblique/i.test(style),
        color: color(run.fills[0]),
        align,
        boxWidth: text.autoResize === 'WIDTH_AND_HEIGHT' ? 0 : node.w,
        boxHeight: text.autoResize === 'NONE' ? node.h : 0,
        tracking: text.tracking ?? 0,
        leading: (text.leading ?? size * 1.2) / size,
        styles: text.runs
          .slice(1)
          .map((r, i) => ({ id: `style-${i}`, start: r.start, end: r.end, values: { color: color(r.fills[0]) } })),
      });
      c.properties['position.x']! += m[0] * ox + m[2] * oy;
      c.properties['position.y']! += m[1] * ox + m[3] * oy;
    } else if (vectors.has(node.type)) {
      if (fills.length > 1 || strokes.length > 1) reasons.push('multiple vector paints');
      c = add('shape', {
        paths: node
          .paths!.flatMap((path) => parseSvgPathData(path.data))
          .map((path) => ({
            ...path,
            name: node.name,
            fill: color(fills[0]).slice(0, 7),
            fillEnabled: fills.length > 0,
            fillOpacity: (fills[0]?.opacity ?? 1) * (fills[0]?.color?.a ?? 1) * 100,
            stroke: color(strokes[0]).slice(0, 7),
            strokeWidth: strokes.length ? (node.strokeWeight ?? 1) : 0,
            strokeOpacity: (strokes[0]?.opacity ?? 1) * (strokes[0]?.color?.a ?? 1) * 100,
          })),
      });
      if (fills.length > 1 || strokes.length > 1) throw new Error('Multiple vector paints require a fallback');
    } else {
      c = add('shape', shape(fills[0], strokes[0]));
      const ramp = fills[0] && gradient(fills[0], node);
      if (ramp) c.effects.push(ramp);
      if (fills.length > 1 || strokes.length > 1) {
        // Use an editable group so transforms and opacity apply to the paint stack once.
        c.layerType = 'group';
        c.content = {};
        c.properties = { ...p };
        const local = {
          ...p,
          'position.x': 0,
          'position.y': 0,
          'scale.x': 100,
          'scale.y': 100,
          rotation: 0,
          skew: 0,
          opacity: 100,
        };
        for (let i = 0; i < strokes.length; i++)
          add('shape', shape(undefined, strokes[i]), `stroke-${i}`, local, c.ref);
        for (let i = 0; i < fills.length; i++) add('shape', shape(fills[i]), `fill-${i}`, local, c.ref);
      }
    }
    effects(c);
    return c.ref;
  };
  let mask: string | undefined;
  const masks = new Map<FigmaNode, string | undefined>();
  for (const n of doc.nodes) {
    if (n.isMask && n.visible !== false) mask = n.id + ':node';
    else masks.set(n, mask);
  }
  for (const node of [...doc.nodes].reverse()) build(node, null, null, node.isMask ? undefined : masks.get(node));
  return { commands, issues, width: box.w, height: box.h, bounds: box };
}
