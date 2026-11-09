import type {
  FigmaDoc,
  FigmaNode,
  Matrix,
  Paint,
} from "../../src/shared/figma/doc";

/** Independent fixture oracle: browser SVG, CSS typography and Canvas blends.
 * Never calls the converter, PM renderer or PM geometry/text helpers.
 * Optical fallbacks are checked against their supplied PNG, not a made-up glass
 * model. Noise uses distribution checks; its random pixels are not comparable.
 */
const rgba = (c: any, opacity = 1) =>
  "rgba(" +
  [c.r * 255, c.g * 255, c.b * 255, (c.a ?? 1) * opacity].join(",") +
  ")";
const matrix = (m: number[]) => "matrix(" + m.join(" ") + ")";
const inverse = (m: Matrix): Matrix => {
  const d = m[0] * m[3] - m[1] * m[2];
  return [
    m[3] / d,
    -m[1] / d,
    -m[2] / d,
    m[0] / d,
    (m[2] * m[5] - m[3] * m[4]) / d,
    (m[1] * m[4] - m[0] * m[5]) / d,
  ];
};
const multiply = (a: Matrix, b: Matrix): Matrix => [
  a[0] * b[0] + a[2] * b[1],
  a[1] * b[0] + a[3] * b[1],
  a[0] * b[2] + a[2] * b[3],
  a[1] * b[2] + a[3] * b[3],
  a[0] * b[4] + a[2] * b[5] + a[4],
  a[1] * b[4] + a[3] * b[5] + a[5],
];
const escape = (s: string) =>
  s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll('"', "&quot;");
const image = async (url: string) => {
  const i = new Image();
  i.src = url;
  await i.decode();
  return i;
};
const blend = (mode?: string) =>
  (
    ({
      NORMAL: "source-over",
      PASS_THROUGH: "source-over",
      COLOR_BURN: "color-burn",
      COLOR_DODGE: "color-dodge",
      HARD_LIGHT: "hard-light",
      SOFT_LIGHT: "soft-light",
      LINEAR_DODGE: "lighter",
    }) as Record<string, string>
  )[mode ?? "NORMAL"] ?? (mode ?? "NORMAL").toLowerCase();

export function referenceOffset(nodes: FigmaNode[], w: number, h: number) {
  const points = nodes.flatMap((n) =>
    [
      [0, 0],
      [n.w, 0],
      [0, n.h],
      [n.w, n.h],
    ].map(([x, y]) => {
      const m = n.absoluteTransform;
      return [m[0] * x! + m[2] * y! + m[4], m[1] * x! + m[3] * y! + m[5]];
    }),
  );
  const x = Math.min(...points.map((p) => p[0]!)),
    y = Math.min(...points.map((p) => p[1]!));
  return {
    x: (w - Math.max(...points.map((p) => p[0]!)) + x) / 2 - x,
    y: (h - Math.max(...points.map((p) => p[1]!)) + y) / 2 - y,
  };
}

