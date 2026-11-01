import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const media = [
    { id: "m1", resource: "image:one" },
    { id: "m2", resource: "image:two" },
  ];
  await e.render(
    <scroll>
      {media.map((m) => (
        <img key={m.id} id={m.id} resource={m.resource} label={m.id} />
      ))}
    </scroll>,
  );
  e.props("m1", { resource: "image:one" });
  e.props("m2", { resource: "image:two" });
}
