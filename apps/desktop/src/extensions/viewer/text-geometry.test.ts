import { describe, expect, it } from 'vitest';
import type { TextCaretLayout } from 'powermove';

import { caretAt, indexAtPoint, lineForIndex, paragraphRangeAt, selectionRects, textFrame, verticalMove, wordRangeAt } from './text-geometry';

/* 10px advance per character, 20px lines: "Hello brave" wraps at "brave"
   (the dropped space is index 5), then a hard break before "x". */
function layout(text = 'Hello brave\nx', boxWidth = 60): TextCaretLayout {
  const size = 16, lineHeight = 20;
  const lines = boxWidth ? [{ text: 'Hello', start: 0 }, { text: 'brave', start: 6 }, { text: 'x', start: 12 }] : [{ text: text.split('\n')[0]!, start: 0 }, { text: 'x', start: 12 }];
  return {
    size, lineHeight, length: text.length, align: 'left', boxWidth, boxHeight: 0,
    lines: lines.map((line, row) => ({
      ...line, x: 0, y: row * lineHeight, baseline: row * lineHeight + size * .82, width: line.text.length * 10,
      boundaries: Array.from({ length: line.text.length + 1 }, (_, k) => ({ index: line.start + k, x: k * 10 })),
    })),
  };
}

describe('caret geometry', () => {
  it('maps indexes to lines, keeping wrap whitespace on the line before', () => {
    const lay = layout();
    expect(lineForIndex(lay, 0)).toBe(0);
    expect(lineForIndex(lay, 5)).toBe(0); // the dropped space
    expect(lineForIndex(lay, 6)).toBe(1);
    expect(lineForIndex(lay, 11)).toBe(1); // before the hard newline
    expect(lineForIndex(lay, 12)).toBe(2);
    expect(lineForIndex(lay, 13)).toBe(2);
  });

  it('places the caret between glyphs and centers one line pitch on the glyph box', () => {
    const lay = layout();
    expect(caretAt(lay, 3)).toEqual({ x: 30, top: -2, height: 20, line: 0 });
    expect(caretAt(lay, 5)).toMatchObject({ x: 50, line: 0 });
    expect(caretAt(lay, 8)).toMatchObject({ x: 20, line: 1, top: 18 });
    expect(caretAt(lay, 13)).toMatchObject({ x: 10, line: 2 });
  });

  it('finds the nearest boundary to a point and clamps outside the text', () => {
    const lay = layout();
    expect(indexAtPoint(lay, 14, 5)).toBe(1);
    expect(indexAtPoint(lay, 16, 5)).toBe(2);
    expect(indexAtPoint(lay, 999, 25)).toBe(11);
    expect(indexAtPoint(lay, -50, 45)).toBe(12);
    expect(indexAtPoint(lay, 0, -400)).toBe(0);
    expect(indexAtPoint(lay, 6, 400)).toBe(13);
  });

  it('builds one highlight per line with a marker past wrapped line ends', () => {
    const lay = layout();
    const rects = selectionRects(lay, 3, 8);
    expect(rects).toHaveLength(2);
    expect(rects[0]).toEqual({ x: 30, y: -2, w: 24, h: 20 });
    expect(rects[1]).toEqual({ x: 0, y: 18, w: 20, h: 20 });
    expect(selectionRects(lay, 4, 4)).toEqual([]);
    expect(selectionRects(lay, 8, 3)).toEqual(rects);
  });

  it('moves vertically with a sticky column and reports edges', () => {
    const lay = layout();
    expect(verticalMove(lay, 3, 1)).toBe(9);
    expect(verticalMove(lay, 9, 1)).toBe(13);
    expect(verticalMove(lay, 13, 1, 30)).toBeNull();
    expect(verticalMove(lay, 2, -1)).toBeNull();
    expect(verticalMove(lay, 13, -1, 40)).toBe(10);
  });

  it('frames paragraph text by its authored width and point text by its glyphs', () => {
    expect(textFrame(layout())).toEqual({ x: 0, y: -2, w: 60, h: 60 });
    expect(textFrame(layout('Hello brave\nx', 0))).toEqual({ x: 0, y: -2, w: 110, h: 40 });
  });
});

describe('word and paragraph ranges', () => {
  it('selects words, whitespace runs and punctuation as separate units', () => {
    expect(wordRangeAt('Hello brave world', 7)).toEqual({ start: 6, end: 11 });
    expect(wordRangeAt('Hello  world', 6)).toEqual({ start: 5, end: 7 });
    expect(wordRangeAt("don't stop", 2)).toEqual({ start: 0, end: 5 });
    expect(wordRangeAt('a, b', 1)).toEqual({ start: 1, end: 2 });
    expect(wordRangeAt('end', 3)).toEqual({ start: 0, end: 3 });
    expect(wordRangeAt('', 0)).toEqual({ start: 0, end: 0 });
  });

  it('never lets a word or paragraph cross a line break', () => {
    expect(wordRangeAt('one\ntwo', 4)).toEqual({ start: 4, end: 7 });
    expect(paragraphRangeAt('one\ntwo\nthree', 5)).toEqual({ start: 4, end: 7 });
    expect(paragraphRangeAt('one\ntwo', 0)).toEqual({ start: 0, end: 3 });
    expect(paragraphRangeAt('one\ntwo', 7)).toEqual({ start: 4, end: 7 });
  });
});
