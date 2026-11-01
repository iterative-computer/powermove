import type { Evaluation } from "../../eval/harness";

import { Menu } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  const calls: string[] = [];
  await e.render(
    <Menu
      label="Commands"
      items={[
        {
          id: "unavailable",
          label: "Unavailable",
          disabled: true,
          onSelect: () => calls.push("bad"),
        },
        { id: "run", label: "Run", onSelect: () => calls.push("run") },
      ]}
    />,
  );
  e.assert(e.byLabel("Unavailable").props.disabled === true, "disabled choice");
  await e.event(e.byLabel("Run").id, "click");
  e.equal(calls, ["run"], "command activation");
}
