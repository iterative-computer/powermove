import { describe, expect, test } from "bun:test";
import {
  EasingModel,
  customPresets,
  librarySave,
  presetName,
  status,
  type EasingAction,
  type EasingState,
} from "./model";

const state = (count: number, open = true): EasingState => ({
  open,
  count,
  presets: [{ id: "easeInOut", curve: [0.42, 0, 0.58, 1] }],
});

describe("Easing panel model", () => {
  test("preset ids read as words", () => {
    expect(presetName("easeInOut")).toBe("Ease In Out");
    expect(presetName("expoOut")).toBe("Expo Out");
    expect(presetName("linear")).toBe("Linear");
  });
  test("the status line says what the actions will change", () => {
    expect(status(state(0, false), "")).toBe(
      "Open a project to ease its keyframes.",
    );
    expect(status(state(0), "")).toBe(
      "Select keyframes in the timeline to ease them.",
    );
    expect(status(state(1), "")).toBe("1 keyframe selected");
    expect(status(state(3), "")).toBe("3 keyframes selected");
    expect(status(state(3), "Refused")).toBe("Refused");
  });
  test("an action runs once, then the panel reads the selection again", async () => {
    let count = 2;
    const sent: EasingAction[] = [];
    const model = new EasingModel(
      () => state(count),
      async (a) => {
        sent.push(a);
      },
      (e) => {
        throw e;
      },
    );
    let seen = 0;
    model.subscribe(() => seen++);
    model.refresh();
    expect(model.snapshot().count).toBe(2);
    model.send({ type: "preset", preset: "easeInOut" });
    count = 5;
    await model.settled();
    expect(sent).toEqual([{ type: "preset", preset: "easeInOut" }]);
    expect(model.snapshot().count).toBe(5);
    expect(seen).toBe(2);
  });
  test("a refusal is shown until the selection changes", async () => {
    let count = 1;
    const model = new EasingModel(
      () => state(count),
      async () => {
        throw new Error("Select keyframes to apply easing");
      },
      () => {},
    );
    model.refresh();
    model.send({ type: "easyEase", side: "both" });
    await model.settled();
    expect(model.message).toBe("Select keyframes to apply easing");
    model.refresh();
    expect(model.message).toBe("Select keyframes to apply easing");
    count = 2;
    model.refresh();
    expect(model.message).toBe("");
  });
});

test("custom save round trips the native stored cubic and retains other groups", () => {
  const initial = {
    ...state(2),
    library: { stateRevision: "1", source: null },
  };
  const saved = librarySave(initial, "My curve", [0.4, 0.2, 0.6, 0.8]);
  expect(saved.action).toBe("import");
  const presets = customPresets(saved.source);
  expect(presets[0]!.name).toBe("My curve");
  presets[0]!.curve.forEach((n, i) =>
    expect(n).toBeCloseTo([0.4, 0.2, 0.6, 0.8][i]!, 10),
  );
  const next = librarySave(
    { ...initial, library: { stateRevision: "2", source: saved.source } },
    "Second",
    [0.5, 0, 0.5, 1],
  );
  expect(next.action).toBe("save");
  expect("group" in next && next.group).toBe("custom");
  expect(customPresets(saved.source)).toHaveLength(1);
  const serialized = JSON.stringify(saved.source);
  expect(customPresets(serialized)).toEqual(presets);
  const storedSave = librarySave(
    { ...initial, library: { stateRevision: "3", source: serialized } },
    "Stored",
    [0.5, 0, 0.5, 1],
  );
  expect(storedSave.action).toBe("save");
  expect("group" in storedSave && storedSave.group).toBe("custom");
  const vertical = librarySave(initial, "Vertical", [0, 1, 1, 0]);
  const normalized = customPresets(vertical.source)[0]!.curve;
  [0.001, 1, 0.999, 0].forEach((n, i) =>
    expect(normalized[i]).toBeCloseTo(n, 10),
  );
  const mixed = structuredClone(saved.source);
  mixed.groups[0].presets[0].curve.points[0].outInterp = "linear";
  const projected = customPresets(mixed)[0]!.curve;
  [1 / 3, 1 / 3, 0.6, 0.8].forEach((n, i) =>
    expect(projected[i]).toBeCloseTo(n, 10),
  );
  mixed.groups[0].presets[0].curve.points[0].outInterp = "hold";
  expect(customPresets(mixed)).toEqual([]);
  expect(mixed.groups[0].presets).toHaveLength(1);
});
