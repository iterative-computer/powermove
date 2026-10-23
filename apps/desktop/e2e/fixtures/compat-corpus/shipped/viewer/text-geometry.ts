import type { TextCaretLayout, TextCaretLine } from 'powermove';

/* Pure caret and selection geometry for on-canvas text editing. All inputs
   and outputs are in the text layer's local space (see RenderAPI.textLayout).
   Source indexes are UTF-16 offsets into the layer's text string, matching
   the hidden textarea's selectionStart/selectionEnd. */

export interface Rect { x: number; y: number; w: number; h: number }
export interface CaretPosition { x: number; top: number; height: number; line: number }

function lineRangeEnd(layout: TextCaretLayout, row: number): number {
  const next = layout.lines[row + 1];
  return next ? next.start : layout.length + 1;
}

/** The display line that owns a source index. Indexes inside the whitespace
    dropped at a wrap boundary belong to the line before the wrap, so the
    caret rests at that line's end instead of the next line's start. */
export function lineForIndex(layout: TextCaretLayout, index: number): number {
  const clamped = Math.max(0, Math.min(layout.length, index));
  for (let row = 0; row < layout.lines.length; row++) {
    if (clamped < lineRangeEnd(layout, row)) return row;
  }
  return Math.max(0, layout.lines.length - 1);
}

/** Vertical extent shared by the caret and selection highlights: one line
    pitch, centered on the glyph box so consecutive lines abut exactly. */
export function lineBox(layout: TextCaretLayout, line: TextCaretLine): { top: number; height: number } {
  const height = Math.max(layout.size, layout.lineHeight);
  return { top: line.y + layout.size / 2 - height / 2, height };
}

function boundaryX(line: TextCaretLine, index: number): number {
  const boundaries = line.boundaries;
  if (!boundaries.length) return line.x;
  if (index <= boundaries[0]!.index) return boundaries[0]!.x;
  const last = boundaries[boundaries.length - 1]!;
  if (index >= last.index) return last.x;
  // Indexes inside a grapheme cluster snap to the nearer cluster edge.
  let previous = boundaries[0]!;
  for (const boundary of boundaries) {
    if (boundary.index === index) return boundary.x;
    if (boundary.index > index) return index - previous.index < boundary.index - index ? previous.x : boundary.x;
    previous = boundary;
  }
  return last.x;
}

export function caretAt(layout: TextCaretLayout, index: number): CaretPosition {
  if (!layout.lines.length) return { x: 0, top: 0, height: layout.size, line: 0 };
  const row = lineForIndex(layout, index);
  const line = layout.lines[row]!;
  const box = lineBox(layout, line);
  return { x: boundaryX(line, index), top: box.top, height: box.height, line: row };
}

/** Nearest source index to a local point. Points above the first line or
    below the last one clamp to those lines; horizontal overshoot clamps to
    the line's ends. */
export function indexAtPoint(layout: TextCaretLayout, x: number, y: number): number {
  if (!layout.lines.length) return 0;
  let row = layout.lines.length - 1;
  for (let candidate = 0; candidate < layout.lines.length; candidate++) {
    const box = lineBox(layout, layout.lines[candidate]!);
    if (y < box.top + box.height) { row = candidate; break; }
  }
  const line = layout.lines[row]!;
  let best = line.boundaries[0] ?? { index: line.start, x: line.x };
  for (const boundary of line.boundaries) {
    if (Math.abs(boundary.x - x) < Math.abs(best.x - x)) best = boundary;
  }
  return Math.min(layout.length, best.index);
}

/** Highlight rectangles for a selection range, one per display line. Lines
    that continue past the range end extend a small marker into the trailing
    whitespace so wrapped-line breaks still read as selected. */
