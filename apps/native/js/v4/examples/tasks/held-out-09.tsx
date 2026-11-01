import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const value = {
    "": false,
    é: [null, "", -1.25, Number.MAX_SAFE_INTEGER, 1e-200],
    nested: { "__proto__-text": "safe", value: 0 },
  };
  await e.render(<div id="json" value={value} />);
  e.props("json", { value });
}
