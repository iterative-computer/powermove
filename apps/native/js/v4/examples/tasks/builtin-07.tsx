import type { Evaluation } from "../../eval/harness";

import { Section, ToggleField, SelectField } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  await e.render(
    <Section title="Settings">
      <ToggleField id="enabled" value={false} binding="synthetic:enabled" />
      <SelectField
        id="quality"
        value={0}
        binding="synthetic:quality"
        options={[
          { v: 0, label: "Draft" },
          { v: false, label: "Off" },
        ]}
      />
    </Section>,
  );
  e.props("enabled", { value: false });
  e.props("quality", {
    value: 0,
    options: [
      { v: 0, label: "Draft" },
      { v: false, label: "Off" },
    ],
  });
}