function outline(n: FigmaNode): string {
  if (n.paths?.length)
    return n.paths
      .map(
        (p) =>
          '<path d="' +
          p.data +
          '" fill-rule="' +
          (p.windingRule === "EVENODD" ? "evenodd" : "nonzero") +
          '"/>',
      )
      .join("");
  if (n.type === "ELLIPSE" && n.arc) {
    const a = n.arc,
      rx = n.w / 2,
      ry = n.h / 2,
      point = (angle: number, r = 1) =>
        [rx + rx * r * Math.cos(angle), ry + ry * r * Math.sin(angle)].join(
          " ",
        );
    const large = a.end - a.start > Math.PI ? 1 : 0;
    return (
      '<path d="M' +
      point(a.start) +
      " A" +
      rx +
      " " +
      ry +
      " 0 " +
      large +
      " 1 " +
      point(a.end) +
      (a.inner
        ? " L" +
          point(a.end, a.inner) +
          " A" +
          rx * a.inner +
          " " +
          ry * a.inner +
          " 0 " +
          large +
          " 0 " +
          point(a.start, a.inner)
        : " L" + rx + " " + ry) +
      ' Z"/>'
    );
  }
  if (n.type === "ELLIPSE")
    return (
      '<ellipse cx="' +
      n.w / 2 +
      '" cy="' +
      n.h / 2 +
      '" rx="' +
      n.w / 2 +
      '" ry="' +
      n.h / 2 +
      '"/>'
    );
  const [tl, tr, br, bl] = n.radii ?? [0, 0, 0, 0],
    w = n.w,
    h = n.h;
  if (tl === tr && tr === br && br === bl)
    return '<rect width="' + w + '" height="' + h + '" rx="' + tl + '"/>';
  // A circular-corner reference bounds the smoothing difference separately.
  return (
    '<path d="M' +
    tl +
    " 0H" +
    (w - tr) +
    "A" +
    tr +
    " " +
    tr +
    " 0 0 1 " +
    w +
    " " +
    tr +
    "V" +
    (h - br) +
    "A" +
    br +
    " " +
    br +
    " 0 0 1 " +
    (w - br) +
    " " +
    h +
    "H" +
    bl +
    "A" +
    bl +
    " " +
    bl +
    " 0 0 1 0 " +
    (h - bl) +
    "V" +
    tl +
    "A" +
    tl +
    " " +
    tl +
    " 0 0 1 " +
    tl +
    ' 0Z"/>'
  );
}

async function polarGradient(n: FigmaNode, p: Paint) {
  const cv = document.createElement("canvas");
  cv.width = n.w;
  cv.height = n.h;
  const c = cv.getContext("2d")!,
    m = p.gradientTransform!;
  if (p.type === "GRADIENT_ANGULAR") {
    c.scale(n.w, n.h);
    c.transform(...inverse(m));
    const g = c.createConicGradient(0, 0.5, 0.5);
    for (const s of p.gradientStops!)
      g.addColorStop(s.position, rgba(s.color, p.opacity));
    c.fillStyle = g;
    c.fillRect(-5, -5, 10, 10);
  } else {
    const d = c.createImageData(cv.width, cv.height),
      stops = p.gradientStops!;
    for (let y = 0; y < cv.height; y++)
      for (let x = 0; x < cv.width; x++) {
        const u = (x + 0.5) / n.w,
          v = (y + 0.5) / n.h,
          t = Math.min(
            1,
            Math.max(
              0,
              2 *
                (Math.abs(m[0] * u + m[2] * v + m[4] - 0.5) +
                  Math.abs(m[1] * u + m[3] * v + m[5] - 0.5)),
            ),
          );
        let k = 1;
        while (k < stops.length - 1 && stops[k]!.position < t) k++;
        const a = stops[k - 1]!,
          b = stops[k]!,
          f = (t - a.position) / (b.position - a.position),
          alpha = (a.color.a ?? 1) * (1 - f) + (b.color.a ?? 1) * f;
        d.data.set(
          [
            ...(["r", "g", "b"] as const).map(
              (key) => 255 * (a.color[key] * (1 - f) + b.color[key] * f),
            ),
            255 * alpha * (p.opacity ?? 1),
          ],
          (y * cv.width + x) * 4,
        );
      }
    c.putImageData(d, 0, 0);
  }
  return cv.toDataURL();
}

