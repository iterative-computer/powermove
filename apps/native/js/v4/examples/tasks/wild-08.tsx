import type { Evaluation } from "../../eval/harness";

import { RampField, PointField, FillField } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  const stops = [
    { id: "a", color: "#11223344", position: 0 },
    { id: "b", color: "#abcd1234", position: 1 },
  ];
  await e.render(
    <div>
      <RampField id="ramp" value={stops} binding="synthetic:ramp" />
      <PointField
        id="point"
        value={[1.5, -2]}
        binding="synthetic:point"
        xBinding="synthetic:x"
        yBinding="synthetic:y"
      />
      <FillField
        id="fill"
        value={{ kind: "solid", color: "#11223344" }}
        binding="synthetic:fill"
      />
    </div>,
  );
  e.props("ramp", { value: stops });
  e.props("point", {
    value: [1.5, -2],
    attr: { xBinding: "synthetic:x", yBinding: "synthetic:y" },
  });
  e.props("fill", { value: { kind: "solid", color: "#11223344" } });
}
