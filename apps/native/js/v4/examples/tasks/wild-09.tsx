import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(<webview id="web" source="synthetic:local-panel" />);
  e.props("web", { source: "synthetic:local-panel" });
  await e.assertWebviewRequiresOptIn();
}