export async function gridReference(doc: FigmaDoc, w = 930, h = 680) {
  let serial = 0;
  const defs: string[] = [];
  const id = () => "ref" + ++serial;
  async function paint(n: FigmaNode, p: Paint) {
    if (p.type === "SOLID") return rgba(p.color!, p.opacity);
    const key = id();
    if (p.type === "GRADIENT_LINEAR" || p.type === "GRADIENT_RADIAL") {
      const tag =
        p.type === "GRADIENT_LINEAR" ? "linearGradient" : "radialGradient";
      defs.push(
        "<" +
          tag +
          ' id="' +
          key +
          '" gradientTransform="' +
          matrix(inverse(p.gradientTransform!)) +
          '">' +
          p
            .gradientStops!.map(
              (s) =>
                '<stop offset="' +
                s.position +
                '" stop-color="' +
                rgba(s.color, p.opacity) +
                '"/>',
            )
            .join("") +
          "</" +
          tag +
          ">",
      );
    } else if (p.type.startsWith("GRADIENT")) {
      defs.push(
        '<pattern id="' +
          key +
          '" patternUnits="userSpaceOnUse" width="' +
          n.w +
          '" height="' +
          n.h +
          '"><image width="' +
          n.w +
          '" height="' +
          n.h +
          '" href="' +
          (await polarGradient(n, p)) +
          '"/></pattern>',
      );
    } else if (p.type === "IMAGE") {
      const url = "data:image/png;base64," + doc.images[p.imageHash!]!,
        i = await image(url);
      // Native Figma magnification uses texel boundaries rather than bilinear
      // interpolation. The native-export regression spec covers that choice.
      const magnified =
        p.scaleMode === "TILE"
          ? (p.scalingFactor ?? 1) >= 1
          : Math.min(n.w / i.width, n.h / i.height) >= 1;
      const sampling = magnified ? ' image-rendering="pixelated"' : "";
      let content = "";
      if (p.scaleMode === "TILE") {
        const scale = p.scalingFactor ?? 1;
        defs.push(
          '<pattern id="' +
            key +
            '" patternUnits="userSpaceOnUse" width="' +
            i.width * scale +
            '" height="' +
            i.height * scale +
            '"><image' +
            sampling +
            ' width="' +
            i.width * scale +
            '" height="' +
            i.height * scale +
            '" href="' +
            url +
            '"/></pattern>',
        );
        return "url(#" + key + ")";
      }
      if (p.scaleMode === "CROP" && p.imageTransform) {
        const m = multiply(
          multiply([n.w, 0, 0, n.h, 0, 0], inverse(p.imageTransform)),
          [1 / n.w, 0, 0, 1 / n.h, 0, 0],
        );
        content =
          "<image" +
          sampling +
          ' width="' +
          n.w +
          '" height="' +
          n.h +
          '" preserveAspectRatio="none" transform="' +
          matrix(m) +
          '" href="' +
          url +
          '"/>';
      } else
        content =
          "<image" +
          sampling +
          ' width="' +
          n.w +
          '" height="' +
          n.h +
          '" preserveAspectRatio="xMidYMid ' +
          (p.scaleMode === "FIT" ? "meet" : "slice") +
          '" href="' +
          url +
          '"/>';
      defs.push(
        '<pattern id="' +
          key +
          '" patternUnits="userSpaceOnUse" width="' +
          n.w +
          '" height="' +
          n.h +
          '">' +
          content +
          "</pattern>",
      );
    }
    return "url(#" + key + ")";
  }
  async function render(n: FigmaNode, parent?: Matrix): Promise<string> {
    if (n.visible === false) return "";
    const transform = parent
      ? multiply(inverse(parent), n.absoluteTransform)
      : n.absoluteTransform;
    const shape = outline(n);
    let body = "";
    // Plugin API paint arrays are in drawing order (last paint is on top).
    for (const fill of n.fills)
      body +=
        '<g fill="' +
        (await paint(n, fill)) +
        '" style="mix-blend-mode:' +
        blend(fill.blendMode).replace("source-over", "normal") +
        '">' +
        shape +
        "</g>";
    let activeMask: string | undefined;
    for (const child of n.children) {
      if (child.isMask) {
        const key = id();
        defs.push(
          '<mask id="' +
            key +
            '" maskUnits="userSpaceOnUse" x="-1000" y="-1000" width="3000" height="3000" style="mask-type:' +
            (child.maskType === "LUMINANCE" ? "luminance" : "alpha") +
            '">' +
            (await render({ ...child, isMask: false }, n.absoluteTransform)) +
            "</mask>",
        );
        activeMask = key;
      } else {
        const childSVG = await render(child, n.absoluteTransform);
        body += activeMask
          ? '<g mask="url(#' + activeMask + ')">' + childSVG + "</g>"
          : childSVG;
      }
    }
    if (n.clipsContent) {
      const key = id();
      defs.push('<clipPath id="' + key + '">' + shape + "</clipPath>");
      body = '<g clip-path="url(#' + key + ')">' + body + "</g>";
    }
    for (const stroke of n.strokes) {
      const color = await paint(n, stroke),
        width = n.strokeWeight ?? 1;
      if (
        n.strokeWeights &&
        n.type === "RECTANGLE" &&
        !n.radii?.some(Boolean)
      ) {
        const [t, r, b, l] = n.strokeWeights;
        body +=
          '<path fill="' +
          color +
          '" fill-rule="evenodd" d="M0 0H' +
          n.w +
          "V" +
          n.h +
          "H0Z M" +
          l +
          " " +
          t +
          "V" +
          (n.h - b) +
          "H" +
          (n.w - r) +
          "V" +
          t +
          'Z"/>';
        continue;
      }
      let attributes = "",
        strokeDashes = n.dashes?.join(" ") ?? "none",
        doubled = false;
      if (n.strokeAlign === "INSIDE") {
        const key = id();
        defs.push('<clipPath id="' + key + '">' + shape + "</clipPath>");
        attributes += ' clip-path="url(#' + key + ')"';
        doubled = true;
      }
      if (n.strokeAlign === "OUTSIDE") {
        const key = id();
        defs.push(
          '<mask id="' +
            key +
            '" maskUnits="userSpaceOnUse" x="-40" y="-40" width="' +
            (n.w + 80) +
            '" height="' +
            (n.h + 80) +
            '"><rect x="-40" y="-40" width="' +
            (n.w + 80) +
            '" height="' +
            (n.h + 80) +
            '" fill="white"/><g fill="black">' +
            shape +
            "</g></mask>",
        );
        attributes += ' mask="url(#' + key + ')"';
        doubled = true;
      }
      if (n.cap === "ARROW_EQUILATERAL") {
        const key = id();
        // SVG markers reproduce the five-width head measured from Figma's
        // native strokeGeometry. SVG computes the fixture shaft length.
        const measured = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "path",
        );
        measured.setAttribute("d", n.paths![0]!.data);
        strokeDashes =
          Math.max(0, measured.getTotalLength() - width * 10) + " 1000000";
        attributes += ' stroke-dashoffset="' + -width * 5 + '"';
        defs.push(
          '<marker id="' +
            key +
            '" markerUnits="strokeWidth" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto-start-reverse"><path d="M6 3.5L1 ' +
            (3.5 - 5 / Math.sqrt(3)) +
            "V" +
            (3.5 + 5 / Math.sqrt(3)) +
            'Z" fill="' +
            color +
            '"/></marker>',
        );
        attributes +=
          ' marker-start="url(#' + key + ')" marker-end="url(#' + key + ')"';
      }
      body +=
        '<g fill="none" stroke="' +
        color +
        '" stroke-width="' +
        width * (doubled ? 2 : 1) +
        '" stroke-linecap="' +
        (["ROUND", "SQUARE"].includes(n.cap ?? "")
          ? n.cap!.toLowerCase()
          : "butt") +
        '" stroke-linejoin="' +
        (n.join ?? "MITER").toLowerCase() +
        '" stroke-miterlimit="' +
        (n.miterLimit ?? 4) +
        '" stroke-dasharray="' +
        strokeDashes +
        '"' +
        attributes +
        ">" +
        shape +
        "</g>";
    }
    const shadows = n.effects.filter((e) =>
      ["DROP_SHADOW", "INNER_SHADOW"].includes(e.type),
    );
    if (shadows.length) {
      const key = id(),
        parts: string[] = [],
        outputs: string[] = [];
      for (const [i, e] of shadows.entries()) {
        const inner = e.type === "INNER_SHADOW",
          base = inner ? "invert" + i : "SourceAlpha";
        if (inner)
          parts.push(
            '<feComponentTransfer in="SourceAlpha" result="' +
              base +
              '"><feFuncA type="table" tableValues="1 0"/></feComponentTransfer>',
          );
        parts.push(
          '<feMorphology in="' +
            base +
            '" operator="dilate" radius="' +
            (e.spread ?? 0) +
            '" result="spread' +
            i +
            '"/><feOffset in="spread' +
            i +
            '" dx="' +
            (e.offset?.x ?? 0) +
            '" dy="' +
            (e.offset?.y ?? 0) +
            '"/><feGaussianBlur stdDeviation="' +
            (e.radius ?? 0) / 2 +
            '" result="blur' +
            i +
            '"/><feFlood flood-color="' +
            rgba(e.color!) +
            '"/><feComposite in2="blur' +
            i +
            '" operator="in" result="shadow' +
            i +
            '"/>',
        );
        if (inner)
          parts.push(
            '<feComposite in2="SourceAlpha" operator="in" result="shadow' +
              i +
              '"/>',
          );
        outputs.push('<feMergeNode in="shadow' + i + '"/>');
      }
      defs.push(
        '<filter id="' +
          key +
          '" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB">' +
          parts.join("") +
          "<feMerge>" +
          (shadows[0]!.type === "INNER_SHADOW"
            ? '<feMergeNode in="SourceGraphic"/>'
            : "") +
          outputs.reverse().join("") +
          (shadows[0]!.type === "DROP_SHADOW"
            ? '<feMergeNode in="SourceGraphic"/>'
            : "") +
          "</feMerge></filter>",
      );
      body = '<g filter="url(#' + key + ')">' + body + "</g>";
    }
    const blur = n.effects.find((e) => e.type === "LAYER_BLUR");
    if (blur) {
      const key = id();
      defs.push(
        '<filter id="' +
          key +
          '" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="' +
          (blur.radius ?? 0) / 2 +
          '"/></filter>',
      );
      body = '<g filter="url(#' + key + ')">' + body + "</g>";
    }
    return (
      '<g transform="' +
      matrix(transform) +
      '" opacity="' +
      (n.opacity ?? 1) +
      '">' +
      body +
      "</g>"
    );
  }
  const cv = document.createElement("canvas");
  cv.width = w;
  cv.height = h;
  const c = cv.getContext("2d")!;
  c.fillStyle = "#272727";
  c.fillRect(0, 0, w, h);
  const offset = referenceOffset(doc.nodes, w, h);
  for (const n of doc.nodes) {
    if (n.type === "TEXT") continue;
    defs.length = 0;
    const body = await render(n),
      svg =
        '<svg xmlns="http://www.w3.org/2000/svg" width="' +
        w +
        '" height="' +
        h +
        '"><defs>' +
        defs.join("") +
        '</defs><g transform="translate(' +
        offset.x +
        " " +
        offset.y +
        ')">' +
        body +
        "</g></svg>";
    const im = await image("data:image/svg+xml;base64," + btoa(svg));
    if (n.blendMode === "LINEAR_BURN") {
      // This fixture is opaque and has a constant backdrop at this cell.
      const m = n.absoluteTransform,
        fill = n.fills[0]!.color!,
        s = [fill.r, fill.g, fill.b].map((v) => Math.round(v * 255));
      c.fillStyle =
        "rgb(" + s.map((v) => Math.max(0, 39 + v - 255)).join(",") + ")";
      c.fillRect(m[4] + offset.x, m[5] + offset.y, n.w, n.h);
    } else {
      c.globalCompositeOperation = blend(
        n.blendMode,
      ) as GlobalCompositeOperation;
      c.drawImage(im, 0, 0);
    }
    c.globalCompositeOperation = "source-over";
  }
  return { canvas: cv, offset };
}

