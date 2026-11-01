import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  await e.render(
    <button id="props" attr={{ retained: true }} disabled value="before" />,
  );
  await e.render(
    <button
      id="props"
      attr={undefined}
      disabled={false}
      value=""
      title={null}
    />,
  );
  e.props("props", { disabled: false, value: "" });
  e.assert(!("attr" in e.node("props").props), "removed attr");
  e.assert(!("title" in e.node("props").props), "null is removal");
}
