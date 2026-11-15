import { expect, test } from "bun:test";
import { checkFrame } from "../model";
import { frame, layer } from "./fixtures";

test("retained 436-row outline is complete before any wheel requests", () => {
  const rows = Array.from({ length: 436 }, (_, i) => layer(`l${i}`));
  const f = frame({
    window: { first: 0, total: rows.length, rows, retained: true },
  });
  expect(checkFrame(f, { first: 0, visible: 20 }).window.rows.length).toBe(436);
  expect(() =>
    checkFrame(
      { ...f, window: { ...f.window, rows: rows.slice(1) } },
      { first: 0, visible: 20 },
    ),
  ).toThrow();
  expect(() =>
    checkFrame(
      { ...f, window: { ...f.window, retained: false } },
      { first: 0, visible: 20 },
    ),
  ).toThrow();
});

test("collapsed retained outline requires the new row total", () => {
  const rows = [layer("group")];
  expect(
    checkFrame(
      frame({ window: { first: 0, total: 1, rows, retained: true } }),
      { first: 400, visible: 20 },
    ).window.total,
  ).toBe(1);
});
