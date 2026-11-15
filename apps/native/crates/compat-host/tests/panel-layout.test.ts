import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
function setup() {
  const context: any = {};
  runInNewContext(readFileSync(new URL("../src/panel-layout.js", import.meta.url), "utf8"), context);
  const frames: (() => void)[] = [], applied: any[] = [], acks: any[] = [];
  const queue = new context.PMPanelLayout((f: () => void) => frames.push(f), (p: any) => applied.push(p), (p: any) => acks.push(p));
  return { queue, applied, acks, tick: () => frames.splice(0).forEach(f => f()) };
}
test("placements coalesce per panel per animation frame", () => {
  const h = setup();
  h.queue.enqueue([{id:'a', generation:1}, {id:'b', generation:1}]);
  h.queue.enqueue([{id:'a', generation:2}]);
  expect(h.applied).toHaveLength(0);
  h.tick();
  expect(h.applied.map(p => [p.id,p.generation])).toEqual([['a',2],['b',1]]);
  expect(h.acks).toHaveLength(0);
  h.tick();
  expect(h.acks).toHaveLength(2);
});
test("stale placements and superseded paint acknowledgements are ignored", () => {
  const h = setup();
  h.queue.enqueue([{id:'a', generation:2}]);
  h.tick();
  h.queue.enqueue([{id:'a', generation:3}, {id:'a', generation:1}]);
  h.tick();
  expect(h.acks).toHaveLength(0);
  h.tick();
  expect(h.acks.map(p => p.generation)).toEqual([3]);
  expect(h.applied.map(p => p.generation)).toEqual([2,3]);
});
test("a not-yet-mounted panel is retried after a DOM batch and never acknowledged early", () => {
  const context: any = {};
  runInNewContext(readFileSync(new URL("../src/panel-layout.js", import.meta.url), "utf8"), context);
  let mounted = false;
  const frames: (() => void)[] = [], acks: any[] = [];
  const queue = new context.PMPanelLayout((f: () => void) => frames.push(f), () => mounted, (p: any) => acks.push(p));
  const tick = () => frames.splice(0).forEach(f => f());
  queue.enqueue([{id:'logos',generation:1}]);
  tick(); tick();
  expect(acks).toHaveLength(0);
  mounted = true;
  queue.refresh(); tick(); tick();
  expect(acks.map(p => p.generation)).toEqual([1]);
});
