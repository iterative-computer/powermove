import { expect, test } from "bun:test";
import { CompatKeymap, type KeyInput } from "../src/keymap";
import { chordOfEvent } from "../src/keychord";
const event = (chord: string, field = false, repeat = false): KeyInput => ({
  chord,
  field,
  repeat,
  defaultPrevented: false,
  preventDefault() {
    this.defaultPrevented = true;
  },
});
test("priority, exact/loose specificity and stable registration order", () => {
  const k = new CompatKeymap(() => {});
  for (const [command, key, priority, looseModifiers] of [
    ["loose", "q", 0, true],
    ["specific", "cmd+q", 0, true],
    ["exact", "cmd+shift+q", 0, false],
    ["priority", "q", -1, true],
  ] as const)
    k.bind("a", { command, key, priority, looseModifiers });
  expect(k.bindingsFor("cmd+shift+q").map((x) => x.command)).toEqual([
    "priority",
    "exact",
    "specific",
    "loose",
  ]);
});
test("ordinary literal-false pre-cancels; fields and repeats decline without cancelling", () => {
  const k = new CompatKeymap(() => {}),
    calls: string[] = [];
  k.commands.register("a", {
    id: "decline",
    run: () => {
      calls.push("decline");
      return false;
    },
  });
  k.commands.register("a", {
    id: "handle",
    run: () => {
      calls.push("handle");
      return 0;
    },
  });
  k.bind("a", { key: "q", command: "decline", priority: -10 });
  const ordinary = event("q");
  k.bubble(ordinary);
  expect(ordinary.defaultPrevented).toBe(true);
  const repeat = event("q", false, true);
  k.bubble(repeat);
  expect(calls).toEqual(["decline"]);
  k.bind("a", {
    key: "q",
    command: "decline",
    priority: -10,
    inFields: true,
    repeat: true,
  });
  const field = event("q", true);
  k.bubble(field);
  expect(field.defaultPrevented).toBe(false);
  k.bind("a", { key: "q", command: "handle", inFields: true, repeat: true });
  k.bubble(field);
  expect(field.defaultPrevented).toBe(true);
});
test("Promise false handles immediately; prior cancellation does not suppress commands", async () => {
  const k = new CompatKeymap(() => {}),
    calls: string[] = [];
  k.commands.register("a", {
    id: "promise",
    run: () => {
      calls.push("promise");
      return Promise.resolve(false);
    },
  });
  k.commands.register("a", {
    id: "after",
    run: () => {
      calls.push("after");
    },
  });
  k.bind("a", { key: "q", command: "promise", priority: -10 });
  k.bind("a", { key: "q", command: "after" });
  const e = event("q");
  e.defaultPrevented = true;
  k.bubble(e);
  await Promise.resolve();
  expect(calls).toEqual(["promise"]);
});
test("command overrides and owner disposal restore providers", () => {
  const k = new CompatKeymap(() => {}),
    base = k.commands.register("base", { id: "q", run: () => 1 });
  k.commands.register("replacement", { id: "q", run: () => 2 });
  expect(k.run("q")).toBe(2);
  base.dispose();
  expect(k.run("q")).toBe(2);
  k.disposeOwner("replacement");
  expect(k.run("q")).toBe(false);
});
test("chord handling uses supplied event fields without browser globals", () => {
  expect(
    chordOfEvent({
      key: "!",
      code: "Digit1",
      metaKey: false,
      ctrlKey: false,
      shiftKey: true,
      altKey: false,
    }),
  ).toBe("shift+1");
  expect(
    chordOfEvent({
      key: "Dead",
      code: "",
      metaKey: false,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
    }),
  ).toBeNull();
});
