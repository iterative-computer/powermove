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
test("restore and page load never revive a hidden tab or acknowledge its old paint", () => {
  const context: any = {};
  runInNewContext(readFileSync(new URL("../src/panel-layout.js", import.meta.url), "utf8"), context);
  const frames: (() => void)[] = [], acks: any[] = [], roots = new Map<string, boolean>();
  const queue = new context.PMPanelLayout((f: () => void) => frames.push(f),
    (p: any) => roots.has(p.id), (p: any) => acks.push(p),
    (active: Set<string>) => { for (const id of roots.keys()) roots.set(id, active.has(id)); });
  const tick = () => frames.splice(0).forEach(f => f());
  // Recover restores Logos first; the shared page mounts it late.
  queue.enqueue([{id:'logos', generation:1}], ['logos']);
  roots.set('logos', false); queue.refresh(); tick();
  // Before its paint acknowledgement, the workspace activates Flappy Bird.
  queue.enqueue([{id:'bird', generation:2}], ['bird']);
  expect(roots.get('logos')).toBe(false);
  tick(); tick();
  expect(acks).toHaveLength(0);
  // A later DOM batch mounts Bird and touches the old Logos root.
  roots.set('bird', false); roots.set('logos', true);
  queue.refresh(); tick(); tick();
  expect(roots.get('logos')).toBe(false);
  expect(roots.get('bird')).toBe(true);
  expect(acks.map(p => p.id)).toEqual(['bird']);
});

test("the shared page keeps recovered panel bodies and late roots in their own slots", async () => {
  const { createRequire } = await import("node:module");
  const require = createRequire(new URL("../../../js/remote-dom/package.json", import.meta.url));
  const { JSDOM } = require("jsdom");
  const dom = new JSDOM('<body><section data-pm-contribution="logos">Commons Logos</section></body>', {runScripts:'outside-only'});
  const frames: (() => void)[] = [];
  dom.window.requestAnimationFrame = (f: () => void) => frames.push(f);
  dom.window.call = () => {};
  dom.window.eval(readFileSync(new URL('../src/panel-layout.js', import.meta.url), 'utf8'));
  const source = readFileSync(new URL('../src/remote-dom.js', import.meta.url), 'utf8');
  const callbacks = source.slice(source.indexOf('  const panelLayouts ='), source.indexOf("  document.addEventListener('scroll'"));
  dom.window.eval(callbacks.replace('const panelLayouts =', 'globalThis.layouts ='));
  const queue = dom.window.layouts;
  const tick = () => frames.splice(0).forEach(f => f());
  try {
    const rect = {x:20,y:30,w:320,h:240};
    queue.enqueue([{id:'logos',generation:1,rect}], ['logos']);
    tick();
    queue.enqueue([{id:'bird',generation:2,rect}], ['bird']);
    tick(); tick();
    const logos = dom.window.document.querySelector('[data-pm-contribution="logos"]');
    expect(logos.style.visibility).toBe('hidden');
    const bird = dom.window.document.createElement('section');
    bird.setAttribute('data-pm-contribution','bird');
    bird.textContent = 'Flappy Bird';
    dom.window.document.body.append(bird);
    queue.refresh(); tick(); tick();
    expect(logos.style.visibility).toBe('hidden');
    expect(bird.style.visibility).toBe('visible');
    expect(bird.style.left).toBe('20px');
    expect(bird.style.width).toBe('320px');
    expect(bird.style.clipPath).toBe('inset(0)');
    expect([...dom.window.document.querySelectorAll('section')].filter((r: any) => r.style.visibility === 'visible').map((r: any) => r.textContent)).toEqual(['Flappy Bird']);
  } finally { dom.window.close(); }
});
