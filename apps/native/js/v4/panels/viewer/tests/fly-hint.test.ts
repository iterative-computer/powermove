import { test, expect } from "bun:test";
import { showsFlyHint } from "../contract";
import { fixture } from "./helpers";
test("Fly hint requires an active flight in a free 3D composition view", () => {
  const model = fixture().model!;
  expect(showsFlyHint(model, false)).toBe(false);
  model.studio = { tier: "fast", look: "camera", fly: true };
  expect(showsFlyHint(model, false)).toBe(false);
  model.studio.look = "free";
  model.studio.fly = false;
  expect(showsFlyHint(model, false)).toBe(false);
  model.studio.fly = true;
  expect(showsFlyHint(model, false)).toBe(true);
  expect(showsFlyHint(model, true)).toBe(false);
});
