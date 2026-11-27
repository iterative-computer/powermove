import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const authored = "  # Heading\n\n\tconst α = 0;  \n";
  await e.render(
    <div>
      <markdown id="markdown" source={authored} />
      <code id="code" source={authored} />
    </div>,
  );
  e.props("markdown", { source: authored });
  e.props("code", { source: authored });
}
