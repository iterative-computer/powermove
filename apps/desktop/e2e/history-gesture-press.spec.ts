import type { Page } from '@playwright/test';
import { expect, test } from './helpers/app';

/*
 * A gesture's press reads the document as it is after Undo, Redo, or a
 * history jump. Undo restores a layer by replacing its object, at an
 * unchanged composition shape; with the new engine on, the selection's
 * transform roots came from a memo that still held the replaced objects, so
 * a rotate after an undone resize wrote the undone scale back (111.481).
 * Every check here runs with the engine on and off and expects today's values.
 */

type Matrix = [number, number, number, number, number, number];
type Point = { x: number; y: number };

async function ready(page: Page) {
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && PM?.GL?.gl); });
}

/** The skewed-parent scene of viewer-resize.spec.ts: a shape under a skewed parent, a text with a child, and a group. */
async function scene(page: Page) {
  return page.evaluate(async () => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    PM.Kernel.services.get('tool').tool = 'select';
    const project = PM.mkProject({ name: 'Press after history', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
    const parent = PM.mkLayer('null', { name: 'Skewed parent', dur: 4, p: { 'position.x': 320, 'position.y': 180, 'scale.x': 160, 'scale.y': 60, rotation: 10, skew: 25 } }, project);
    const shape = PM.mkLayer('shape', {
      name: 'Shape', dur: 4,
      d: { shape: 'rect', color: '#FF6B1A', w: 120, h: 80, radius: 8, stroke: 0, strokeColor: '#000000', points: 5 },
      p: { 'position.x': -90, 'position.y': 0 },
    }, project);
    shape.parent = parent.id;
    const text = PM.mkLayer('text', {
      name: 'Text', dur: 4,
      d: { text: 'TOGETHER', font: 'Geist', weight: 700, size: 54, tracking: -1, leading: 1, color: '#FFFFFF', align: 'center' },
      p: { 'position.x': 450, 'position.y': 180 },
    }, project);
    const textChild = PM.mkLayer('shape', {
      name: 'Text child', dur: 4,
      d: { shape: 'rect', color: '#4C8DFF', w: 44, h: 44, radius: 6, stroke: 0, strokeColor: '#000000', points: 5 },
      p: { 'position.x': 0, 'position.y': 90 },
    }, project);
    textChild.parent = text.id;
    const a = PM.mkLayer('solid', { name: 'A', dur: 4, d: { color: '#22CC88', w: 50, h: 50 } }, project);
    const b = PM.mkLayer('solid', { name: 'B', dur: 4, d: { color: '#22CC88', w: 50, h: 50 } }, project);
    // Solids take their size from `d`; place them explicitly.
    for (const [layer, x] of [[a, 90], [b, 160]] as const) { layer.p['position.x'].v = x; layer.p['position.y'].v = 290; }
    project.layers = [b, a, textChild, text, shape, parent];
    PM.replaceProject(project);
    PM.setKey(shape, 'scale.x', 0, 100, 'linear');
    PM.setKey(shape, 'scale.y', 0, 100, 'linear');
    PM.setKey(text, 'scale.x', 0, 100, 'linear');
    PM.setKey(text, 'scale.y', 0, 100, 'linear');
    PM.groupLayers([a.id, b.id], 'Group');
    const group = PM.proj.layers.find((layer: any) => layer.type === 'group');
    PM.setTime(1, { raw: true, force: true });
    PM.hist.clear?.();
    viewer.layout();
    await document.fonts.ready;
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    return { shape: shape.id, text: text.id, textChild: textChild.id, parent: parent.id, group: group.id, a: a.id, b: b.id };
  });
}

/** The document as the legacy reads of the live layers see it (today's ground truth). */
async function state(page: Page, ids: string[]) {
  return page.evaluate((ids) => {
    const PM = (window as any).PM;
    return Object.fromEntries(ids.map((id) => {
      const L = PM.L(id);
      const content = PM.resolveContent(L, 1);
      return [id, {
        world: [...PM.worldMatrix(L, 1)],
        values: ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'skew', 'anchor.x', 'anchor.y'].map((key) => PM.ev(L, key, 1)),
        content: { size: content.size ?? null, w: content.w ?? null, h: content.h ?? null },
        keys: ['scale.x', 'scale.y', 'rotation', 'position.x'].map((key) => L.p[key]?.kf?.length ?? 0),
      }];
    }));
  }, ids) as Promise<Record<string, { world: Matrix; values: number[]; content: Record<string, number | null>; keys: number[] }>>;
}

