import type { Evaluation } from "../../eval/harness";

import { Section, NumField, TextField } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  await e.render(
    <Section title="Export">
      <TextField id="name" value="" binding="synthetic:name" />
      <NumField id="fps" value={23.976} binding="synthetic:fps" />
      <div id="progress" role="progressbar" value={0.5} />
    </Section>,
  );
  e.props("name", { value: "" });
  e.props("fps", { value: 23.976 });
  e.props("progress", { value: 0.5, role: "progressbar" });
}
