import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const points = [
    { x: 0, y: 0 },
    { x: 1, y: 0.25 },
    { x: 2, y: 1 },
  ];
  await e.render(
    <div>
      <path id="curve" points={points} />
      <frame id="frame" resource="engine:preview" time={1.25} />
    </div>,
  );
  e.props("curve", { points });
  e.props("frame", { resource: "engine:preview", time: 1.25 });
}
