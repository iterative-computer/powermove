import type { Evaluation } from "../../eval/harness";

import { useState } from "powermove-v4/hooks";

export async function evaluate(e: Evaluation) {
  function Caption() {
    const [value, setValue] = useState("A🙂B");
    return (
      <textarea
        id="caption"
        value={value}
        onInput={(event) => setValue(String(event.value))}
      />
    );
  }
  await e.render(<Caption />);
  await e.event("caption", "input", { value: "A日本🙂B" });
  e.props("caption", { value: "A日本🙂B" });
}