async function select(page: Page, ids: string[], tool = 'select') {
  await page.evaluate(async ({ ids, tool }) => {
    const PM = (window as any).PM;
    PM.Kernel.services.get('tool').tool = tool;
    PM.selectLayers(ids);
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  }, { ids, tool });
}

/** Composition points of the current selection's geometry, in page pixels. */
async function geometry(page: Page) {
  const box = await page.locator('#stage-inner').boundingBox();
  if (!box) throw new Error('viewer frame is unavailable');
  const g = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const selection = viewer.resolveSelectionGeometry();
    return { shown: viewer.shown, mode: selection.mode, handles: selection.handles, pivot: selection.pivotWorld, corners: selection.corners };
  });
  const page_ = (p: Point) => ({ x: box.x + p.x * g.shown, y: box.y + p.y * g.shown });
  return { ...g, page: page_ };
}

async function drag(page: Page, from: Point, to: Point, modifier?: 'Meta') {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.move(to.x, to.y, { steps: 8 });
  if (modifier) await page.keyboard.up(modifier);
  await page.mouse.up();
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
}

async function resizeSouthEast(page: Page, dx: number, dy: number) {
  const g = await geometry(page);
  const se = g.page(g.handles.se);
  await drag(page, se, { x: se.x + dx, y: se.y + dy });
}

/** Rotates the selection by +45° with the rotation handle (Command snaps to 45°). */
async function rotate45(page: Page) {
  const g = await geometry(page);
  const center = g.page(g.pivot);
  const north = g.page(g.handles.n);
  const start = { x: north.x, y: north.y - 22 };
  const radius = Math.hypot(start.x - center.x, start.y - center.y);
  await drag(page, start, { x: center.x + radius * Math.SQRT1_2, y: center.y - radius * Math.SQRT1_2 }, 'Meta');
}

const undo = (page: Page, n = 1) => page.evaluate((n) => { const PM = (window as any).PM; let ok = true; for (let i = 0; i < n; i++) ok = PM.hist.undo() && ok; return ok; }, n);
const redo = (page: Page, n = 1) => page.evaluate((n) => { const PM = (window as any).PM; let ok = true; for (let i = 0; i < n; i++) ok = PM.hist.redo() && ok; return ok; }, n);

const rotated45 = (m: Matrix) => [
  (m[0] - m[1]) * Math.SQRT1_2, (m[0] + m[1]) * Math.SQRT1_2,
  (m[2] - m[3]) * Math.SQRT1_2, (m[2] + m[3]) * Math.SQRT1_2,
];

function expectClose(actual: number[], expected: number[], digits = 3) {
  expect(actual).toHaveLength(expected.length);
  actual.forEach((value, i) => expect(value, `component ${i}`).toBeCloseTo(expected[i]!, digits));
}

function expectSameState(actual: Awaited<ReturnType<typeof state>>, expected: Awaited<ReturnType<typeof state>>) {
  for (const id of Object.keys(expected)) {
    expectClose(actual[id]!.world, expected[id]!.world, 3);
    expectClose(actual[id]!.values, expected[id]!.values, 3);
    expect(actual[id]!.content).toEqual(expected[id]!.content);
    expect(actual[id]!.keys).toEqual(expected[id]!.keys);
  }
}

/** The rotate wrote a rigid +45° turn of `before` (axes rotated, scale and content kept). */
function expectRigidTurn(after: Awaited<ReturnType<typeof state>>, before: Awaited<ReturnType<typeof state>>, ids: string[]) {
  for (const id of ids) {
    expectClose(after[id]!.world.slice(0, 4), rotated45(before[id]!.world), 3);
    expect(after[id]!.content).toEqual(before[id]!.content);
  }
}

