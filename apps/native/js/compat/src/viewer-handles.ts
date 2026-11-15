/** Registration-time checks mirror pm-fx-defs::handles. UI declarations stay
 * on the definition; render snapshots do not need them. */
export function validateViewerHandles(def: any): void {
  for (const p of Array.isArray(def?.params) ? def.params : [])
    if (p.type === "gradientStops") validateGradientStops(p.def);
  const handles = def?.viewerHandles;
  if (handles === undefined) return;
  if (!Array.isArray(handles) || handles.length > 32)
    throw new Error("viewerHandles: expected at most 32 controls");
  const param = (key: unknown, type = "number") => {
    if (
      typeof key !== "string" ||
      !def.params?.some(
        (p: any) => p.k === key && (p.type ?? "number") === type,
      )
    )
      throw new Error(
        `viewerHandles: ${String(key)} must bind a ${type} parameter`,
      );
  };
  const point = (p: any) => {
    if (!p || Object.keys(p).some((k) => !["x", "y"].includes(k)))
      throw new Error("viewerHandles: invalid point");
    param(p.x);
    param(p.y);
    if (p.x === p.y) throw new Error("viewerHandles: point axes must differ");
  };
  for (const h of handles) {
    if (!h || typeof h !== "object")
      throw new Error("viewerHandles: invalid control");
    let fields = ["kind"];
    switch (h.kind) {
      case "point":
        fields.push("point");
        point(h.point);
        break;
      case "line":
        fields.push("start", "end", "stops", "radial");
        point(h.start);
        point(h.end);
        if (h.stops != null) param(h.stops, "gradientStops");
        if (h.radial != null) param(h.radial);
        break;
      case "rect":
        fields.push("start", "end");
        point(h.start);
        point(h.end);
        break;
      case "angle":
        fields.push("center", "angle");
        point(h.center);
        param(h.angle);
        break;
      case "radius":
        fields.push("center", "radius");
        point(h.center);
        param(h.radius);
        break;
      default:
        throw new Error("viewerHandles: unknown kind");
    }
    if (Object.keys(h).some((k) => !fields.includes(k)))
      throw new Error("viewerHandles: unknown field");
  }
}

export const MAX_GRADIENT_STOPS = 16;
export function gradientDeclarations(key: string): string {
  let out = `uniform float u_${key}Count;\n`;
  for (let i = 0; i < 16; i++)
    out += `uniform float u_${key}P${i}; uniform vec4 u_${key}C${i};\n`;
  out += `vec4 pm_${key}(float t){vec4 c=u_${key}C0; float p=u_${key}P0;\n`;
  for (let i = 1; i < 16; i++)
    out += `if(u_${key}Count>${i}.5){if(t<=u_${key}P${i}) return mix(c,u_${key}C${i},clamp((t-p)/max(u_${key}P${i}-p,1e-8),0.,1.));c=u_${key}C${i};p=u_${key}P${i};}\n`;
  return out + "return c;}\n";
}
export function validateGradientStops(stops: any): void {
  if (
    !Array.isArray(stops) ||
    stops.length < 2 ||
    stops.length > 16 ||
    stops.some(
      (s) =>
        !s ||
        !Number.isFinite(s.position) ||
        s.position < 0 ||
        s.position > 1 ||
        !Array.isArray(s.color) ||
        s.color.length !== 4 ||
        s.color.some((v: any) => !Number.isFinite(v) || v < 0 || v > 1),
    )
  )
    throw new Error(
      "gradientStops: expected 2..16 finite positions and RGBA channels in 0..1",
    );
}
