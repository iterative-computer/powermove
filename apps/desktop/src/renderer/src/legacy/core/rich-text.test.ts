import { it, expect } from "vitest";
import { layoutRichText } from "./rich-text";
const base = {
  text: "ab cd ef",
  font: "Arial",
  size: 10,
  leading: 1.2,
  tracking: 0,
  align: "left",
  boxWidth: 0,
  boxHeight: 0,
};
const measure = (text: string, s: any) => ({
  width: (text.length * s.size) / 2,
  ascent: s.size * 0.8,
  descent: s.size * 0.2,
});
it("keeps auto-width mixed metrics on one line without changing the source", () => {
  const d = {
      ...base,
      runs: [{ start: 3, end: 5, size: 20, font: "Other", leading: 24 }],
    },
    l = layoutRichText(d, measure);
  expect(l.lines).toHaveLength(1);
  expect(l.width).toBe(50);
  expect(l.lines[0]!.height).toBe(24);
  expect(d.text).toBe("ab cd ef");
});
it("wraps using run metrics and preserves UTF-16 offsets", () => {
  const l = layoutRichText({ ...base, text: "ab cd", boxWidth: 16 }, measure);
  expect(l.lines.map((l) => l.glyphs.map((g) => g.text).join(""))).toEqual([
    "ab ",
    "cd",
  ]);
  expect(l.lines[1]!.glyphs[0]!.sourceStart).toBe(3);
});
it("applies paragraph spacing/indent, vertical alignment, case, lists and truncation without destructive edits", () => {
  const d = {
      ...base,
      text: "first\nsecond\nthird",
      boxWidth: 90,
      boxHeight: 60,
      verticalAlign: "bottom",
      maxLines: 2,
      truncation: "ENDING",
      paragraphSpacing: 5,
      paragraphIndent: 8,
      runs: [
        {
          start: 0,
          end: 18,
          listType: "ORDERED",
          indentation: 1,
          textCase: "UPPER",
        },
      ],
    },
    l = layoutRichText(d, measure);
  expect(l.lines).toHaveLength(2);
  expect(l.lines[0]!.marker).toBe("1.");
  expect(l.lines[0]!.indent).toBe(32);
  expect(l.glyphs[0]!.text).toBe("F");
  expect(l.glyphs.at(-1)!.text).toBe("…");
  expect(l.glyphs[0]!.y).toBeGreaterThan(0);
  expect(d.text).toBe("first\nsecond\nthird");
});
it("retains small-cap source case and releases first-line indent on wrapped lines", () => {
  const text = "Ab cd ef gh";
  const d = {
    ...base,
    text,
    boxWidth: 32,
    paragraphIndent: 10,
    runs: [{ start: 0, end: text.length, textCase: "SMALL_CAPS" }],
  };
  const l = layoutRichText(d, measure);
  expect(l.glyphs[0]!.text).toBe("A");
  expect(l.glyphs[1]!.text).toBe("b");
  expect(l.lines[0]!.indent).toBe(10);
  expect(l.lines[1]!.indent).toBe(0);
  expect(l.lines[1]!.width).toBeGreaterThan(22);
  expect(d.text).toBe(text);
});

it("justifies visible word gaps without distributing space into a hanging trailing blank", () => {
  const d = {
      ...base,
      text: "ab cd ef",
      align: "justify",
      boxWidth: 32,
      paragraphIndent: 2,
    },
    layout = layoutRichText(d, measure),
    line = layout.lines[0]!;
  expect(line.glyphs.map((g) => g.text).join("")).toBe("ab cd ");
  expect(line.glyphs.at(-1)!.width).toBe(0);
  const last = line.glyphs.at(-2)!;
  expect(last.x + last.width).toBe(32);
  expect(line.width).toBe(30);
  expect(layout.lines[1]!.glyphs[0]!.x).toBe(0);
  expect(d.text).toBe("ab cd ef");
});