/** Browser CSS is an independent layout oracle for these two fixture text boxes. */
export function textReference(n: FigmaNode, offset: { x: number; y: number }) {
  const t = n.text!,
    first = t.runs[0]!,
    parts: string[] = [];
  for (const r of t.runs) {
    let text = t.characters.slice(r.start, r.end);
    if (r.textCase === "UPPER") text = text.toUpperCase();
    if (r.textCase === "LOWER") text = text.toLowerCase();
    const style =
      "font-family:" +
      escape(r.font) +
      ";font-size:" +
      r.size +
      "px;font-weight:" +
      (r.weight ?? (/Bold/.test(r.style) ? 700 : 400)) +
      ";font-style:" +
      (/Italic/.test(r.style) ? "italic" : "normal") +
      ";color:" +
      rgba(r.fills[0]!.color!) +
      ";line-height:" +
      (r.leading ?? t.leading ?? r.size * 1.2) +
      "px;letter-spacing:" +
      (r.tracking ?? t.tracking ?? 0) +
      "px;text-decoration:" +
      (r.decoration === "UNDERLINE"
        ? "underline"
        : r.decoration === "STRIKETHROUGH"
          ? "line-through"
          : "none") +
      ";";
    parts.push('<span style="' + style + '">' + escape(text) + "</span>");
  }
  const leading = t.leading ?? first.leading ?? first.size * 1.2;
  const align =
    t.align === "JUSTIFIED" ? "justify" : (t.align ?? "LEFT").toLowerCase();
  const style =
    "position:absolute;left:" +
    offset.x +
    "px;top:" +
    offset.y +
    "px;transform:" +
    "matrix(" +
    n.absoluteTransform.join(",") +
    ")" +
    ";transform-origin:0 0;width:" +
    n.w +
    "px;height:" +
    n.h +
    "px;overflow:hidden;font:" +
    first.size +
    "px " +
    escape(first.font) +
    ";line-height:" +
    leading +
    "px;display:flex;align-items:" +
    (t.verticalAlign === "CENTER"
      ? "center"
      : t.verticalAlign === "BOTTOM"
        ? "flex-end"
        : "flex-start") +
    ";text-align:" +
    align +
    ";";
  let html = parts.join("");
  if (t.maxLines) html = html.replaceAll("\n", "<br>");
  else if (t.characters.includes("\n"))
    html = html
      .split("\n")
      .map(
        (p) =>
          '<p style="margin:0 0 ' +
          (t.paragraphSpacing ?? 0) +
          "px;text-indent:" +
          (t.paragraphIndent ?? 0) +
          'px">' +
          p +
          "</p>",
      )
      .join("");
  return (
    '<div data-reference-text="' +
    n.id +
    '" style="' +
    style +
    '"><div style="width:100%;text-indent:' +
    (t.paragraphIndent ?? 0) +
    "px;" +
    '">' +
    html +
    "</div></div>"
  );
}

