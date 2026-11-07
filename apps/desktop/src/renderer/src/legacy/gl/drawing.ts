import { inverse, type Matrix } from "../../../../shared/figma/doc";
import { roundedRectPath } from "./corner-geometry";

/** Shared shape paint implementation. Source remains geometry/paint data;
 * these canvases are transient renderer caches, never imported PNG assets. */
export function shapeOutline(d: any): Path2D {
  if (d.shape === "ellipse") {
    const path = new Path2D(),
      x = d.w / 2,
      y = d.h / 2;
    const a = d.arc,
      start = a?.start ?? 0,
      end = a?.end ?? Math.PI * 2;
    const full = Math.abs(end - start) >= Math.PI * 2 - 1e-8,
      inner = a?.inner ?? 0;
    path.ellipse(x, y, x, y, 0, start, end);
    if (inner > 0) {
      if (full)
        path.moveTo(
          x + x * inner * Math.cos(end),
          y + y * inner * Math.sin(end),
        );
      else
        path.lineTo(
          x + x * inner * Math.cos(end),
          y + y * inner * Math.sin(end),
        );
      path.ellipse(x, y, x * inner, y * inner, 0, end, start, true);
    } else if (!full) path.lineTo(x, y);
    path.closePath();
    return path;
  }
  if (d.shape === "line") {
    const p = new Path2D();
    p.moveTo(0, 0);
    p.lineTo(d.w, d.h);
    return p;
  }
  if (d.shape === "polygon" || d.shape === "star") {
    const p = new Path2D(),
      count = Math.max(3, d.points || 5) * (d.shape === "star" ? 2 : 1);
    for (let i = 0; i < count; i++) {
      const angle = -Math.PI / 2 + (i * Math.PI * 2) / count,
        r = d.shape === "star" && i % 2 ? 0.46 : 1;
      const x = d.w / 2 + ((Math.cos(angle) * d.w) / 2) * r,
        y = d.h / 2 + ((Math.sin(angle) * d.h) / 2) * r;
      if (i) p.lineTo(x, y);
      else p.moveTo(x, y);
    }
    p.closePath();
    return p;
  }
  return new Path2D(roundedRectPath(d));
}

export function gradientParameter(type: string, x: number, y: number): number {
  if (type === "radial") return Math.hypot(x - 0.5, y - 0.5) * 2;
  if (type === "diamond") return (Math.abs(x - 0.5) + Math.abs(y - 0.5)) * 2;
  if (type === "angular")
    return (((Math.atan2(y - 0.5, x - 0.5) / (Math.PI * 2)) % 1) + 1) % 1;
  return x;
}
/** Alpha interpolation is premultiplied, matching Canvas/CSS gradients. */
export function gradientSampler(
  stops: Array<{ position: number; color: string }>,
): (t: number) => number[] {
  const colors = [...stops]
    .sort((a, b) => a.position - b.position)
    .map((s) => {
      const h = s.color.slice(1);
      return {
        position: s.position,
        rgba: [0, 2, 4, 6].map((v, i) =>
          i === 3 && h.length === 6 ? 255 : parseInt(h.slice(v, v + 2), 16),
        ),
      };
    });
  return (t) => {
    if (!colors.length) return [0, 0, 0, 0];
    let lo = 0,
      hi = colors.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (colors[mid]!.position <= t) lo = mid + 1;
      else hi = mid;
    }
    const index = Math.max(0, lo - 1),
      a = colors[index]!,
      b = colors[Math.min(index + 1, colors.length - 1)]!,
      c = a.rgba,
      d = b.rgba;
    const f = Math.max(
        0,
        Math.min(1, (t - a.position) / (b.position - a.position || 1)),
      ),
      alpha = c[3]! * (1 - f) + d[3]! * f;
    return [0, 1, 2]
      .map((k) =>
        alpha ? (c[k]! * c[3]! * (1 - f) + d[k]! * d[3]! * f) / alpha : 0,
      )
      .concat(alpha);
  };
}
export function sampleGradient(
  stops: Array<{ position: number; color: string }>,
  t: number,
): number[] {
  return gradientSampler(stops)(t);
}

