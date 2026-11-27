import { expect, test } from "bun:test";
import { createRoot, type Commit } from ".";
import { memo } from "./memo";
import { TimelinePanel } from "../panels/timeline/panel";

test("native memo skips equal props and renders changed props", async () => {
  let renders = 0;
  const Child = memo(
    ({ value }: { value: number }) => {
      renders++;
      return <text>{value}</text>;
    },
    (a, b) => a.value === b.value,
  );
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(<Child value={1} />);
  await root.render(<Child value={1} />);
  expect(renders).toBe(1);
  await root.render(<Child value={2} />);
  expect(renders).toBe(2);
  expect(root.inspect().nodes.some((n) => n.text === "2")).toBe(true);
  await root.dispose();
});

test("loading Timeline preserves native focus and blur names and delivery", async () => {
  expect(typeof TimelinePanel).toBe("function");
  const seen: string[] = [];
  const commits: Commit[] = [];
  const root = createRoot(async (c) => {
    commits.push(c);
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  await root.render(
    <input
      id="focus-probe"
      onFocus={() => seen.push("focus")}
      onBlur={() => seen.push("blur")}
    />,
  );
  const node = root.inspect().nodes.find((n) => n.props.id === "focus-probe")!;
  expect(
    commits
      .flatMap((c) => c.ops)
      .flatMap((op) =>
        op.op === "setListeners" && op.id === node.id
          ? [Object.keys(op.flags).sort()]
          : [],
      ),
  ).toContainEqual(["blur", "focus"]);
  await root.drive(node.id, "focus");
  await root.drive(node.id, "blur");
  expect(seen).toEqual(["focus", "blur"]);
  await root.dispose();
});