/** Native line-clamp can omit the marker at an explicit BR. Measure browser
 * line boxes and append Figma's explicit ENDING marker instead of relying on
 * that browser quirk. The renderer's text layout is never consulted. */
export function finishReferenceText(host: HTMLElement, doc: FigmaDoc) {
  for (const n of doc.nodes.filter(
    (n) => n.text?.maxLines && n.text.truncation === "ENDING",
  )) {
    const body = host.shadowRoot!.querySelector(
      '[data-reference-text="' + n.id + '"] > div',
    ) as HTMLElement;
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT),
      chars: Array<{ node: Text; end: number; line: number }> = [];
    const tops: number[] = [];
    while (walker.nextNode()) {
      const text = walker.currentNode as Text;
      for (let i = 0; i < text.length; i++) {
        const r = document.createRange();
        r.setStart(text, i);
        r.setEnd(text, i + 1);
        const box = r.getBoundingClientRect();
        let line = tops.findIndex((y) => Math.abs(y - box.top) < 0.5);
        if (line < 0) {
          line = tops.length;
          tops.push(box.top);
        }
        chars.push({ node: text, end: i + 1, line });
      }
    }
    if (tops.length <= n.text!.maxLines!) continue;
    const last = chars.filter((c) => c.line < n.text!.maxLines!).at(-1)!;
    const range = document.createRange();
    range.setStart(last.node, last.end);
    range.setEnd(body, body.childNodes.length);
    range.deleteContents();
    last.node.parentElement!.append(document.createTextNode("…"));
  }
}