export function gradientStyle(
  ctx: CanvasRenderingContext2D,
  paint: any,
  w: number,
  h: number,
  offsetX = 0,
  offsetY = 0,
): CanvasGradient | CanvasPattern | string {
  if (!paint?.stops?.length) return "transparent";
  if (paint.type === "diamond") {
    // Canvas lacks diamond gradients. Generate a resolution-independent paint
    // cache at the current display density; the editable source is still stops.
    const at = gradientSampler(paint.stops);
    const m = paint.transform as Matrix,
      current = ctx.getTransform();
    const density = Math.max(
      0.25,
      Math.hypot(current.a, current.b),
      Math.hypot(current.c, current.d),
    );
    const cv = document.createElement("canvas");
    cv.width = Math.max(1, Math.min(8192, Math.ceil(w * density)));
    cv.height = Math.max(1, Math.min(8192, Math.ceil(h * density)));
    const c = cv.getContext("2d")!,
      image = c.createImageData(cv.width, cv.height);
    for (let y = 0; y < cv.height; y++)
      for (let x = 0; x < cv.width; x++) {
        const u = (x + 0.5) / cv.width,
          v = (y + 0.5) / cv.height;
        const rgba = at(
          gradientParameter(
            "diamond",
            m[0] * u + m[2] * v + m[4],
            m[1] * u + m[3] * v + m[5],
          ),
        );
        rgba[3]! *= paint.opacity ?? 1;
        image.data.set(rgba, (y * cv.width + x) * 4);
      }
    c.putImageData(image, 0, 0);
    const pattern = ctx.createPattern(cv, "no-repeat")!;
    pattern.setTransform(
      new DOMMatrix([w / cv.width, 0, 0, h / cv.height, offsetX, offsetY]),
    );
    return pattern;
  }
  const transform = paint.transform ?? [1, 0, 0, 1, 0, 0],
    inv = inverse(transform),
    current = ctx.getTransform();
  const density = Math.max(
    0.25,
    Math.hypot(current.a, current.b),
    Math.hypot(current.c, current.d),
  );
  const cv = document.createElement("canvas");
  cv.width = Math.max(1, Math.min(8192, Math.ceil(w * density)));
  cv.height = Math.max(1, Math.min(8192, Math.ceil(h * density)));
  const c = cv.getContext("2d")!;
  c.scale(cv.width, cv.height);
  c.transform(...inv);
  const g =
    paint.type === "radial"
      ? c.createRadialGradient(0.5, 0.5, 0, 0.5, 0.5, 0.5)
      : paint.type === "angular"
        ? c.createConicGradient(0, 0.5, 0.5)
        : c.createLinearGradient(0, 0.5, 1, 0.5);
  for (const s of [...paint.stops].sort(
    (a: any, b: any) => a.position - b.position,
  )) {
    const rgba = sampleGradient([s], 0);
    g.addColorStop(
      Math.max(0, Math.min(1, s.position)),
      `rgba(${rgba[0]},${rgba[1]},${rgba[2]},${(rgba[3]! / 255) * (paint.opacity ?? 1)})`,
    );
  }
  // Gradient coordinates are evaluated at paint time. Keep the affine CTM in
  // place when painting, rather than restoring it after creating a gradient.
  const corners = [
    [0, 0],
    [0, 1],
    [1, 0],
    [1, 1],
  ].map(([x, y]) => [
    transform[0] * x! + transform[2] * y! + transform[4],
    transform[1] * x! + transform[3] * y! + transform[5],
  ]);
  const x = Math.min(...corners.map((p) => p[0]!)),
    y = Math.min(...corners.map((p) => p[1]!)),
    right = Math.max(...corners.map((p) => p[0]!)),
    bottom = Math.max(...corners.map((p) => p[1]!));
  c.fillStyle = g;
  c.fillRect(x - 1, y - 1, right - x + 2, bottom - y + 2);
  const pattern = ctx.createPattern(cv, "no-repeat")!;
  pattern.setTransform(
    new DOMMatrix([w / cv.width, 0, 0, h / cv.height, offsetX, offsetY]),
  );
  return pattern;
}

