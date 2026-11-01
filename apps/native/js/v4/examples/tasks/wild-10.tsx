import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(
    <button
      id="styled"
      class="item"
      role="option"
      attr={{ kind: "metric" }}
      state={{ selected: false }}
      style={{ gap: { token: "metrics.gap" } }}
    />,
  );
  await e.root.setStyleRules([
    {
      selector: {
        kind: "all",
        value: [
          { kind: "id", value: "styled" },
          { kind: "class", value: "item" },
          { kind: "role", value: "option" },
          { kind: "attr", value: { name: "kind", value: "metric" } },
          { kind: "state", value: { name: "selected", value: false } },
        ],
      },
      style: { gap: { token: "metrics.gap" } },
    },
  ]);
  e.props("styled", {
    class: "item",
    role: "option",
    attr: { kind: "metric" },
    state: { selected: false },
  });
  e.compareNative();
}
