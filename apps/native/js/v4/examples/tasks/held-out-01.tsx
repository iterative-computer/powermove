import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const view = (order: string[]) => (
    <div id="siblings">
      {order.map((id) => (
        <text id={id} key={id}>
          {id}
        </text>
      ))}
    </div>
  );
  await e.render(view(["a", "b", "c"]));
  const id = e.node("a").id;
  await e.render(view(["b", "a", "c"]));
  e.equal(e.node("a").id, id, "move identity");
  e.equal(e.node("siblings").children[1], id, "middle position");
}
