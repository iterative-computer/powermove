import type { Evaluation } from "../../eval/harness";
import { Fragment } from "powermove-v4";

export async function evaluate(e: Evaluation) {
  await e.render(
    <>
      <Fragment>
        {false}
        {null}
        {undefined}
        <text id="real">Exact</text>
        {true}
      </Fragment>
      {[]}
    </>,
  );
  e.equal(
    e.nodes().filter((n) => n.id !== 0).length,
    2,
    "only authored element and text leaf",
  );
  e.assert(
    e.nodes().some((n) => n.text === "Exact"),
    "exact text",
  );
}
