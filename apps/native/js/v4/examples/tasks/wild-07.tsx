import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(
    <div focusScope>
      <button id="outside" label="Outside" />
      <div id="scope" focusScope>
        <input id="first" label="First" value="" />
        <input id="second" label="Second" value="" />
      </div>
    </div>,
  );
  e.props("scope", { focusScope: true });
  e.props("first", { label: "First", value: "" });
  e.props("second", { label: "Second" });
}
