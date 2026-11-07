/** Content channels use the same persisted {v, kf, expr} model as transforms. */
export function isProperty(value: any): boolean {
  return (
    !!value &&
    typeof value === "object" &&
    Array.isArray(value.kf) &&
    "v" in value
  );
}

export const contentFields: Record<string, string[]> = {
  group: ["isolate"],
  text: [
    "text",
    "font",
    "weight",
    "size",
    "tracking",
    "leading",
    "align",
    "color",
    "italic",
    "boxWidth",
    "boxHeight",
    "paragraphSpacing",
    "paragraphIndent",
    "verticalAlign",
    "truncation",
    "maxLines",
    "textStroke",
    "textStrokeColor",
  ],
  solid: ["color", "w", "h", "radius"],
  shape: [
    "color",
    "shape",
    "w",
    "h",
    "radius",
    "radiusTL",
    "radiusTR",
    "radiusBR",
    "radiusBL",
    "smoothing",
    "stroke",
    "strokeColor",
    "points",
  ],
  image: ["fit", "w", "h"],
  video: [
    "fit",
    "w",
    "h",
    "trim",
    "speed",
    "timeRemap",
    "sourceTime",
    "audioGain",
  ],
  audio: ["trim", "gain", "fadeIn", "fadeOut"],
  shader: ["w", "h"],
  extension: ["w", "h", "trim", "speed", "timeRemap", "sourceTime"],
  precomp: ["w", "h", "trim", "sourceTime", "timeRemap", "audioGain"],
};

/** Nested drawing channels have explicit roots; arbitrary content/asset IDs are not animatable. */
export const drawingRoots = [
  "fillGradient",
  "strokeGradient",
  "strokeStyle",
  "arc",
  "imageMapping",
  "runs",
];
export function contentSlot(
  layer: any,
  key: string,
): { owner: any; key: string } | null {
  if (!drawingRoots.some((root) => key.startsWith(root + ".")))
    return layer.d && { owner: layer.d, key };
  const parts = key.split(".");
  let owner = layer.d;
  for (const part of parts.slice(0, -1)) {
    if (!owner || !Object.hasOwn(owner, part)) return null;
    owner = owner[part];
  }
  return owner && Object.hasOwn(owner, parts.at(-1)!)
    ? { owner, key: parts.at(-1)! }
    : null;
}
export function drawingProperties(layer: any): any[] {
  const output: any[] = [];
  const visit = (value: any, path: string) => {
    if (isProperty(value))
      output.push({
        key: "c." + path,
        prop: value,
        label: path,
        group: "Content",
      });
    else if (value && typeof value === "object")
      for (const key of Object.keys(value)) visit(value[key], path + "." + key);
  };
  for (const key of drawingRoots) visit(layer.d?.[key], key);
  return output;
}
export function canAnimateContent(layer: any, key: string): boolean {
  if (/^runs\.\d+\.(start|end)$/.test(key)) return false;
  if (drawingRoots.some((root) => key.startsWith(root + "."))) {
    const slot = contentSlot(layer, key);
    const v = slot?.owner[slot.key];
    return isProperty(v) || ["number", "string", "boolean"].includes(typeof v);
  }
  return (
    !!contentFields[layer.type]?.includes(key) ||
    (layer.type === "text" && /^fontAxis\.[\x20-\x7e]{4}$/.test(key))
  );
}

export function resolveContent(PM: any, layer: any, time: number): any {
  const content = { ...layer.d };
  // Most content includes unanimated geometry, text styling or media metadata.
  // Visit the small declared channel set without allocating entry pairs for
  // every field on every layer and every motion-blur sample.
  for (const key of contentFields[layer.type] || []) {
    const value = content[key];
    if (isProperty(value))
      content[key] = PM.evP ? PM.evP(layer, value, time, `c.${key}`) : value.v;
  }
  if (layer.type === "text")
    for (const key in content) {
      if (!/^fontAxis\.[\x20-\x7e]{4}$/.test(key)) continue;
      const value = content[key];
      if (isProperty(value))
        content[key] = PM.evP
          ? PM.evP(layer, value, time, `c.${key}`)
          : value.v;
    }
  const resolve = (value: any, path: string): any =>
    isProperty(value)
      ? PM.evP
        ? PM.evP(layer, value, time, "c." + path)
        : value.v
      : Array.isArray(value)
        ? value.map((item, i) => resolve(item, path + "." + i))
        : value && typeof value === "object"
          ? Object.fromEntries(
              Object.entries(value).map(([k, v]) => [
                k,
                resolve(v, path + "." + k),
              ]),
            )
          : value;
  for (const key of drawingRoots)
    if (content[key]) content[key] = resolve(content[key], key);
  return content;
}

export function contentLabel(key: string): string {
  const names: Record<string, string> = {
    w: "Width",
    h: "Height",
    text: "Text",
    font: "Font",
    weight: "Weight",
    size: "Size",
    tracking: "Tracking",
    leading: "Leading",
    align: "Align",
    color: "Color",
    italic: "Italic",
    boxWidth: "Text Box Width",
    boxHeight: "Text Box Height",
    shape: "Shape",
    radius: "Corner radius",
    radiusTL: "Top left radius",
    radiusTR: "Top right radius",
    radiusBR: "Bottom right radius",
    radiusBL: "Bottom left radius",
    smoothing: "Corner smoothing",
    stroke: "Stroke",
    strokeColor: "Stroke color",
    points: "Points",
    fit: "Fit",
    trim: "Trim start",
    speed: "Speed",
    gain: "Gain",
    audioGain: "Audio gain",
    fadeIn: "Fade in",
    fadeOut: "Fade out",
  };
  return names[key] ?? key;
}

export function evaluatedValue(
  PM: any,
  layer: any,
  value: any,
  time: number,
  path: string,
): any {
  return isProperty(value) ? PM.evP(layer, value, time, path) : value;
}
