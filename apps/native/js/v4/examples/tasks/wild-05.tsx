import type { Evaluation } from "../../eval/harness";

export async function evaluate(e: Evaluation) {
  const view = (start: number) => (
    <virtual-list
      id="list"
      count={1000000}
      range={[start, start + 12]}
      itemSize={1}
    >
      {Array.from({ length: 12 }, (_, offset) => (
        <button
          key={start + offset}
          id={`item-${start + offset}`}
          label={String(start + offset)}
        />
      ))}
    </virtual-list>
  );
  await e.render(view(0));
  e.equal(e.node("list").children.length, 12, "bounded initial window");
  await e.render(view(500000));
  e.equal(e.node("list").children.length, 12, "bounded moved window");
  e.assert(!e.find("item-0"), "old window retired");
}