test.describe('@viewer a press after history reads the current document', () => {
  test('resize a multi-selection, undo, then rotate: the rotate starts from the restored layers', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await ready(page);
    const ids = await scene(page);
    const all = [ids.shape, ids.text, ids.textChild, ids.parent];
    await select(page, [ids.shape, ids.text, ids.textChild]);
    expect((await geometry(page)).mode).toBe('common');
    const s0 = await state(page, all);

    await resizeSouthEast(page, 72, 48);
    const s1 = await state(page, all);
    expect(s1[ids.text]!.values[2]).toBeGreaterThan(s0[ids.text]!.values[2]!);
    expect(s1[ids.shape]!.content.w).toBeGreaterThan(s0[ids.shape]!.content.w!);

    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);

    await rotate45(page);
    const s2 = await state(page, all);
    expectRigidTurn(s2, s0, [ids.shape, ids.text, ids.textChild]);
    // The text has no parent: Scale X/Y stay exactly as restored (never 111.481).
    expect(s2[ids.text]!.values.slice(2, 4)).toEqual(s0[ids.text]!.values.slice(2, 4));
    expect(s2[ids.text]!.values[4]).toBe(45);
    expect(s2[ids.parent]).toEqual(s0[ids.parent]);

    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('resize, undo, redo, then rotate: the rotate starts from the redone layers', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await ready(page);
    const ids = await scene(page);
    const all = [ids.shape, ids.text, ids.textChild, ids.parent];
    await select(page, [ids.shape, ids.text, ids.textChild]);
    const s0 = await state(page, all);
    await resizeSouthEast(page, 60, 40);
    const s1 = await state(page, all);
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(await redo(page)).toBe(true);
    expectSameState(await state(page, all), s1);

    await rotate45(page);
    const s2 = await state(page, all);
    expectRigidTurn(s2, s1, [ids.shape, ids.text, ids.textChild]);
    expect(s2[ids.text]!.values.slice(2, 4)).toEqual(s1[ids.text]!.values.slice(2, 4));

    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s1);
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('two resizes undone in one jump, then a move: the move starts from the original layers', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await ready(page);
    const ids = await scene(page);
    const all = [ids.shape, ids.text, ids.textChild, ids.parent];
    await select(page, [ids.shape, ids.text, ids.textChild]);
    const s0 = await state(page, all);
    await resizeSouthEast(page, 50, 30);
    await resizeSouthEast(page, 40, 30);
    expect(await undo(page, 2)).toBe(true);
    expectSameState(await state(page, all), s0);

    const g = await geometry(page);
    const from = g.page(g.pivot);
    // Command bypasses snapping, so the delta is exactly the pointer's.
    await drag(page, from, { x: from.x + 40, y: from.y + 30 }, 'Meta');
    const s1 = await state(page, all);
    for (const id of [ids.shape, ids.text, ids.textChild]) {
      expectClose(s1[id]!.world.slice(0, 4), s0[id]!.world.slice(0, 4), 6);
      expectClose(s1[id]!.world.slice(4), [s0[id]!.world[4] + 40 / g.shown, s0[id]!.world[5] + 30 / g.shown], 1);
      expect(s1[id]!.content).toEqual(s0[id]!.content);
    }
    expect(s1[ids.text]!.values.slice(2)).toEqual(s0[ids.text]!.values.slice(2));
    expect(s1[ids.shape]!.values.slice(2)).toEqual(s0[ids.shape]!.values.slice(2));
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a group root and a layer: resize, undo, rotate', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await ready(page);
    const ids = await scene(page);
    const all = [ids.group, ids.a, ids.b, ids.text];
    await select(page, [ids.group, ids.text]);
    expect((await geometry(page)).mode).toBe('common');
    const s0 = await state(page, all);
    await resizeSouthEast(page, 60, 40);
    expect((await state(page, all))[ids.group]!.values[2]).not.toBe(s0[ids.group]!.values[2]);
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    await rotate45(page);
    const s1 = await state(page, all);
    expectRigidTurn(s1, s0, [ids.a, ids.b, ids.text]);
    expect(s1[ids.group]!.values.slice(2, 4)).toEqual(s0[ids.group]!.values.slice(2, 4));
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('one layer: resize, undo, then the rotate tool and the anchor tool', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await ready(page);
    const ids = await scene(page);
    const all = [ids.shape, ids.parent];
    await select(page, [ids.shape]);
    expect((await geometry(page)).mode).toBe('single');
    const s0 = await state(page, all);
    await resizeSouthEast(page, 40, 30);
    expect((await state(page, all))[ids.shape]!.content.w).not.toBe(s0[ids.shape]!.content.w);
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);

    // The rotate tool: press inside the selection, turn about the pivot.
    await select(page, [ids.shape], 'rotate');
    let g = await geometry(page);
    const pivot = g.page(g.pivot), corner = g.page(g.corners[1]);
    const start = { x: (pivot.x + corner.x) / 2, y: (pivot.y + corner.y) / 2 };
    const angle = Math.atan2(start.y - pivot.y, start.x - pivot.x) + Math.PI / 6;
    const radius = Math.hypot(start.x - pivot.x, start.y - pivot.y);
    await drag(page, start, { x: pivot.x + radius * Math.cos(angle), y: pivot.y + radius * Math.sin(angle) });
    const s1 = await state(page, all);
    expect(s1[ids.shape]!.values[4]).not.toBe(s0[ids.shape]!.values[4]);
    expect(s1[ids.shape]!.values.slice(2, 4)).toEqual(s0[ids.shape]!.values.slice(2, 4));
    expect(s1[ids.shape]!.content).toEqual(s0[ids.shape]!.content);
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);

    // The anchor tool: the anchor moves, the layer stays where it is.
    await select(page, [ids.shape], 'anchor');
    g = await geometry(page);
    const from = g.page(g.pivot);
    await drag(page, from, { x: from.x + 30, y: from.y + 12 });
    const s2 = await state(page, all);
    expect(s2[ids.shape]!.values.slice(6)).not.toEqual(s0[ids.shape]!.values.slice(6));
    expect(s2[ids.shape]!.values.slice(2, 6)).toEqual(s0[ids.shape]!.values.slice(2, 6));
    expect(s2[ids.shape]!.content).toEqual(s0[ids.shape]!.content);
    const g2 = await geometry(page);
    g2.corners.forEach((p: Point, i: number) => expectClose([p.x, p.y], [g.corners[i].x, g.corners[i].y], 2));
    expect(await undo(page)).toBe(true);
    expectSameState(await state(page, all), s0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('drag a keyframe, undo, then drag it again from its restored time', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; return !!PM.Kernel.services.get('timeline')?.cv; });
    const before = await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Keyframe press after undo', dur: 6 }));
      const L = PM.mkLayer('text', { dur: 6 });
      PM.proj.layers.push(L); PM.selectLayers(L.id);
      for (const [time, value] of [[1, 100], [4, 300]]) PM.Edit.apply({ type: 'set_property', target: L.id, path: 'position.y', time, value, mode: 'keyframe', preserveHandEdits: false });
      PM.UIState.setLayerCollapsed(L, false); PM.bus.emit('layers'); PM.setTime(0);
      return JSON.stringify(PM.findProp(L, 'position.y').kf);
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').rows.some((r: any) => r.key === 'position.y'))).toBe(true);
    const keyPoint = (t: number) => page.evaluate((t) => {
      const timeline = (window as any).PM.Kernel.services.get('timeline');
      const rect = timeline.cv.getBoundingClientRect();
      const i = timeline.rows.findIndex((r: any) => r.key === 'position.y');
      return { x: rect.x + timeline.gut + (t - timeline.scrollT) * timeline.pps, y: rect.y + timeline.ruler + i * timeline.row - timeline.scrollY + timeline.row / 2, pps: timeline.pps };
    }, t);
    const keys = () => page.evaluate(() => { const PM = (window as any).PM; return PM.findProp(PM.proj.layers[0], 'position.y').kf.map((k: any) => [k.t, k.v]); });
    let point = await keyPoint(1);
    await drag(page, point, { x: point.x + point.pps, y: point.y });
    await expect.poll(async () => (await keys())[0]).toEqual([2, 100]);
    expect(await undo(page)).toBe(true);
    expect(JSON.stringify(await page.evaluate(() => { const PM = (window as any).PM; return PM.findProp(PM.proj.layers[0], 'position.y').kf; }))).toBe(before);
    point = await keyPoint(1);
    await drag(page, point, { x: point.x + 2 * point.pps, y: point.y });
    await expect.poll(keys).toEqual([[3, 100], [4, 300]]);
    expect(await undo(page)).toBe(true);
    expect(await keys()).toEqual([[1, 100], [4, 300]]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
