import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const view = (ids: string[]) => (
    <scroll id="rows">
      {ids.map((id) => (
        <button key={id} id={id} label={id} />
      ))}
    </scroll>
  );
  await e.render(view(["one", "two", "three"]));
  const ids = ["one", "two", "three"].map((id) => e.node(id).id);
  await e.render(view(["three", "two", "one"]));
  e.equal(e.node("rows").children, [...ids].reverse(), "keyed reverse");
}
