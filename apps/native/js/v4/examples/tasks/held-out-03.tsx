import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const calls: string[] = [];
  const view = (capture: boolean) => (
    <button
      id="listener"
      onClickCapture={capture ? () => calls.push("capture") : undefined}
      onClick={!capture ? () => calls.push("bubble") : undefined}
      preventDefault={{ click: capture }}
    />
  );
  await e.render(view(false));
  const id = e.node("listener").id;
  await e.event(id, "click");
  await e.render(view(true));
  e.equal(e.node("listener").id, id, "listener identity");
  await e.event(id, "click");
  e.equal(calls, ["bubble", "capture"], "new phase only");
  e.listener(id, "click", {
    capture: true,
    bubble: false,
    preventDefault: true,
    keys: [],
  });
}
