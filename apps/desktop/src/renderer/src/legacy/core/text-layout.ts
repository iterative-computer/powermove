/* Glyph-level text layout shared by the rasterizer and the SVG player. Both
   pass a 2D context already configured with the layer's font and tracking,
   so positions match whichever backend paints them. Offsets are in the text
   layer's local space: x from the alignment origin, y at the top of each line. */

export interface GlyphLayout { characters: any[]; words: any[]; lines: any[] }

/** The Type tool's drag gesture creates AE-style paragraph text. Keep the
    complete source string, but wrap its rendered lines inside the authored
    box and clip overflow below the box. */
export interface SourceLine { text: string; start: number }

/** Wrapped display lines with the source offset each one starts at. Caret
    placement maps a source index to the line whose range contains it, so
    every branch below records where its line begins in the source string. */
export function textSourceLines(d: any, context: any, lineHeight: number): SourceLine[] {
  const text = String(d.text == null ? '' : d.text);
  const paragraphs = text.split('\n');
  const boxWidth = Number(d.boxWidth);
  const lines: SourceLine[] = [];
  let offset = 0;
  if (!d.paragraph || !Number.isFinite(boxWidth) || boxWidth <= 0) {
    for (const paragraph of paragraphs) { lines.push({ text: paragraph, start: offset }); offset += paragraph.length + 1; }
    return lines;
  }
  const width = (value: string) => context.measureText(value).width;
  for (const paragraph of paragraphs) {
    const paragraphStart = offset;
    offset += paragraph.length + 1;
    if (!paragraph) { lines.push({ text: '', start: paragraphStart }); continue; }
    let line = '', lineStart = paragraphStart, tokenStart = paragraphStart;
    for (const token of paragraph.split(/(\s+)/u).filter(Boolean)) {
      const candidate = line + token;
      if (line && width(candidate) > boxWidth) {
        lines.push({ text: line.trimEnd(), start: lineStart });
        line = token.trimStart();
        lineStart = tokenStart + (token.length - line.length);
      } else {
        if (!line) lineStart = tokenStart;
        line = candidate;
      }
      tokenStart += token.length;
      while (line && width(line) > boxWidth) {
        let cut = 1;
        while (cut < line.length && width(line.slice(0, cut + 1)) <= boxWidth) cut++;
        lines.push({ text: line.slice(0, cut), start: lineStart });
        line = line.slice(cut);
        lineStart += cut;
      }
    }
    lines.push({ text: line.trimEnd(), start: lineStart });
  }
  const boxHeight = Number(d.boxHeight);
  if (!Number.isFinite(boxHeight) || boxHeight <= 0) return lines;
  return lines.slice(0, Math.max(1, Math.floor(boxHeight / Math.max(1, lineHeight))));
}

/* Use the exact same canvas text metrics as the rasterizer when a procedural
   tool needs to reason about glyph placement. */
export function glyphLayout(d: any, meas: CanvasRenderingContext2D | any): GlyphLayout {
  const size = Math.max(1, Number(d.size) || 16);
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  const lh = size * (d.leading || 1.15);
  const lines = textSourceLines(d, meas, lh).map(line => line.text);
  const width = (value: any) => meas.measureText(value).width;
  // Metrics stay local to this layout: a later font load or axis change must
  // measure again. Repeated glyphs can share their isolated width, but joined
  // prefixes still use the canvas so kerning and tracking remain exact.
  const segmentWidths = new Map<string, number>();
  const segmentWidth = (value: string): number => {
    const cached = segmentWidths.get(value);
    if (cached !== undefined) return cached;
    const measured = width(value);
    segmentWidths.set(value, measured);
    return measured;
  };
  const segmenter = typeof Intl !== 'undefined' && Intl.Segmenter
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : null;
  const graphemes = (value: any): any[] => {
    if (segmenter) {
      return [...segmenter.segment(value)].map(item => item.segment);
    }
    return Array.from(value);
  };
  const output: any = { characters: [], words: [], lines: [] };
  /* measureText(prefix) omits kerning between the prefix and the next glyph.
     Measuring the joined run and subtracting the isolated segment preserves
     that incoming pair adjustment when the segment becomes its own layer. */
  const segmentX = (lineStart: any, prefix: any, segment: any) => lineStart + width(prefix + segment) - segmentWidth(segment);
  let characterIndex = 0, wordIndex = 0, lineIndex = 0, sourceOffset=0;
  lines.forEach((line, row) => {
    const lineWidth = width(line);
    const startX = align === 'center' ? -lineWidth / 2 : align === 'right' ? -lineWidth : 0;
    const y = row * lh;
    const lineSource=Math.max(sourceOffset,String(d.text||'').indexOf(line,sourceOffset));
    const lineUnit = lineIndex;
    if (line.length) output.lines.push({ text: line, x: startX, y, w: lineWidth, line: row, sourceStart: lineSource, index: lineIndex++ });

    let prefix = '';let localWord=-1,wasSpace=true;
    for (const segment of graphemes(line)) {
      const x = segmentX(startX, prefix, segment);
      const space=/^\s+$/u.test(segment);if(!space&&wasSpace)localWord++;
      if (!space) output.characters.push({ text: segment, x, y, w: segmentWidth(segment), line: row, lineUnit, word:wordIndex+localWord, sourceStart:lineSource+prefix.length,index: characterIndex++ });
      wasSpace=space;
      prefix += segment;
    }

    sourceOffset=lineSource+line.length;
    const matcher = /\S+/gu;
    let match;
    while ((match = matcher.exec(line))) {
      const prefix = line.slice(0, match.index);
      output.words.push({ text: match[0], x: segmentX(startX, prefix, match[0]), y, w: segmentWidth(match[0]), line: row, sourceStart: lineSource + match.index, index: wordIndex++ });
    }
  });
  return output;
}
