import type { Evaluation } from "../../eval/harness";

import { Disclosure } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  function Layers() {
    return (
      <Disclosure label="Layer">
        <button id="child" label="Child" />
      </Disclosure>
    );
  }
  await e.render(<Layers />);
  e.assert(!e.find("child"), "initially collapsed");
  await e.event(e.byLabel("Layer").id, "click");
  e.assert(!!e.find("child"), "expanded children");
}
