import type { Evaluation } from "../../eval/harness";

import { ToolbarItem, Glass } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  let calls = 0;
  await e.render(
    <Glass id="glass" surface="toolbar">
      <ToolbarItem
        id="play"
        label="Play"
        active={false}
        onClick={() => calls++}
      />
    </Glass>,
  );
  e.props("glass", { surface: "toolbar" });
  await e.event("play", "click");
  e.equal(calls, 1, "toolbar activation");
}
