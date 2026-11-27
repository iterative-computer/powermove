import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(
    <scroll>
      <canvas id="canvas" resource="recorder:scene" />
      <frame id="preview" resource="engine:frame" />
    </scroll>,
  );
  e.props("canvas", { resource: "recorder:scene" });
  e.props("preview", { resource: "engine:frame" });
}
