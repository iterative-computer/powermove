import type { Evaluation } from "../../eval/harness";

import { useState } from "powermove-v4/hooks";

export async function evaluate(e: Evaluation) {
  function Counter({ id }: { id: string }) {
    const [count, setCount] = useState(0);
    return <button id={id} value={count} onClick={() => setCount(count + 1)} />;
  }
  const view = (order: string[]) => (
    <div>
      {order.map((id) => (
        <div key={id}>
          <Counter id={id} />
        </div>
      ))}
    </div>
  );
  await e.render(view(["a", "b"]));
  await e.event("a", "click");
  const id = e.node("a").id;
  await e.render(view(["b", "a"]));
  e.props("a", { value: 1 });
  e.props("b", { value: 0 });
  e.equal(e.node("a").id, id, "nested keyed state");
}
