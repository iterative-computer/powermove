/** Styled source ranges stay in one editable text layer. All layout coordinates
 * are measured with the same font/kerning engine used to paint the glyphs. */
export interface RichGlyph {
  text: string;
  sourceStart: number;
  sourceEnd: number;
  style: any;
  width: number;
  x: number;
  y: number;
  baseline: number;
  line: number;
}
export interface RichLine {
  glyphs: RichGlyph[];
  width: number;
  height: number;
  y: number;
  paragraphEnd: boolean;
  marker?: string;
  indent: number;
}
export interface RichLayout {
  lines: RichLine[];
  glyphs: RichGlyph[];
  width: number;
  height: number;
  boxWidth: number;
  boxHeight: number;
}
export type MeasureRich = (
  text: string,
  style: any,
) => { width: number; ascent?: number; descent?: number };
export function layoutRichText(d: any, measure: MeasureRich): RichLayout {
  const source = String(d.text ?? ""),
    base = Object.fromEntries(
      [
        "font",
        "size",
        "weight",
        "italic",
        "color",
        "tracking",
        "decoration",
        "textCase",
        "paragraphSpacing",
        "paragraphIndent",
        "listSpacing",
        "listType",
        "indentation",
      ].map((k) => [k, d[k]]),
    ),
    runs = d.runs ?? [];
  base.leading = (d.leading ?? 1.2) * (d.size ?? 16);
  let activeRun = 0;
  const styleAt = (i: number) => {
    while (activeRun < runs.length - 1 && i >= runs[activeRun].end) activeRun++;
    const r = runs[activeRun];
    return { ...base, ...(r && i >= r.start && i < r.end ? r : {}) };
  };
  const boxWidth = Math.max(0, Number(d.boxWidth) || 0),
    boxHeight = Math.max(0, Number(d.boxHeight) || 0);
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  const lines: RichLine[] = [],
    all: RichGlyph[] = [];
  let line: RichLine,
    paragraph = 0,
    y = 0,
    sourceOffset = 0;
  const startLine = (style: any, first: boolean) => {
    const list = style.listType && style.listType !== "NONE",
      indent =
        (list ? (Number(style.indentation) || 1) * 24 : 0) +
        (first ? Number(style.paragraphIndent) || 0 : 0);
    line = {
      glyphs: [],
      width: 0,
      height: Math.max(1, Number(style.leading) || Number(style.size) * 1.2),
      y,
      paragraphEnd: false,
      indent,
      ...(first && list
        ? { marker: style.listType === "ORDERED" ? `${paragraph + 1}.` : "•" }
        : {}),
    };
  };
  const finish = (end = false) => {
    line.paragraphEnd = end;
    lines.push(line);
    y +=
      line.height +
      (end
        ? (Number(
            line.glyphs[0]?.style.paragraphSpacing ?? d.paragraphSpacing,
          ) || 0) + (Number(line.glyphs[0]?.style.listSpacing) || 0)
        : 0);
  };
  for (const para of source.split("\n")) {
    const firstStyle = styleAt(sourceOffset);
    startLine(firstStyle, true);
    const glyphs: RichGlyph[] = [];
    for (const item of segmenter.segment(para)) {
      const index = sourceOffset + item.index,
        style = styleAt(index);
      let text = item.segment;
      if (style.textCase === "UPPER") text = text.toUpperCase();
      else if (style.textCase === "SMALL_CAPS_FORCED")
        text = text.toLowerCase();
      else if (style.textCase === "LOWER") text = text.toLowerCase();
      else if (
        style.textCase === "TITLE" &&
        (item.index === 0 || /\s/u.test(para[item.index - 1]!))
      )
        text = text.toUpperCase();
      glyphs.push({
        text,
        sourceStart: index,
        sourceEnd: index + item.segment.length,
        style,
        width: 0,
        x: 0,
        y: 0,
        baseline: 0,
        line: 0,
      });
    }
    const metrics = (gs: RichGlyph[]) => {
      let x = 0,
        prefix = "",
        last: any = null;
      for (const g of gs) {
        const key = JSON.stringify([
          g.style.font,
          g.style.size,
          g.style.weight,
          g.style.italic,
          g.style.tracking,
          g.style.textCase,
        ]);
        if (key !== last) prefix = "";
        const width =
          measure(prefix + g.text, g.style).width -
          measure(prefix, g.style).width;
        g.width = width;
        g.x = x;
        x += width;
        prefix += g.text;
        last = key;
      }
      return x;
    };
    let token: RichGlyph[] = [];
    const place = (chunk: RichGlyph[]) => {
      const width = metrics([...line.glyphs, ...chunk]),
        available = () =>
          boxWidth ? Math.max(1, boxWidth - line.indent) : Infinity;
      if (line.glyphs.length && width > available()) {
        finish();
        startLine(chunk[0]!.style, false);
        while (chunk.length && /^\s+$/u.test(chunk[0]!.text)) chunk.shift();
        metrics(chunk);
      }
      for (const g of chunk) {
        if (line.glyphs.length && line.width + g.width > available()) {
          finish();
          startLine(g.style, false);
          metrics([g]);
        }
        line.glyphs.push(g);
        line.width += g.width;
        line.height = Math.max(
          line.height,
          Number(g.style.leading) || g.style.size * 1.2,
        );
      }
    };
    for (const g of glyphs) {
      token.push(g);
      if (/^\s+$/u.test(g.text)) {
        place(token);
        token = [];
      }
    }
    if (token.length) place(token);
    finish(true);
    sourceOffset += para.length + 1;
    paragraph++;
  }
  let visible = lines,
    limit = d.maxLines > 0 ? d.maxLines : Infinity;
  if (boxHeight > 0) {
    let count = 0;
    for (const line of lines) {
      if (line.y + line.height > boxHeight + 0.01) break;
      count++;
    }
    limit = Math.min(limit, Math.max(1, count));
  }
  const truncated = lines.length > limit;
  visible = lines.slice(0, limit);
  if (truncated && d.truncation === "ENDING" && visible.length) {
    const line = visible.at(-1)!,
      last = line.glyphs.at(-1),
      style = last?.style ?? firstStyleFallback(d),
      ellipsis = {
        text: "…",
        sourceStart: last?.sourceEnd ?? source.length,
        sourceEnd: last?.sourceEnd ?? source.length,
        style,
        width: measure("…", style).width,
        x: 0,
        y: 0,
        baseline: 0,
        line: 0,
      };
    while (
      line.glyphs.length &&
      boxWidth &&
      line.glyphs.reduce((sum, g) => sum + g.width, 0) + ellipsis.width >
        boxWidth - line.indent
    )
      line.glyphs.pop();
    line.glyphs.push(ellipsis);
    line.width = line.glyphs.reduce((sum, g) => sum + g.width, 0);
  }
  const totalHeight = visible.length
      ? visible.at(-1)!.y + visible.at(-1)!.height
      : 0,
    vertical =
      boxHeight > totalHeight
        ? d.verticalAlign === "center"
          ? (boxHeight - totalHeight) / 2
          : d.verticalAlign === "bottom"
            ? boxHeight - totalHeight
            : 0
        : 0;
  for (const [row, line] of visible.entries()) {
    const available = boxWidth ? boxWidth - line.indent : line.width,
      align = d.align === "center" ? 0.5 : d.align === "right" ? 1 : 0;
    let x =
      line.indent +
      Math.max(0, available - line.width) * align -
      (boxWidth ? boxWidth * align : line.width * align);
    const spaces = line.glyphs.filter((g) => /^\s+$/u.test(g.text)).length,
      justify =
        d.align === "justify" && !line.paragraphEnd && spaces
          ? Math.max(0, available - line.width) / spaces
          : 0;
    const ascent = Math.max(
        0,
        ...line.glyphs.map(
          (g) => measure(g.text, g.style).ascent ?? g.style.size * 0.82,
        ),
      ),
      descent = Math.max(
        0,
        ...line.glyphs.map(
          (g) => measure(g.text, g.style).descent ?? g.style.size * 0.18,
        ),
      );
    const baseline =
      line.y +
      vertical +
      Math.max(ascent, (line.height - ascent - descent) / 2 + ascent);
    for (const g of line.glyphs) {
      g.x = x;
      g.y = line.y + vertical;
      g.baseline = baseline;
      g.line = row;
      x += g.width + (/^\s+$/u.test(g.text) ? justify : 0);
      all.push(g);
    }
  }
  return {
    lines: visible,
    glyphs: all,
    width: Math.max(1, boxWidth, ...visible.map((l) => l.width + l.indent)),
    height: Math.max(1, boxHeight, totalHeight),
    boxWidth,
    boxHeight,
  };
}
function firstStyleFallback(d: any) {
  return { ...d, leading: (d.leading ?? 1.2) * d.size };
}
