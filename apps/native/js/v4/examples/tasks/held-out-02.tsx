import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  let calls = 0;
  const view = (old: boolean) => (
    <div>
      {old ? (
        <div key="old" id="old">
          <button id="listener" onClick={() => calls++} />
        </div>
      ) : (
        <button key="new" id="new" label="New" />
      )}
    </div>
  );
  await e.render(view(true));
  const stale = e.node("listener").id;
  const revision = e.root.inspect().revision;
  await e.render(view(false));
  e.assert(!e.nodes().some((n) => n.id === stale), "subtree destroyed");
  e.assert(e.node("new").id > stale, "fresh ID");
  let refused = false;
  try {
    await e.root.dispatch(
      {
        revision,
        target: stale,
        preventDefault: false,
        deliveries: [{ id: stale, capture: false }],
      },
      "click",
    );
  } catch (error) {
    refused = String(error).includes("Stale surface event");
  }
  e.assert(refused, "stale revision is explicitly refused");
  e.equal(calls, 0, "late listener retired");
}
