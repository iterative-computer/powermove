import type { FigmaDoc, FigmaNode } from '../doc';
const solid = (r = 0.95, g = 0.3, b = 0.12) => ({ type: 'SOLID', color: { r, g, b, a: 1 } });
const stops = [
  { position: 0, color: { r: 1, g: 0.2, b: 0, a: 1 } },
  { position: 0.4, color: { r: 0, g: 1, b: 0.4, a: 0.5 } },
  { position: 1, color: { r: 0, g: 0.3, b: 1, a: 1 } },
];
export function allFeatures(): FigmaDoc {
  const nodes: FigmaNode[] = [];
  const add = (name: string, patch: any = {}) => {
    const i = nodes.length,
      n: FigmaNode = {
        id: String(i + 1),
        fileKey: 'synthetic-all-features',
        name,
        type: 'RECTANGLE',
        w: 90,
        h: 60,
        absoluteTransform: [1, 0, 0, 1, 20 + (i % 7) * 130, 20 + Math.floor(i / 7) * 110],
        fills: [solid()],
        strokes: [],
        effects: [],
        children: [],
        ...patch,
      };
    nodes.push(n);
    return n;
  };
  for (const type of ['LINEAR', 'RADIAL', 'ANGULAR', 'DIAMOND'])
    add(`${type} gradient`, {
      fills: [{ type: 'GRADIENT_' + type, gradientTransform: [1, 0.2, -0.1, 1, 0, 0], gradientStops: stops }],
    });
  for (const scaleMode of ['FILL', 'FIT', 'CROP', 'TILE'])
    add(`Image ${scaleMode}`, {
      radii: [12, 12, 12, 12],
      fills: [
        {
          type: 'IMAGE',
          imageHash: 'checker',
          scaleMode,
          imageTransform: [1, 0, 0, 1, 0.1, 0],
          scalingFactor: 0.5,
        },
      ],
    });
  for (const strokeAlign of ['INSIDE', 'OUTSIDE', 'CENTER'])
    add(`${strokeAlign} stroke`, { strokeAlign, strokeWeight: 8, strokes: [solid(0.2, 0.6, 1)] });
  add('Dashed round stroke', {
    strokeWeight: 6,
    strokeAlign: 'CENTER',
    strokes: [solid()],
    fills: [],
    dashes: [9, 5],
    cap: 'ROUND',
    join: 'BEVEL',
  });
  add('Arrow caps', {
    type: 'VECTOR',
    paths: [{ data: 'M 0 40 L 90 10', windingRule: 'NONZERO' }],
    fills: [],
    strokes: [solid()],
    strokeWeight: 4,
    cap: 'ARROW_EQUILATERAL',
    join: 'MITER',
    miterLimit: 4,
  });
  add('Per-side strokes', {
    strokeWeights: [2, 5, 8, 11],
    strokeAlign: 'INSIDE',
    strokeWeight: 11,
    strokes: [solid(0.2, 0.6, 1)],
  });
  add('Compound holes', {
    type: 'VECTOR',
    paths: [{ data: 'M0 0H90V60H0Z M20 15H70V45H20Z', windingRule: 'EVENODD' }],
  });
  add('Open curved path', {
    type: 'VECTOR',
    fills: [],
    paths: [{ data: 'M0 50 C20 -20 70 80 90 10', windingRule: 'NONZERO' }],
    strokes: [solid()],
    strokeWeight: 5,
    cap: 'SQUARE',
  });
  add('Arc donut', { type: 'ELLIPSE', arc: { start: 0.2, end: 5.5, inner: 0.5 } });
  add('Star', {
    type: 'STAR',
    paths: [{ data: 'M45 0L55 21L90 21L65 36L74 60L45 45L16 60L25 36L0 21L35 21Z', windingRule: 'NONZERO' }],
  });
  const shadow = {
    radius: 5,
    spread: 3,
    offset: { x: 4, y: 4 },
    color: { r: 0, g: 0, b: 0, a: 0.7 },
    blendMode: 'NORMAL',
  };
  add('Inner shadow', { effects: [{ ...shadow, type: 'INNER_SHADOW' }] });
  add('Multiple spread shadows', {
    effects: [
      { ...shadow, type: 'DROP_SHADOW' },
      { ...shadow, type: 'DROP_SHADOW', offset: { x: -4, y: 0 }, color: { r: 0, g: 0.5, b: 1, a: 0.8 } },
    ],
  });
  add('Layer blur', { effects: [{ type: 'LAYER_BLUR', radius: 4 }] });
  add('Background blur', {
    fills: [{ type: 'SOLID', color: { r: 1, g: 1, b: 1, a: 0.3 } }],
    effects: [{ type: 'BACKGROUND_BLUR', radius: 6 }],
  });
  const text = {
    characters: 'Hello Figma',
    align: 'LEFT',
    autoResize: 'NONE',
    runs: [
      { start: 0, end: 6, font: 'Arial', style: 'Regular', size: 18, fills: [solid()] },
      {
        start: 6,
        end: 11,
        font: 'Arial',
        style: 'Bold Italic',
        weight: 700,
        size: 24,
        fills: [solid(0.1, 0.7, 1)],
        tracking: 1,
        leading: 30,
        decoration: 'UNDERLINE',
        textCase: 'UPPER',
      },
    ],
  };
  add('Styled text', { type: 'TEXT', text });
  add('Paragraph text', {
    type: 'TEXT',
    text: {
      ...text,
      characters: 'One two three\nfour five six',
      runs: [{ ...text.runs[0], end: 27 }],
      align: 'JUSTIFIED',
      verticalAlign: 'CENTER',
      paragraphSpacing: 5,
      paragraphIndent: 8,
      truncation: 'ENDING',
      maxLines: 2,
      advanced: true,
    },
  });
  const frame = add('Nested clipping', {
    type: 'FRAME',
    clipsContent: true,
    radii: [12, 12, 12, 12],
    fills: [],
  });
  const nested: any = {
    ...structuredClone(frame),
    id: frame.id + '-inner',
    name: 'Inner clip',
    w: 70,
    h: 50,
    absoluteTransform: [1, 0, 0, 1, frame.absoluteTransform[4] + 25, frame.absoluteTransform[5] + 15],
    children: [],
  };
  nested.children = [
    {
      ...structuredClone(nested),
      id: frame.id + '-child',
      type: 'RECTANGLE',
      name: 'Clipped green',
      clipsContent: false,
      w: 90,
      h: 60,
      fills: [solid(0.1, 0.8, 0.3)],
      children: [],
    },
  ];
  frame.children = [nested];
  const maskGroup = add('Luminance mask', { type: 'GROUP', fills: [] });
  maskGroup.children = [
    {
      ...structuredClone(maskGroup),
      id: maskGroup.id + '-mask',
      type: 'ELLIPSE',
      name: 'Luma matte',
      isMask: true,
      maskType: 'LUMINANCE',
      fills: [solid(0.5, 0.5, 0.5)],
      children: [],
    },
    {
      ...structuredClone(maskGroup),
      id: maskGroup.id + '-paint',
      type: 'RECTANGLE',
      name: 'Masked blue',
      fills: [solid(0.1, 0.5, 1)],
      children: [],
    },
  ];
  add('Boolean result', {
    type: 'BOOLEAN_OPERATION',
    paths: [{ data: 'M0 0H90V60H0Z M20 15V45H70V15Z', windingRule: 'NONZERO' }],
    children: [],
  });
  const instance = add('Instance overrides', { type: 'INSTANCE', fills: [], clipsContent: false });
  instance.children = [
    {
      ...structuredClone(instance),
      id: instance.id + '-child',
      type: 'RECTANGLE',
      name: 'Resolved override',
      fills: [solid(0.7, 0.2, 1)],
      children: [],
    },
  ];
  const transform = add('Skew flip smoothing', { radii: [12, 8, 16, 4], smoothing: 0.6 });
  transform.absoluteTransform[0] = -1;
  transform.absoluteTransform[2] = 0.3;
  transform.absoluteTransform[4] += 85;
  for (const blendMode of [
    'COLOR_BURN',
    'COLOR_DODGE',
    'HARD_LIGHT',
    'EXCLUSION',
    'HUE',
    'SATURATION',
    'COLOR',
    'LUMINOSITY',
    'LINEAR_BURN',
  ])
    add(`Blend ${blendMode}`, { blendMode });
  add('Multiple blended fills', { fills: [{ ...solid(0.1, 0.6, 1), blendMode: 'SCREEN' }, solid()] });
  add('Noise effect', {
    effects: [
      {
        type: 'NOISE',
        noiseSize: 3,
        density: 0.3,
        opacity: 0.4,
        color: { r: 0, g: 0, b: 0, a: 1 },
        noiseType: 'MONOTONE',
      },
    ],
  });
  add('Texture effect', { effects: [{ type: 'TEXTURE', noiseSize: 3, radius: 2, clipToShape: true }] });
  add('Glass effect', {
    effects: [
      {
        type: 'GLASS',
        lightAngle: 30,
        lightIntensity: 0.7,
        refraction: 0.3,
        depth: 4,
        dispersion: 0.2,
        radius: 3,
      },
    ],
  });
  return {
    format: 'powermove-figma',
    version: 1,
    source: 'plugin',
    nodes,
    images: {
      checker:
        'iVBORw0KGgoAAAANSUhEUgAAABAAAAAMCAYAAABr5z2BAAAAJ0lEQVR4nGMQqfjwHxl//foVBROSZxgGBpCqAV1+OBgw8LEw4AYAAI4+YX+uF8UrAAAAAElFTkSuQmCC',
    },
  };
}
/** Independently authored SVG references, not pixels from the Powermove renderer. */
export function referenceSVG(n: FigmaNode): string {
  let defs = '',
    body = `<rect width="90" height="60" fill="#f24d1f"/>`;
  if (n.name.includes('gradient')) {
    const type = n.fills[0]!.type;
    defs = `<linearGradient id="g"><stop stop-color="#ff3300"/><stop offset=".4" stop-color="#00ff66" stop-opacity=".5"/><stop offset="1" stop-color="#004dff"/></linearGradient>`;
    if (type === 'GRADIENT_RADIAL') defs = defs.replaceAll('linearGradient', 'radialGradient');
    body = '<rect width="90" height="60" fill="url(#g)"/>';
  }
  if (n.type === 'TEXT')
    body = '<text x="0" y="20" font-family="Arial" font-size="18" fill="#f24d1f">Hello Figma</text>';
  if (n.paths?.length)
    body = n.paths
      .map(
        (p) =>
          `<path d="${p.data}" fill="${n.fills.length ? '#f24d1f' : 'none'}" fill-rule="${p.windingRule === 'EVENODD' ? 'evenodd' : 'nonzero'}" stroke="${n.strokes.length ? '#f24d1f' : 'none'}" stroke-width="${n.strokeWeight ?? 0}"/>`,
      )
      .join('');
  if (n.type === 'ELLIPSE') body = '<ellipse cx="45" cy="30" rx="45" ry="30" fill="#f24d1f"/>';
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${n.w}" height="${n.h}">${defs}${body}</svg>`;
}
