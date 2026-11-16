import { expect, test } from "bun:test";
import { menuEntries } from "../presentation";

test("parity Project menus name the composition and asset timeline commands", () => {
  const rows = menuEntries([
    {command: {NestComposition: "nested"}, enabled: true},
    {command: {AddAsset: "image"}, enabled: true},
  ]);
  expect(rows.map(row => row.label)).toEqual(["Add Composition to Timeline", "Add Layer from Asset"]);
  expect(rows.every(row => row.enabled)).toBe(true);
});
