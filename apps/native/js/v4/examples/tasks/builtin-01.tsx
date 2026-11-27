import type { Evaluation } from "../../eval/harness";

import { Row, NumField } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  await e.render(
    <Row label="Opacity">
      <NumField
        id="number"
        value={0.625}
        binding="synthetic:opacity"
        precision={3}
        unit="%"
      />
    </Row>,
  );
  e.props("number", {
    value: 0.625,
    binding: "synthetic:opacity",
    precision: 3,
    unit: "%",
  });
  e.assert(
    typeof e.node("number").props.labelledBy === "string",
    "row association",
  );
}
