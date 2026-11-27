import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const metrics = [
    { id: "latency", value: 0.75 },
    { id: "frames", value: 120 },
  ];
  await e.render(
    <div id="grid" style={{ display: "grid" }}>
      {metrics.map((metric) => (
        <div key={metric.id} id={metric.id} value={metric.value}>
          <text>{metric.id}</text>
        </div>
      ))}
    </div>,
  );
  e.props("latency", { value: 0.75 });
  e.props("frames", { value: 120 });
}