export function paintShape(
  ctx: CanvasRenderingContext2D,
  path: Path2D,
  d: any,
  extent: { x: number; y: number; w: number; h: number },
  ends: Array<{ x: number; y: number; angle: number; cap?: string }> = [],
) {
  const rule: CanvasFillRule = d.fillRule === "evenodd" ? "evenodd" : "nonzero";
  ctx.fillStyle = d.fillGradient
    ? gradientStyle(ctx, d.fillGradient, d.w, d.h)
    : (d.color ?? "transparent");
  ctx.fill(path, rule);
  const style = d.strokeStyle ?? {},
    width = Math.max(0, Number(d.stroke) || 0);
  if (!width) return;
  const stroke = document.createElement("canvas");
  stroke.width = ctx.canvas.width;
  stroke.height = ctx.canvas.height;
  const c = stroke.getContext("2d")!;
  c.setTransform(ctx.getTransform());
  c.strokeStyle = "#fff";
  c.lineWidth =
    width * (style.align === "inside" || style.align === "outside" ? 2 : 1);
  c.lineCap = ["round", "square"].includes(String(style.cap).toLowerCase())
    ? (String(style.cap).toLowerCase() as CanvasLineCap)
    : "butt";
  c.lineJoin = style.join ?? "miter";
  c.miterLimit = style.miterLimit ?? 4;
  c.setLineDash(style.dashes ?? []);
  c.lineDashOffset = style.dashOffset ?? 0;
  if (style.weights && d.shape === "rect") {
    // Diagonal joins allocate the corner between adjacent side widths. Clip
    // each side of the same rounded outline, preserving smoothing and radii.
    const [t, r, b, l] = style.weights,
      W = d.w,
      H = d.h;
    const polygons = [
      [
        [-width, -width],
        [W + width, -width],
        [W - r, t],
        [l, t],
      ],
      [
        [W + width, -width],
        [W + width, H + width],
        [W - r, H - b],
        [W - r, t],
      ],
      [
        [W + width, H + width],
        [-width, H + width],
        [l, H - b],
        [W - r, H - b],
      ],
      [
        [-width, H + width],
        [-width, -width],
        [l, t],
        [l, H - b],
      ],
    ];
    for (let i = 0; i < 4; i++) {
      c.save();
      const clip = new Path2D();
      polygons[i]!.forEach(([x, y], j) =>
        j ? clip.lineTo(x!, y!) : clip.moveTo(x!, y!),
      );
      clip.closePath();
      c.clip(clip);
      c.lineWidth = style.weights[i] * (style.align === "center" ? 1 : 2);
      c.stroke(path);
      c.restore();
    }
  } else c.stroke(path);
  for (const end of ends)
    drawCap(c, end.cap ?? style.cap, end.x, end.y, end.angle, width);
  if (style.align === "inside" || style.align === "outside") {
    c.globalCompositeOperation =
      style.align === "inside" ? "destination-in" : "destination-out";
    c.fillStyle = "#fff";
    c.fill(path, rule);
  }
  c.globalCompositeOperation = "source-in";
  c.fillStyle = d.strokeGradient
    ? gradientStyle(c, d.strokeGradient, d.w, d.h)
    : (d.strokeColor ?? "#fff");
  c.fillRect(extent.x, extent.y, extent.w, extent.h);
  ctx.save();
  ctx.resetTransform();
  ctx.drawImage(stroke, 0, 0);
  ctx.restore();
  stroke.width = stroke.height = 0;
}
function drawCap(
  c: CanvasRenderingContext2D,
  cap: string,
  x: number,
  y: number,
  angle: number,
  w: number,
) {
  if (!cap || ["NONE", "BUTT"].includes(cap.toUpperCase())) return;
  c.save();
  c.translate(x, y);
  c.rotate(angle);
  c.fillStyle = c.strokeStyle;
  c.setLineDash([]);
  c.lineWidth = w;
  c.beginPath();
  const size = w * 3;
  if (cap === "ROUND") {
    c.arc(0, 0, w / 2, 0, Math.PI * 2);
    c.fill();
  } else if (cap === "SQUARE") {
    c.fillRect(0, -w / 2, w / 2, w);
  } else if (cap === "ARROW_LINES") {
    c.moveTo(-size, -size / 2);
    c.lineTo(0, 0);
    c.lineTo(-size, size / 2);
    c.stroke();
  } else if (cap.includes("CIRCLE")) {
    c.arc(0, 0, w * 1.5, 0, Math.PI * 2);
    c.fill();
  } else if (cap.includes("DIAMOND")) {
    c.moveTo(size / 2, 0);
    c.lineTo(-size / 2, -size / 2);
    c.lineTo(-size * 1.5, 0);
    c.lineTo(-size / 2, size / 2);
    c.closePath();
    c.fill();
  } else {
    c.moveTo(0, 0);
    c.lineTo(-size, -size / 2);
    c.lineTo(-size, size / 2);
    c.closePath();
    c.fill();
  }
  c.restore();
}