export async function blendProof(
  doc: FigmaDoc,
  gray: string,
  grayReference: string,
  colored: string,
  offset: { x: number; y: number },
) {
  const actual = await image(gray),
    reference = await image(grayReference),
    colorActual = await image(colored);
  const colorReference = document.createElement("canvas");
  colorReference.width = actual.width;
  colorReference.height = actual.height;
  const native = colorReference.getContext("2d")!;
  native.fillStyle = "#345678";
  native.fillRect(0, 0, actual.width, actual.height);
  const nodes = doc.nodes.filter(
    (n) => n.blendMode === "HUE" || n.blendMode === "SATURATION",
  );
  for (const n of nodes) {
    native.globalCompositeOperation =
      n.blendMode!.toLowerCase() as GlobalCompositeOperation;
    native.fillStyle =
      "rgb(" +
      [n.fills[0]!.color!.r, n.fills[0]!.color!.g, n.fills[0]!.color!.b]
        .map((v) => Math.round(v * 255))
        .join(",") +
      ")";
    native.fillRect(
      n.absoluteTransform[4] + offset.x,
      n.absoluteTransform[5] + offset.y,
      n.w,
      n.h,
    );
  }
  const a = document.createElement("canvas");
  a.width = actual.width;
  a.height = actual.height;
  const ac = a.getContext("2d")!;
  ac.drawImage(colorActual, 0, 0);
  const proof = document.createElement("canvas");
  proof.width = 640;
  proof.height = 230;
  const c = proof.getContext("2d")!;
  c.fillStyle = "#eee";
  c.fillRect(0, 0, 640, 230);
  c.fillStyle = "#111";
  c.font = "12px Arial";
  const xs = [110, 240, 370, 500],
    images = [actual, reference, colorActual, colorReference],
    labels = [
      "Gray: Powermove",
      "Gray: browser",
      "Blue: Powermove",
      "Blue: browser",
    ];
  labels.forEach((label, i) => c.fillText(label, xs[i]!, 20));
  const cells = nodes.map((n, row) => {
    const x = n.absoluteTransform[4] + offset.x,
      y = n.absoluteTransform[5] + offset.y,
      top = 40 + row * 100;
    c.fillStyle = "#111";
    c.fillText(n.blendMode!, 8, top + 32);
    images.forEach((im, i) => {
      c.drawImage(im, x, y, n.w, n.h, xs[i]!, top, n.w, n.h);
      c.strokeStyle = "#aaa";
      c.strokeRect(xs[i]! - 0.5, top - 0.5, n.w + 1, n.h + 1);
    });
    const av = ac.getImageData(x + 2, y + 2, n.w - 4, n.h - 4).data,
      rv = native.getImageData(x + 2, y + 2, n.w - 4, n.h - 4).data;
    let max = 0,
      changed = 0;
    for (let i = 0; i < av.length; i += 4) {
      for (let k = 0; k < 3; k++)
        max = Math.max(max, Math.abs(av[i + k]! - rv[i + k]!));
      if (av[i] !== 52 || av[i + 1] !== 86 || av[i + 2] !== 120) changed++;
    }
    return {
      name: n.name,
      maxInterior: max,
      changedInterior: changed,
      center: Array.from(ac.getImageData(x + n.w / 2, y + n.h / 2, 1, 1).data),
      reference: Array.from(
        native.getImageData(x + n.w / 2, y + n.h / 2, 1, 1).data,
      ),
    };
  });
  return { png: proof.toDataURL(), cells };
}
