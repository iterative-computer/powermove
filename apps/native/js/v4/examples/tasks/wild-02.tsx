import type { Evaluation } from "../../eval/harness";
export async function evaluate(e: Evaluation) {
  const edge = (
    <path
      id="edge"
      points={[
        { x: 0, y: 0 },
        { x: 10, y: 20 },
      ]}
    />
  );
  await e.render(<div>{edge}</div>);
  // Public docs do not specify anchors. Wire diagnostics reject a point object;
  // use the accepted node ID and preserve authored point data as metadata.
  await e.render(
    <div>
      {edge}
      <anchored
        id="node"
        anchor={e.node("edge").id}
        attr={{ point: { x: 10, y: 20 } }}
      >
        <text>Node</text>
      </anchored>
    </div>,
  );
  e.props("node", {
    anchor: e.node("edge").id,
    attr: { point: { x: 10, y: 20 } },
  });
  e.props("edge", {
    points: [
      { x: 0, y: 0 },
      { x: 10, y: 20 },
    ],
  });
}
