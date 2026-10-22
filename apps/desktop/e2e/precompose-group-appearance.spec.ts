import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
});

test('partial precomposition preserves overlapping group alpha, animated rigs and Undo', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 320, h: 180, fps: 30, dur: 3 });
    const group = PM.mkLayer('group', { p: { opacity: 50 } }, project);
    group.p.opacity.kf = [PM.KF(0, 20), PM.KF(3, 80)];
    const parent = PM.mkLayer('null', {}, project);
    parent.group = group.id;
    parent.p['position.x'].kf = [PM.KF(0, 0), PM.KF(3, 12)];
    const first = PM.mkLayer('shape', { from: .3, dur: 2.7, d: { shape: 'rect', w: 100, h: 80, color: '#ffffff', stroke: 0 }, p: { 'position.x': 160, 'position.y': 90 } }, project);
    const second = PM.cloneLayer(first);
    first.parent = parent.id; first.group = second.group = group.id;
    project.layers = [group, first, second, parent]; PM.replaceProject(project);
    const times = [.5, 1.5, 2.5];
    const render = (time: number) => PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false });
    const before = times.map(render);
    const difference = () => times.map((time, frame) => {
      const after = render(time); let max = 0;
      for (let index = 0; index < after.length; index++) max = Math.max(max, Math.abs(after[index] - before[frame][index]));
      return max;
    });
    const alpha = before.map((pixels: Uint8Array) => pixels[(90 * 320 + 160) * 4 + 3]);
    const id = PM.Comps.precompose([first.id], { adjustDuration: true });
    const after = difference();
    const grouped = PM.proj.layers.find((layer: any) => layer.type === 'precomp').group === group.id;
    const undo = PM.hist.undo(), restored = difference();
    const redo = PM.hist.redo(), redone = difference();
    return { alpha, after, grouped, undo, restored, redo, redone, duration: PM.proj.comps[id].dur };
  });
  expect(result.alpha[0]).toBeGreaterThan(60); expect(result.alpha[0]).toBeLessThan(90);
  expect(result.alpha[1]).toBeGreaterThan(115); expect(result.alpha[1]).toBeLessThan(140);
  expect(result.alpha[2]).toBeGreaterThan(165); expect(result.alpha[2]).toBeLessThan(195);
  expect(result.grouped).toBe(true); expect(result.undo).toBe(true); expect(result.redo).toBe(true);
  expect(result.duration).toBeCloseTo(2.7);
  [...result.after, ...result.restored, ...result.redone].forEach(delta => expect(delta).toBeLessThanOrEqual(2));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('nested shared blur, mask and blend remain on their original group boundary', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, project = PM.mkProject({ w: 320, h: 180, dur: 3 });
    const outer = PM.mkLayer('group', { p: { opacity: 80 } }, project);
    const inner = PM.mkLayer('group', { p: { opacity: 50 } }, project); inner.group = outer.id;
    // A nonzero anchor and equal position still form an identity coordinate space.
    inner.p['anchor.x'].v = inner.p['position.x'].v = 160;
    inner.p['anchor.y'].v = inner.p['position.y'].v = 90;
    inner.blend = 'multiply';
    const blur = PM.mkEffect('blur'); blur.p.amount.v = 4; inner.fx.push(blur);
    const mask = PM.mkMask('rect', project);
    mask.p.x.v = 160; mask.p.y.v = 90; mask.p.w.v = 120; mask.p.h.v = 90;
    inner.masks.push(mask);
    const first = PM.mkLayer('shape', { d: { shape: 'rect', w: 100, h: 80, color: '#ff8040', stroke: 0 }, p: { 'position.x': 145, 'position.y': 90 } }, project);
    const second = PM.mkLayer('shape', { d: { shape: 'rect', w: 100, h: 80, color: '#40a0ff', stroke: 0 }, p: { 'position.x': 175, 'position.y': 90 } }, project);
    first.group = second.group = inner.id;
    const background = PM.mkLayer('shape', { d: { shape: 'rect', w: 320, h: 180, color: '#c0c0c0', stroke: 0 }, p: { 'position.x': 160, 'position.y': 90 } }, project);
    project.layers = [outer, inner, first, second, background]; PM.replaceProject(project);
    const render = () => PM.GL.renderToPixels(1, 320, 180, { transparent: true, mblur: false });
    const before = render(), center = Array.from(before.slice((90 * 320 + 160) * 4, (90 * 320 + 160) * 4 + 4));
    const difference = () => {
      const after = render(); let max = 0;
      for (let index = 0; index < after.length; index++) max = Math.max(max, Math.abs(after[index] - before[index]));
      return max;
    };
    PM.Comps.precompose([first.id]);
    const after = difference(), undo = PM.hist.undo(), restored = difference();
    return { center, after, undo, restored };
  });
  // Confirm the fixture exercises rendered color, not an empty transparent image.
  expect(result.center[3]).toBe(255); expect(result.center.slice(0, 3)).not.toEqual([192, 192, 192]);
  expect(result.after).toBeLessThanOrEqual(2); expect(result.undo).toBe(true); expect(result.restored).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('offcanvas transformed styled groups reject partial moves and preserve whole-group pixels', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, project = PM.mkProject({ w: 320, h: 180, dur: 3 });
    const group = PM.mkLayer('group', { p: { 'position.x': -900, opacity: 50 } }, project);
    const first = PM.mkLayer('shape', { d: { shape: 'rect', w: 100, h: 80, color: '#ffffff', stroke: 0 }, p: { 'position.x': 1060, 'position.y': 90 } }, project);
    const second = PM.cloneLayer(first); first.group = second.group = group.id;
    project.layers = [group, first, second]; PM.replaceProject(project);
    const render = () => PM.GL.renderToPixels(1, 320, 180, { transparent: true, mblur: false });
    const before = render(), state = PM.serialize(), history = PM.hist.list().length;
    let error = '';
    try { PM.Comps.precompose([first.id]); } catch (failure) { error = String(failure); }
    const rejectedUnchanged = state === PM.serialize() && history === PM.hist.list().length;
    const difference = () => {
      const after = render(); let max = 0;
      for (let index = 0; index < after.length; index++) max = Math.max(max, Math.abs(after[index] - before[index]));
      return max;
    };
    const partial = difference();
    PM.Comps.precompose([group.id]);
    const whole = difference(), undo = PM.hist.undo(), restored = difference();
    return { alpha: before[(90 * 320 + 160) * 4 + 3], error, rejectedUnchanged, partial, whole, undo, restored };
  });
  expect(result.alpha).toBeGreaterThan(115); expect(result.alpha).toBeLessThan(140);
  expect(result.error).toContain('Select the whole styled group'); expect(result.rejectedUnchanged).toBe(true);
  expect(result.partial).toBe(0); expect(result.whole).toBeLessThanOrEqual(2);
  expect(result.undo).toBe(true); expect(result.restored).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
