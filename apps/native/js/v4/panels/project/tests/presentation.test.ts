import { expect, test } from "bun:test";
import { cardActions, menuEntries, renditions } from "../presentation";
import { asset } from "./fixtures";

test("queued and generating previews use quiet per-clip preparation text", () => {
  expect(renditions({ proxy: "Queued" })).toBe("Preparing preview");
  expect(renditions({ proxy: { Generating: { percent: 42 } } })).toBe("Preparing preview 42%");
  expect(renditions({ proxy: { Ready: {} } })).toBe("Proxy");
  expect(renditions({ proxy: null, optimized: "Queued" })).toBe("Optimized Waiting");
});

test("project menu renders unused media commands and preserves their enabled state", () => {
  for (const enabled of [true, false]) {
    const entries = menuEntries([
      { command: "ImportFiles", enabled: true },
      { command: "SelectUnusedMedia", enabled },
      { command: "RemoveUnusedMedia", enabled },
    ]);
    expect(
      entries.slice(1).map(({ label, enabled, separatorBefore }) => ({
        label,
        enabled,
        separatorBefore,
      })),
    ).toEqual([
      { label: "Select Unused Media", enabled, separatorBefore: true },
      { label: "Remove Unused Media…", enabled, separatorBefore: false },
    ]);
  }
});

test("an offline asset has no card shortcut to Finder", () => {
  const card = asset("missing", "Offline");
  if ("Asset" in card.data)
    card.data.Asset[0].source_path = "/synthetic/missing.mov";
  expect(cardActions(card, 1).map(({ label }) => label)).not.toContain(
    "Show in Finder",
  );
});
