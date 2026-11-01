import type { Evaluation } from "../../eval/harness";

import { useState } from "powermove-v4/hooks";
import { Menu } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  function Popup() {
    const [open, setOpen] = useState(true);
    return (
      <div>
        {open && (
          <anchored id="popup" anchor={0}>
            <Menu
              label="Menu"
              items={[
                { id: "close", label: "Close", onSelect: () => setOpen(false) },
              ]}
            />
          </anchored>
        )}
      </div>
    );
  }
  await e.render(<Popup />);
  await e.event(e.byLabel("Close").id, "click");
  e.assert(!e.find("popup"), "menu closes");
}