export function selectionRects(layout: TextCaretLayout, start: number, end: number): Rect[] {
  const from = Math.max(0, Math.min(start, end)), to = Math.min(layout.length, Math.max(start, end));
  if (to <= from || !layout.lines.length) return [];
  const rects: Rect[] = [];
  const firstRow = lineForIndex(layout, from), lastRow = lineForIndex(layout, to);
  for (let row = firstRow; row <= lastRow; row++) {
    const line = layout.lines[row]!;
    const box = lineBox(layout, line);
    const lineEnd = line.start + line.text.length;
    const x0 = boundaryX(line, row === firstRow ? from : line.start);
    const continues = row < lastRow;
    const x1 = continues ? boundaryX(line, lineEnd) + layout.size * .25 : boundaryX(line, Math.min(to, lineEnd));
    rects.push({ x: Math.min(x0, x1), y: box.top, w: Math.max(0, Math.abs(x1 - x0)), h: box.height });
  }
  return rects;
}

/** Word range around an index for double-click selection. Whitespace runs
    select as their own unit, matching native textarea behavior. */
export function wordRangeAt(text: string, index: number): { start: number; end: number } {
  if (!text.length) return { start: 0, end: 0 };
  const at = Math.max(0, Math.min(text.length - 1, index));
  const isSpace = (ch: string) => /\s/u.test(ch);
  const isWord = (ch: string) => /[\p{L}\p{N}_'’]/u.test(ch);
  const probe = index >= text.length ? text[text.length - 1]! : text[at]!;
  const kind = isSpace(probe) ? isSpace : isWord(probe) ? isWord : (ch: string) => !isSpace(ch) && !isWord(ch);
  let start = index >= text.length ? text.length - 1 : at, end = start + 1;
  while (start > 0 && kind(text[start - 1]!) && text[start - 1] !== '\n') start--;
  while (end < text.length && kind(text[end]!) && text[end] !== '\n') end++;
  return { start, end };
}

/** Paragraph range around an index for triple-click selection. */
export function paragraphRangeAt(text: string, index: number): { start: number; end: number } {
  const at = Math.max(0, Math.min(text.length, index));
  const start = text.lastIndexOf('\n', at - 1) + 1;
  const newline = text.indexOf('\n', at);
  return { start, end: newline === -1 ? text.length : newline };
}

/** Caret index one display line up or down from `index`, keeping the caret
    x as the browser does with a sticky column. Returns null at the edges so
    the caller can fall back to line start / end. */
export function verticalMove(layout: TextCaretLayout, index: number, direction: -1 | 1, stickyX?: number): number | null {
  if (!layout.lines.length) return null;
  const current = caretAt(layout, index);
  const targetRow = current.line + direction;
  if (targetRow < 0 || targetRow >= layout.lines.length) return null;
  const line = layout.lines[targetRow]!;
  const box = lineBox(layout, line);
  return indexAtPoint(layout, stickyX ?? current.x, box.top + box.height / 2);
}

/** Local-space frame of the editable text: the authored box for paragraph
    text, otherwise the union of line boxes and glyph advances. */
export function textFrame(layout: TextCaretLayout): Rect {
  if (!layout.lines.length) return { x: 0, y: 0, w: 0, h: layout.size };
  const first = lineBox(layout, layout.lines[0]!), last = lineBox(layout, layout.lines[layout.lines.length - 1]!);
  const top = first.top, bottom = last.top + last.height;
  if (layout.boxWidth > 0) {
    const x = layout.align === 'center' ? -layout.boxWidth / 2 : layout.align === 'right' ? -layout.boxWidth : 0;
    const height = layout.boxHeight > 0 ? Math.max(layout.boxHeight, bottom - top) : bottom - top;
    return { x, y: top, w: layout.boxWidth, h: height };
  }
  let x0 = Infinity, x1 = -Infinity;
  for (const line of layout.lines) { x0 = Math.min(x0, line.x); x1 = Math.max(x1, line.x + line.width); }
  if (!Number.isFinite(x0) || x1 <= x0) { x0 = layout.lines[0]!.x; x1 = x0; }
  return { x: x0, y: top, w: x1 - x0, h: bottom - top };
}
