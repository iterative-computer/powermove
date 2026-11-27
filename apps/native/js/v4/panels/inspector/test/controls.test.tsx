import { expect, test } from "bun:test";
import { InspectorPanel } from "../view";
import { fixture, snapshot, binding } from "./fixture";
import { Probe } from "./probe";

async function controls() {
  const s = snapshot();
  const c = s.sections[0].rows[0].control;
  if (c.kind === "number") {
    c.axes[0].min = 0;
    c.axes[0].max = 100;
    c.axes[0].value = 50;
  }
  s.sections[0].rows.push(
    {
      id: "visible",
      label: "Visible",
      control: {
        kind: "toggle",
        value: false,
        mixed: true,
        binding: binding("visible"),
      },
    },
    {
      id: "choice",
      label: "Parent",
      control: {
        kind: "choice",
        value: "a",
        binding: binding("choice"),
        options: [
          { v: null, label: "None" },
          { v: false, label: "None" },
        ],
      },
    },
    {
      id: "text",
      label: "Text",
      control: { kind: "text", value: "A日本🙂B", binding: binding("text") },
    },
  );
  const f = fixture(s),
    p = new Probe();
  await f.session.start();
  await p.render(<InspectorPanel session={f.session} />);
  return {
    f,
    p,
    async dispose() {
      await p.dispose();
      await f.session.dispose();
    },
  };
}
test("native numeric typing evaluates relative arithmetic, draft stepping and Enter once", async () => {
  const c = await controls();
  try {
    const results = c.p.native(
      { control: { type: "focus", id: "value:opacity:0" } },
      { control: { type: "input", id: "value:opacity:0", text: "*2" } },
      {
        control: {
          type: "key",
          id: "value:opacity:0",
          key: "ArrowDown",
          shift: true,
        },
      },
      { control: { type: "key", id: "value:opacity:0", key: "Enter" } },
    );
    expect(results.every((r) => r.accepted)).toBe(true);
    expect(results.flatMap((r) => r.edits)).toEqual([
      { type: "begin", binding: "opacity" },
      { type: "write", binding: "opacity", value: 90 },
      { type: "commit", binding: "opacity" },
    ]);
    expect(c.f.model.intents).toEqual([]);
  } finally {
    await c.dispose();
  }
});
test("native scrub is unbounded, uses both axes and modifiers, and Escape cancels original binding", async () => {
  const c = await controls();
  try {
    const results = c.p.native(
      { scrub: { type: "begin", id: "value:opacity:0", x: 0, y: 0 } },
      { scrub: { type: "sample", x: 20, y: 10 } },
      { scrub: { type: "sample", x: 10000, y: 0, alt: true } },
      { scrub: { type: "finish", cancel: true } },
    );
    expect(results.every((r) => r.accepted)).toBe(true);
    expect(results.flatMap((r) => r.edits)).toEqual([
      { type: "begin", binding: "opacity" },
      { type: "write", binding: "opacity", value: 55 },
      { type: "write", binding: "opacity", value: 100 },
      { type: "cancel", binding: "opacity" },
    ]);
  } finally {
    await c.dispose();
  }
});
test("native mixed checkbox and typed null/false choices preserve exact values", async () => {
  const c = await controls();
  try {
    const results = c.p.native(
      { control: { type: "toggle", id: "value:visible", key: " " } },
      { control: { type: "select", id: "value:choice", index: 0 } },
      { control: { type: "select", id: "value:choice", index: 1 } },
    );
    expect(
      results
        .flatMap((r) => r.edits)
        .filter((edit) => edit.type === "write")
        .map((edit) => edit.value),
    ).toEqual([true, null, false]);
  } finally {
    await c.dispose();
  }
});
test("native text preview preserves Unicode and cancellation emits one rollback", async () => {
  const c = await controls();
  try {
    const results = c.p.native(
      { control: { type: "focus", id: "value:text" } },
      { control: { type: "input", id: "value:text", text: "日本🙂追加" } },
      { control: { type: "finish", id: "value:text", cancel: true } },
    );
    expect(results.flatMap((r) => r.edits)).toEqual([
      { type: "begin", binding: "text" },
      { type: "write", binding: "text", value: "日本🙂追加" },
      { type: "cancel", binding: "text" },
    ]);
  } finally {
    await c.dispose();
  }
});
test("all 16 appearance combinations validate tokens and preserve authored control values", async () => {
  const c = await controls();
  try {
    for (const light of [false, true])
      for (const contrast of [false, true])
        for (const motion of [false, true])
          for (const transparency of [false, true]) {
            const [result] = c.p.native({
              appearance: { light, contrast, motion, transparency },
            });
            expect(result.diagnostics).toEqual([]);
            expect(
              result.nodes[c.p.node("value:opacity:0").id].props.value,
            ).toBe(50);
            expect(
              result.accessibility.find(
                (n) => n.id === c.p.node("value:visible").id,
              )?.toggled,
            ).toBe("Mixed");
          }
  } finally {
    await c.dispose();
  }
});
