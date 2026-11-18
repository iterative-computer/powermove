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

test("a realm batch cannot move acknowledged panels into each other's slots", () => {
  const context: any = {};
  runInNewContext(readFileSync(new URL("../src/panel-layout.js", import.meta.url), "utf8"), context);
  const frames: (() => void)[] = [];
  const positions = new Map<string, number>();
  const queue = new context.PMPanelLayout(
    (frame: () => void) => frames.push(frame),
    (placement: { id: string; y: number }) => positions.set(placement.id, placement.y),
    () => {},
  );
  const tick = () => frames.splice(0).forEach(frame => frame());
  queue.enqueue([
    { id: "pixel-comparison", generation: 1, y: 10 },
    { id: "pexels", generation: 2, y: 190 },
  ]);
  tick();
  tick();

  // A delayed realm batch writes its old inline styles after the host ack.
  positions.set("pixel-comparison", 190);
  positions.set("pexels", 10);
  queue.refresh();
  expect([...positions]).toEqual([["pixel-comparison", 10], ["pexels", 190]]);
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
    const logosBeforeLayout = dom.window.document.querySelector('section');
    queue.refresh(); tick(); tick();
    expect(logosBeforeLayout.style.visibility).toBe('');
    const rect = {x:20,y:30,w:320,h:240};
    queue.enqueue([{id:'logos',generation:1,rect}]);
    tick(); tick();
    expect(logosBeforeLayout.style.visibility).toBe('');
    queue.enqueue([{id:'logos',generation:2,rect}], ['logos']);
    tick();
    queue.enqueue([{id:'bird',generation:3,rect}], ['bird']);
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

test("a delayed clear cannot remove another panel's prepared snapshot", async () => {
  const { createRequire } = await import("node:module");
  const require = createRequire(new URL("../../../js/remote-dom/package.json", import.meta.url));
  const { JSDOM } = require("jsdom");
  const dom = new JSDOM('<body><section data-pm-contribution="pexels">Pexels</section></body>', { runScripts: 'outside-only' });
  const { window } = dom;
  window.requestAnimationFrame = () => {};
  window.call = () => {};
  const source = readFileSync(new URL('../src/remote-dom.js', import.meta.url), 'utf8');
  const apply = source.slice(source.indexOf('  const apply = async (event)'), source.indexOf('    } else if (event.kind === "panel_layout")'));
  window.eval(apply.replace('const apply =', 'globalThis.apply =') + '}};');
  try {
    await window.apply({ kind: 'overscan_prepare', id: 'pexels', generation: 4, rect: { x: 0, y: 0, w: 200, h: 100 } });
    await window.apply({ kind: 'overscan_clear', id: 'pixel-comparison', generation: 3 });
    expect(window.document.querySelector('[data-pm-overscan]').textContent).toBe('Pexels');
  } finally {
    window.close();
  }
}, 20000);
