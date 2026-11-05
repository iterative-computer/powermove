import { expect, test } from "bun:test";
import { createRoot } from "../src";
import { NumberField, TextField } from "../src/components";
test("native numeric commits carry a typed value without becoming text observations", async () => {
  let observed: unknown;
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(
    <NumberField
      value={1}
      binding="native-lease"
      onChange={(event) => {
        observed = event.value;
      }}
    />,
  );
  const node = root.inspect().nodes.find((n) => n.props.control === "number")!;
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: node.id,
      preventDefault: false,
      deliveries: [{ id: node.id, capture: false }],
    },
    "change",
    { value: 12.5 },
  );
  expect(observed).toBe(12.5);
  await expect(
    root.dispatch(
      {
        revision: root.inspect().revision,
        target: node.id,
        preventDefault: false,
        deliveries: [],
      },
      "input",
      { value: 12.5 },
    ),
  ).rejects.toThrow("string");
  await root.dispose();
});
test("text controls still reject numeric observations", async () => {
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(<TextField value="1" binding="text-lease" />);
  const node = root.inspect().nodes.find((n) => n.props.control === "text")!;
  await expect(
    root.dispatch(
      {
        revision: root.inspect().revision,
        target: node.id,
        preventDefault: false,
        deliveries: [],
      },
      "change",
      { value: 2 },
    ),
  ).rejects.toThrow("string");
  await root.dispose();
});
