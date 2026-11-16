import { describe, expect, test } from "bun:test";
import {
  EasingModel,
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
