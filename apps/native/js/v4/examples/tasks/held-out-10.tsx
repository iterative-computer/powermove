import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(<button id="accepted" label="Accepted" />);
  const before = e.root.inspect();
  await e.rejectTail();
  e.equal(
    e.root.inspect(),
    before,
    "accepted tree unchanged after invalid tail",
  );
  await e.render(<button id="accepted" label="Still usable" />);
  e.props("accepted", { label: "Still usable" });
}
