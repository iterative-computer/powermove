import { chooseNativeMenu, expect, test } from './helpers/app';

/*
 * The geometry-dependent edits pm-edit answers behind the native evaluator
 * flag (docs/engine/pm-edit.md): the alignment strip, reparenting, grouping and
 * an animated ungroup, the size anchor's Pan Behind, the font anchor, and split
 * text. Each must leave the world where it was, whatever the flag; with the
 * flag on, the kernels must have answered (the only fallback allowed is glyph
 * ink, which needs outlines the Phase 1 font registry does not serve), and in
 * the dual modes the TypeScript edits must agree with them. A second test
 * covers move to group and Pre-compose › Leave all attributes. Fonts need
 * POWERMOVE_ENGINE_FONTS=1 and the pm-bridge addon.
 */

type Stats = { calls: Record<string, number>; applied: Record<string, number>; fallbacks: Record<string, number> };

const close = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]!) <= 1e-6 * (1 + Math.abs(b[i]!)));


/**
 * The e2e build bakes `VITE_POWERMOVE_NATIVE_EVALUATOR` from this process's
 * environment (global setup builds with it), so the page must report that
 * mode. Use New Engine turns a default `off` build into `on` so its document
 * mirror and native edits can serve the interactive renderer.
 */
function expectBuiltMode(report: { mode: string } | null, newEngine: boolean): void {
  const raw = (process.env.VITE_POWERMOVE_NATIVE_EVALUATOR ?? 'off').trim().toLowerCase();
  const configured = raw === '1' || raw === 'true' || raw === 'on' ? 'on'
    : ['audit', 'audit-proxy', 'strict'].includes(raw) ? raw : 'off';
  const want = newEngine && configured === 'off' ? 'on' : configured;
  expect(report?.mode ?? 'off', 'native evaluator mode of the build').toBe(want);
}
test('align, reparent, group, ungroup, anchors and split text through pm-edit keep the artwork in place', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('viewer')));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'pm-edit', w: 1280, h: 720, dur: 2, fps: 29.97 });
    const rig = PM.mkLayer('null', { name: 'Rig', p: { 'position.x': 400, 'position.y': 300, rotation: 30, 'scale.x': 150 } }, project);
    rig.p.rotation.kf = [PM.KF(0, 0, 'linear'), PM.KF(1, 90, 'easeOut')];
    const card = PM.mkLayer('shape', { name: 'Card', d: { w: 120, h: 80, color: '#EF683B' }, p: { 'position.x': 220, 'position.y': 140 } }, project);
    const badge = PM.mkLayer('shape', { name: 'Badge', d: { w: 60, h: 60, color: '#4274EC' }, p: { 'position.x': 80, 'position.y': 40 } }, project);
    badge.parent = rig.id;
    const title = PM.mkLayer('text', { name: 'Title', d: { text: 'Hello world', size: 48 }, p: { 'position.x': 640, 'position.y': 500 } }, project);
    project.layers = [title, card, badge, rig];
    PM.replaceProject(project);
    PM.setTime(10 / 29.97, { force: true });
    PM.hist.clear();
  });

  // Fonts for the text kernels arrive in the background after the first
  // text question; wait until pm-wasm lays the title out natively.
  await page.waitForFunction(() => {
    const PM = (window as any).PM, doc = PM.EngineDoc;
    if (!doc?.enabled) return true;
    if (!doc.ready) return false;
    doc.barrier?.flush();
    const ask = () => doc.wasm?.query({ op: 'textLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: PM.time });
    let reply = ask();
    if (reply?.fonts) reply = doc.api?.fonts?.resolve(doc.wasm, ask, (r: any) => !!r?.fonts) ?? reply;
    return !!reply && 'ok' in reply;
  }, null, { timeout: 20_000, polling: 250 });

  const world = (name: string, t?: number) => page.evaluate(([name, t]) => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.name === name);
    return PM.worldMatrix(layer, t ?? PM.time) as number[];
  }, [name, t] as const);
  const box = () => page.evaluate(() => {
    const PM = (window as any).PM;
    const corners = ['Card', 'Badge'].flatMap(name => {
      const layer = PM.proj.layers.find((l: any) => l.name === name), b = PM.GL.bounds(layer, PM.time), m = PM.worldMatrix(layer, PM.time);
      return [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]].map(([x, y]) => m[0] * x + m[2] * y + m[4]);
    });
    return Math.min(...corners);
  });

  // Align left through the strip: the selection's common box meets x = 0.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.selectLayers(PM.proj.layers.filter((l: any) => l.name === 'Card' || l.name === 'Badge').map((l: any) => l.id));
  });
  await page.getByRole('button', { name: 'Align left', exact: true }).click();
  await expect.poll(box).toBeLessThan(0.002);
  expect(await box()).toBeGreaterThan(-0.002);

  // Unparent the badge: its world pose stays.
  const badgeBefore = await world('Badge');
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, badge = PM.proj.layers.find((l: any) => l.name === 'Badge');
    return PM.Edit.apply({ type: 'set_layer', target: badge.id, patch: { parent: null } }).ok;
  })).toBe(true);
  expect(close(await world('Badge'), badgeBefore)).toBe(true);

  // Group card and rig (pivot at the members' center), rotate and fade the
  // group, then ungroup: the rig is animated, so the members are baked and
  // keep their world pose at every frame.
  const frames = [0, 7, 15, 29, 45];
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, ids = PM.proj.layers.filter((l: any) => l.name === 'Card' || l.name === 'Rig').map((l: any) => l.id);
    const ok = PM.Edit.apply({ type: 'group_layers', targets: ids, name: 'Pair' }).ok;
    const group = PM.proj.layers.find((l: any) => l.name === 'Pair');
    PM.Edit.apply([{ type: 'set_property', target: group.id, path: 'rotation', value: 25, time: PM.time, mode: 'static' },
      { type: 'set_property', target: group.id, path: 'opacity', value: 60, time: PM.time, mode: 'static' }]);
    return ok;
  })).toBe(true);
  const rigBefore = await Promise.all(frames.map(n => world('Rig', n / 29.97)));
  const cardBefore = await Promise.all(frames.map(n => world('Card', n / 29.97)));
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, group = PM.proj.layers.find((l: any) => l.name === 'Pair');
    return PM.Edit.apply({ type: 'ungroup_layers', targets: [group.id] }).ok;
  })).toBe(true);
  for (const [i, n] of frames.entries()) {
    expect(close(await world('Rig', n / 29.97), rigBefore[i]!), `Rig at frame ${n}`).toBe(true);
    expect(close(await world('Card', n / 29.97), cardBefore[i]!), `Card at frame ${n}`).toBe(true);
  }
  // Baked key times sit on the document's frame grid.
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, rig = PM.proj.layers.find((l: any) => l.name === 'Rig');
    return rig.p['position.x'].kf.filter((k: any) => Math.abs(k.t * 29.97 - Math.round(k.t * 29.97)) < 1e-9)
      .every((k: any) => k.t === Math.round(k.t * 29.97) / 29.97);
  })).toBe(true);

  // Size anchor: a resize records the bounds; Pan Behind then moves only the
  // pivot, so the artwork keeps its box in the layer's own space (without the
  // rescale the widened card would slide by the anchor's move times 60/120).
  const cardBox = () => page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.name === 'Card'), b = PM.GL.bounds(layer, PM.time);
    return { ref: layer.d.sizeAnchorBounds, box: [b.x0, b.y0, b.x1, b.y1] as number[] };
  });
  await page.evaluate(() => {
    const PM = (window as any).PM, card = PM.proj.layers.find((l: any) => l.name === 'Card');
    PM.Edit.apply({ type: 'set_content', target: card.id, patch: { w: 180 }, time: PM.time });
  });
  const resized = await cardBox();
  expect(resized.ref).toBeTruthy();
  await page.evaluate(() => {
    const PM = (window as any).PM, card = PM.proj.layers.find((l: any) => l.name === 'Card');
    PM.Edit.apply({ type: 'set_property', target: card.id, path: 'anchor.x', value: 30, time: PM.time, mode: 'static' });
  });
  const panned = await cardBox();
  expect(panned.ref).not.toEqual(resized.ref);
  expect(close(panned.box, resized.box), `artwork box ${panned.box} after Pan Behind, ${resized.box} before`).toBe(true);

  // Font anchor: a size change records the title's bounds.
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, title = PM.proj.layers.find((l: any) => l.name === 'Title');
    PM.Edit.apply({ type: 'set_content', target: title.id, patch: { size: 64 }, time: PM.time });
    return !!title.d.fontAnchorBounds;
  })).toBe(true);

  // Split the title by word through the layer menu.
  await page.evaluate(() => { const PM = (window as any).PM; PM.selectLayers(PM.proj.layers.find((l: any) => l.name === 'Title').id); });
  const point = await page.evaluate(() => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const layer = PM.proj.layers.find((l: any) => l.name === 'Title');
    const glyph = PM.textLayout(layer).characters[0], m = PM.worldMatrix(layer, PM.time);
    const x = glyph.x + 10, y = glyph.y + 20, rect = document.querySelector('#stage-inner')!.getBoundingClientRect();
    return { x: rect.x + (m[0] * x + m[2] * y + m[4]) * viewer.shown, y: rect.y + (m[1] * x + m[3] * y + m[5]) * viewer.shown };
  });
  await chooseNativeMenu(session, ['Split text into layers…', 'By word'], () => page.mouse.click(point.x, point.y, { button: 'right' }));
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, group = PM.firstSel();
    return PM.proj.layers.filter((l: any) => l.group === group.id && l.type === 'text' && l.on !== false && !(l.on?.v === false)).map((l: any) => l.d.text);
  })).toEqual(['Hello', 'world']);

  // Undo walks every edit back.
  await page.evaluate(() => { const PM = (window as any).PM; while (PM.hist.undo()) { /* to the start */ } });
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.name))).toEqual(['Title', 'Card', 'Badge', 'Rig']);

  const report = await page.evaluate(() => {
    const doc = (window as any).PM.EngineDoc, stats = doc?.stats?.();
    return stats?.enabled === false || !stats ? null : { mode: stats.mode as string, edits: stats.edits as Stats, dual: stats.dual };
  });
  expectBuiltMode(report, await page.evaluate(() => !!(window as any).PM.EngineConnection));
  if (report) {
    const kernels = ['align', 'reparent', 'groupPlacement', 'ungroup', 'captureSizeAnchor', 'anchorReference', 'anchorMoved', 'captureFontAnchor', 'splitText'];
    for (const name of kernels) expect(report.edits.calls[name] ?? 0, `${name} asked`).toBeGreaterThan(0);
    // Glyph ink (a text layer's raster bounds and font offset) needs outlines,
    // which the Phase 1 font registry does not serve: those questions run the
    // TypeScript edit. Nothing else may fall back.
    const fallbacks = Object.keys(report.edits.fallbacks);
    expect(fallbacks.filter(key => !key.endsWith(':ink')), 'fallbacks other than glyph ink').toEqual([]);
    const ink = new Set(fallbacks.map(key => key.split(':')[0]));
    if (report.mode === 'on') {
      for (const name of kernels) {
        const applied = report.edits.applied[name] ?? 0;
        if (ink.has(name)) expect(applied + (report.edits.fallbacks[`${name}:ink`] ?? 0), `${name} answered`).toBe(report.edits.calls[name]);
        else expect(applied, `${name} applied`).toBe(report.edits.calls[name]);
      }
    }
    if (report.dual) {
      const edits = Object.keys(report.dual.compared ?? {}).filter(op => op.startsWith('edit.'));
      expect(edits.length).toBeGreaterThan(0);
      expect(Object.keys(report.dual.mismatched ?? {}).filter(op => op.startsWith('edit.'))).toEqual([]);
    }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('move to group and Leave all attributes through pm-edit keep timing and artwork', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services.get('viewer')));
  // The old peer fixture used scene3d.createMaterial/setMaterial, which are
  // absent from the released Scene3DAPI. The remaining edit kernels still
  // exercise undoable geometry and pixel preservation through the public API.
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'pm-edit grouping', w: 640, h: 360, dur: 4, fps: 24, bg: '#FFFFFF' });
    const card = PM.mkLayer('shape', { name: 'Card', d: { w: 90, h: 60, color: '#EF683B' }, p: { 'position.x': 470, 'position.y': 250, rotation: -20, 'scale.x': 130 } }, project);
    const holder = PM.mkLayer('shape', { name: 'Holder', d: { w: 50, h: 50, color: '#4274EC' }, p: { 'position.x': 160, 'position.y': 120 } }, project);
    const plate = PM.mkLayer('solid', { name: 'Plate', from: 0.3, dur: 3, d: { w: 160, h: 90, color: '#386BDC' }, p: { 'position.x': 300, 'position.y': 200, rotation: 15, 'anchor.x': 20 } }, project);
    project.layers = [card, holder, plate];
    PM.replaceProject(project);
    PM.setTime(1, { force: true });
    PM.hist.clear();
    return PM.proj.layers.map((layer: any) => layer.name);
  })).toEqual(['Card', 'Holder', 'Plate']);

  // Move the card into a rotated, scaled group and back out: it keeps its world pose.
  const world = (name: string) => page.evaluate(name => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.name === name);
    return PM.worldMatrix(layer, PM.time) as number[];
  }, name);
  const cardBefore = await world('Card');
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, holder = PM.proj.layers.find((l: any) => l.name === 'Holder');
    const grouped = PM.Edit.apply({ type: 'group_layers', targets: [holder.id], name: 'Box' }).ok;
    const group = PM.proj.layers.find((l: any) => l.name === 'Box');
    PM.Edit.apply([{ type: 'set_property', target: group.id, path: 'rotation', value: 35, time: PM.time, mode: 'static' },
      { type: 'set_property', target: group.id, path: 'scale.x', value: 70, time: PM.time, mode: 'static' }]);
    const card = PM.proj.layers.find((l: any) => l.name === 'Card');
    return grouped && PM.Edit.apply({ type: 'move_to_group', targets: [card.id], group: group.id }).ok && card.group === group.id;
  })).toBe(true);
  expect(close(await world('Card'), cardBefore), 'Card inside the group').toBe(true);
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, card = PM.proj.layers.find((l: any) => l.name === 'Card');
    return PM.Edit.apply({ type: 'move_to_group', targets: [card.id], group: null }).ok && !card.group;
  })).toBe(true);
  expect(close(await world('Card'), cardBefore), 'Card out of the group').toBe(true);

  // Pre-compose › Leave all attributes: the plate's pixels stay.
  const leave = await page.evaluate(() => {
    const PM = (window as any).PM, plate = PM.proj.layers.find((l: any) => l.name === 'Plate');
    const pixels = () => Array.from(PM.renderFrameTo(1.25, 320, 180).getContext('2d').getImageData(0, 0, 320, 180).data) as number[];
    const before = pixels();
    const left = PM.Comps.precompose([plate.id], { name: 'Plate Comp', mode: 'leave' });
    const after = pixels();
    return { left: !!left, type: PM.proj.layers.find((l: any) => l.name === 'Plate').type, same: before.every((v, i) => Math.abs(v - after[i]!) <= 2) };
  });
  expect(leave).toEqual({ left: true, type: 'precomp', same: true });

  await page.evaluate(() => { const PM = (window as any).PM; while (PM.hist.undo()) { /* to the start */ } });
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.name))).toEqual(['Card', 'Holder', 'Plate']);

  const report = await page.evaluate(() => {
    const doc = (window as any).PM.EngineDoc, stats = doc?.stats?.();
    return stats?.enabled === false || !stats ? null : { mode: stats.mode as string, edits: stats.edits as Stats, dual: stats.dual };
  });
  expectBuiltMode(report, await page.evaluate(() => !!(window as any).PM.EngineConnection));
  if (report) {
    const kernels = ['moveToGroup', 'leaveAttributes'];
    for (const name of kernels) expect(report.edits.calls[name] ?? 0, `${name} asked`).toBeGreaterThan(0);
    expect(Object.keys(report.edits.fallbacks), 'fallbacks').toEqual([]);
    if (report.mode === 'on') for (const name of kernels) expect(report.edits.applied[name] ?? 0, `${name} applied`).toBe(report.edits.calls[name]);
    if (report.dual) {
      for (const name of kernels) expect(report.dual.compared?.[`edit.${name}`] ?? 0, `edit.${name} compared`).toBeGreaterThan(0);
      expect(Object.keys(report.dual.mismatched ?? {}).filter(op => op.startsWith('edit.'))).toEqual([]);
    }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
