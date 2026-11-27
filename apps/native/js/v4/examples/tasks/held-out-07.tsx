import type { Evaluation } from "../../eval/harness";

import { useState } from "powermove-v4/hooks";

export async function evaluate(e: Evaluation) {
  const { register, signature, refresh } = await import("powermove-v4/refresh");
  function Before() {
    const [n, setN] = useState(0);
    return (
      <button
        id="refresh"
        value={n}
        label="Before"
        onClick={() => setN(n + 1)}
      />
    );
  }
  function After() {
    const [n, setN] = useState(0);
    return (
      <button
        id="refresh"
        value={n}
        label="After"
        onClick={() => setN(n + 1)}
      />
    );
  }
  const identity = "synthetic/eval/refresh:Counter";
  register(Before, identity);
  signature(Before, "useState");
  await e.render(<Before />);
  await e.event("refresh", "click");
  register(After, identity);
  signature(After, "useState");
  refresh([{ previous: Before, next: After }]);
  await e.flush();
  e.props("refresh", { value: 1, label: "After" });
}