export function rasterImageFill(
  image: CanvasImageSource,
  sw: number,
  sh: number,
  w: number,
  h: number,
  mapping: any,
  density: number,
): HTMLCanvasElement {
  const cv = document.createElement("canvas");
  cv.width = Math.max(1, Math.min(8192, Math.ceil(w * density)));
  cv.height = Math.max(1, Math.min(8192, Math.ceil(h * density)));
  const c = cv.getContext("2d")!;
  c.scale(cv.width / w, cv.height / h);
  if (mapping.mode === "crop" && mapping.transform) {
    c.scale(w, h);
    c.transform(...inverse(mapping.transform));
    c.drawImage(image, 0, 0, 1, 1);
  } else if (mapping.mode === "tile") {
    const p = c.createPattern(image, "repeat")!,
      scale = mapping.scale ?? 1;
    const angle = ((mapping.rotation ?? 0) * Math.PI) / 180,
      cs = Math.cos(angle) * scale,
      sn = Math.sin(angle) * scale;
    p.setTransform(new DOMMatrix([cs, sn, -sn, cs, 0, 0]));
    c.fillStyle = p;
    c.fillRect(0, 0, w, h);
  } else {
    const scale =
      mapping.mode === "fit"
        ? Math.min(w / sw, h / sh)
        : Math.max(w / sw, h / sh);
    c.drawImage(
      image,
      (w - sw * scale) / 2,
      (h - sh * scale) / 2,
      sw * scale,
      sh * scale,
    );
  }
  return cv;
}

/** Canvas implements every imported paint blend except linear burn. Keep that
 * final mode in the editable text rasterizer using the compositor's alpha math. */
export function linearBurnPixel(
  dst: ArrayLike<number>,
  src: ArrayLike<number>,
): number[] {
  const sa = src[3]! / 255,
    da = dst[3]! / 255,
    a = sa + da * (1 - sa);
  return [0, 1, 2]
    .map((k) =>
      a
        ? ((1 - sa) * dst[k]! * da +
            (1 - da) * src[k]! * sa +
            sa * da * Math.max(0, dst[k]! + src[k]! - 255)) /
          a
        : 0,
    )
    .concat(a * 255);
}
export function compositePaint(
  ctx: CanvasRenderingContext2D,
  mode: string | undefined,
  draw: (target: CanvasRenderingContext2D) => void,
): void {
  if (mode !== "linearburn") {
    ctx.save();
    ctx.globalCompositeOperation = (
      mode === "add"
        ? "lighter"
        : (mode ?? "source-over")
            .replace("normal", "source-over")
            .replace("softlight", "soft-light")
            .replace("hardlight", "hard-light")
            .replace("colorburn", "color-burn")
            .replace("colordodge", "color-dodge")
    ) as GlobalCompositeOperation;
    draw(ctx);
    ctx.restore();
    return;
  }
  const cv = document.createElement("canvas");
  cv.width = ctx.canvas.width;
  cv.height = ctx.canvas.height;
  const c = cv.getContext("2d")!;
  c.setTransform(ctx.getTransform());
  for (const key of [
    "font",
    "textAlign",
    "textBaseline",
    "direction",
    "letterSpacing",
    "wordSpacing",
    "fontKerning",
    "fontStretch",
    "fontVariantCaps",
    "globalAlpha",
    "filter",
    "fillStyle",
  ] as const)
    if (key in ctx) (c as any)[key] = (ctx as any)[key];
  draw(c);
  const source = c.getImageData(0, 0, cv.width, cv.height),
    result = ctx.getImageData(0, 0, cv.width, cv.height);
  for (let i = 0; i < source.data.length; i += 4) {
    if (!source.data[i + 3]) continue;
    result.data.set(
      linearBurnPixel(
        result.data.subarray(i, i + 4),
        source.data.subarray(i, i + 4),
      ),
      i,
    );
  }
  c.resetTransform();
  c.putImageData(result, 0, 0);
  ctx.save();
  ctx.resetTransform();
  ctx.globalAlpha = 1;
  ctx.filter = "none";
  ctx.globalCompositeOperation = "copy";
  ctx.drawImage(cv, 0, 0);
  ctx.restore();
  cv.width = cv.height = 0;
}
