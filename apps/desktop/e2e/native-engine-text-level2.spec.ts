import cases from '../../../engine/corpus/fixtures/text-level2.json';
import { expect, test } from './helpers/app';
import { expectEngineDrewText } from './helpers/engine-drew';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1', POWERMOVE_ENGINE_FONTS: '1' } } });

test('level 2 corpus draws natively with animated bounds, picks, styles and anchors', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  for (const item of cases) {
    await session.page.evaluate(item => {
      const PM = (window as any).PM;
      const { id, title, samples, d, ...controls } = item;
      const project = PM.mkProject({ name: title, w: 640, h: 360, dur: 3 });
      const layer = PM.mkLayer('text', { name: title, d: { ...d, ...controls },
        p: { 'position.x': 100, 'position.y': 80 } }, project);
      for (const key of ['boxWidth', 'boxHeight']) {
        if (typeof layer.d[key] === 'number') layer.d[key] = PM.P(layer.d[key]);
      }
      layer.compat = 2;
      project.layers = [layer];
      PM.replaceProject(project); PM.touch(); PM.setTime(.75, { force: true }); PM.invalidate();
    }, item);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route)).toBe('engine');
    const captures: string[] = [];
    for (const time of [0, .75, 2.5]) {
      const result = await session.page.evaluate(async time => {
        const PM = (window as any).PM;
        PM.setTime(time, { force: true });
        const shot = await PM.EngineCaptures.snapshotAsync(time, 640);
        const layer = PM.proj.layers[0];
        const before = { ...PM.EngineDoc.stats().native.fallbacks };
        const calls = { ...PM.EngineDoc.stats().native.calls };
        // The same native provider used by the public render facade and
        // Viewer; raw PM.GL/PM.textLayout are the flag-off renderer's internals.
        const api = PM.EngineDoc.api;
        const bounds = api.bounds(layer, time);
        const layout = api.glyphLayout(layer, time);
        const caret = api.textLayout(layer, time);
        const x = 100 + (bounds.x0 + bounds.x1) / 2;
        const y = 80 + (bounds.y0 + bounds.y1) / 2;
        const hit = api.pick(x, y, time);
        const native = PM.EngineDoc.stats().native;
        return { shot, bounds, characters: layout.characters.length, picked: hit?.id === layer.id,
          caretBaselines: caret.lines.map((line: any) => line.baseline),
          fallbacks: Object.entries(native.fallbacks).filter(([key, count]) => Number(count) > Number(before[key] || 0)),
          queries: Object.keys(native.calls).filter(key => native.calls[key] > Number(calls[key] || 0)),
          status: PM.EngineConnection.status() };
      }, time);
      expect(result.shot.startsWith('data:image/'), `${item.id} snapshot`).toBe(true);
      expect(result.characters).toBeGreaterThan(0);
      expect(result.caretBaselines.length).toBeGreaterThan(0);
      expect(result.caretBaselines.every((value: number) => Number.isFinite(value))).toBe(true);
      expect(result.status.route).toBe('engine');
      expect(result.status.failures).toEqual([]);
      expect(result.fallbacks, `${item.id} geometry stays native`).toEqual([]);
      expect(result.queries).toEqual(expect.arrayContaining(['gl.bounds', 'gl.pick', 'glyphLayout', 'textLayout']));
      // The selection is the resting text: it never collapses while
      // animators hide every glyph, so the layer stays pickable.
      expect(result.picked, `${item.id} pick at ${time}`).toBe(true);
      expect(result.bounds.x1 - result.bounds.x0, `${item.id} width at ${time}`).toBeGreaterThan(1);
      expect(result.bounds.y1 - result.bounds.y0, `${item.id} height at ${time}`).toBeGreaterThan(1);
      expect(Object.values(result.bounds).every(v => typeof v !== 'number' || Number.isFinite(v))).toBe(true);
      captures.push(result.shot);
    }
    if (['text-level2/stagger', 'text-level2/line-wave', 'text-level2/variable'].includes(item.id)) {
      expect(new Set(captures).size, `${item.id} animated pixels`).toBeGreaterThan(1);
    }
    await expectEngineDrewText(session);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Pan Behind on font-anchored level 2 text keeps the resting text in place while animators move it', async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Level 2 anchor', w: 640, h: 360, dur: 3 });
    const layer = PM.mkLayer('text', { name: 'Anchored', d: { text: 'Motion type', font: 'Georgia', weight: 400, size: 40, color: '#FFFFFF',
      animators: [{ id: 'anim', mode: 'range', unit: 'characters', shape: 'square',
        p: { x: PM.P(72), y: PM.P(24), start: PM.P(0), end: PM.P(100) } }] },
      p: { 'position.x': 100, 'position.y': 80 } }, project);
    layer.compat = 2;
    project.layers = [layer];
    PM.replaceProject(project); PM.touch(); PM.setTime(.75, { force: true });
    const apply = (command: any) => {
      const edit = PM.Edit.apply({ target: layer.id, preserveHandEdits: false, ...command });
      if (!edit.ok) throw new Error(JSON.stringify(edit));
    };
    // A typography edit records the font anchor bounds.
    apply({ type: 'set_property', path: 'c.size', value: 48 });
    const api = PM.EngineDoc.api;
    const before = { ...api.bounds(layer, .75) };
    const fallbacks = { ...PM.EngineDoc.stats().native.fallbacks };
    apply({ type: 'set_property', path: 'anchor.x', value: 30, time: .75 });
    apply({ type: 'set_property', path: 'anchor.y', value: 10, time: .75 });
    const after = { ...api.bounds(layer, .75) };
    const native = PM.EngineDoc.stats().native;
    return { recorded: !!layer.d.fontAnchorBounds, before, after,
      anchor: [PM.ev(PM.L(layer.id), 'anchor.x', .75), PM.ev(PM.L(layer.id), 'anchor.y', .75)],
      fallbacks: Object.entries(native.fallbacks).filter(([key, count]) => Number(count) > Number(fallbacks[key] || 0)),
      route: PM.EngineConnection.status().route };
  });
  expect(result.recorded).toBe(true);
  expect(result.anchor).toEqual([30, 10]);
  expect(result.route).toBe('engine');
  expect(result.fallbacks, 'anchor edits stay native').toEqual([]);
  for (const key of ['x0', 'y0', 'x1', 'y1'] as const) {
    expect(Math.abs(result.after[key] - result.before[key]), `${key}: ${JSON.stringify(result)}`).toBeLessThan(0.01);
  }
  await expectEngineDrewText(session);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('level 2 selection keeps paragraph boxes, small-text padding and blank-text picks', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const cases = [
    { name: 'whitespace', text: '   ', size: 8, leading: 1.25, align: 'left', boxWidth: 0, boxHeight: 0 },
    { name: 'empty', text: '', size: 8, leading: 1.25, align: 'left', boxWidth: 0, boxHeight: 0 },
    { name: 'unicode whitespace', text: '\t\u2003\n', size: 8, leading: 1.25, align: 'left', boxWidth: 0, boxHeight: 0 },
    ...(['left', 'center', 'right'] as const).flatMap(align => [0, 220].map(boxHeight => ({
      name: `paragraph ${align} height ${boxHeight}`, text: 'one\ntwo', size: 12, leading: 1.25,
      align, boxWidth: 260, boxHeight,
    }))),
    { name: 'small point text', text: 'Tiny', size: 8, leading: 1.25, align: 'left', boxWidth: 0, boxHeight: 0 },
  ];
  for (const item of cases) {
    const result = await session.page.evaluate(async item => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: item.name, w: 640, h: 360, dur: 3 });
      const layer = PM.mkLayer('text', { d: { ...item, font: 'Georgia', weight: 400, color: '#FFFFFF',
        boxWidth: PM.P(item.boxWidth), boxHeight: PM.P(item.boxHeight) },
        p: { 'position.x': 320, 'position.y': 80 } }, project);
      layer.compat = 2;
      project.layers = [layer];
      PM.replaceProject(project); PM.touch(); PM.setTime(0, { force: true });
      const shot = await PM.EngineCaptures.snapshotAsync(0, 640);
      const before = { ...PM.EngineDoc.stats().native.fallbacks };
      const api = PM.EngineDoc.api;
      const bounds = api.bounds(layer, 0);
      const x = 320 + bounds.x1 - .25;
      const y = 80 + (bounds.y0 + bounds.y1) / 2;
      const inside = api.pick(x, y, 0)?.id === layer.id;
      const outside = !!api.pick(x + 1, y, 0);
      // Inspect this composition's rendered pixels, never the user's screen.
      const image = new Image(); image.src = shot; await image.decode();
      const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
      const ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0);
      const pixels = ctx.getImageData(0, 0, 640, 360).data;
      let right = -1;
      for (let i = 0; i < pixels.length; i += 4) {
        if (pixels[i]! > 128 && pixels[i + 1]! > 128 && pixels[i + 2]! > 128) right = Math.max(right, (i / 4) % 640);
      }
      const native = PM.EngineDoc.stats().native;
      return { bounds, inside, outside, right,
        fallbacks: Object.entries(native.fallbacks).filter(([key, count]) => Number(count) > Number(before[key] || 0)),
        status: PM.EngineConnection.status() };
    }, item);
    expect(result.status.route).toBe('engine');
    expect(result.status.failures).toEqual([]);
    expect(result.fallbacks, item.name).toEqual([]);
    expect(result.inside, `${item.name}: edge inside selection`).toBe(true);
    expect(result.outside, `${item.name}: outside selection`).toBe(false);
    if (item.boxWidth > 0) {
      const x0 = item.align === 'center' ? -130 : item.align === 'right' ? -260 : 0;
      expect(result.bounds.x0).toBeCloseTo(x0 - 3, 6);
      expect(result.bounds.x1).toBeCloseTo(x0 + 263, 6);
      expect(result.bounds.y0).toBeCloseTo(-3, 6);
      expect(result.bounds.y1).toBeCloseTo(item.boxHeight > 0 ? 223 : 33, 6);
    } else if (!item.text.trim()) {
      expect(result.bounds.x0).toBeCloseTo(-3.5, 6);
      expect(result.bounds.x1).toBeCloseTo(3.5, 6);
      expect(result.bounds.y1 - result.bounds.y0).toBeGreaterThanOrEqual(14);
      if (item.text.includes('\n')) expect(result.bounds.y1 - result.bounds.y0).toBeGreaterThanOrEqual(26);
    } else {
      expect(result.right).toBeGreaterThan(320);
      expect(320 + result.bounds.x1 - result.right, 'padding beyond visible small-text ink').toBeGreaterThanOrEqual(2);
    }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a character no font covers draws .notdef in level 2 text and keeps every query native', async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  const measure = (text: string) => session.page.evaluate(async text => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Uncovered', w: 640, h: 360, dur: 3 });
    // Plain level 2 text: no animators, range styles or typography anchor.
    const layer = PM.mkLayer('text', { d: { text, font: 'Georgia', weight: 400, size: 40, color: '#FFFFFF', align: 'left' },
      p: { 'position.x': 100, 'position.y': 80 } }, project);
    layer.compat = 2;
    project.layers = [layer];
    PM.replaceProject(project); PM.touch(); PM.setTime(0, { force: true }); PM.invalidate();
    const shot = await PM.EngineCaptures.snapshotAsync(0, 640);
    const before = { ...PM.EngineDoc.stats().native.fallbacks };
    const api = PM.EngineDoc.api;
    const bounds = api.bounds(layer, 0);
    const layout = api.glyphLayout(layer, 0);
    const caret = api.textLayout(layer, 0);
    const picked = api.pick(100 + bounds.x1 - 6, 80 + (bounds.y0 + bounds.y1) / 2, 0)?.id === layer.id;
    // This composition's rendered pixels, never the user's screen.
    const image = new Image(); image.src = shot; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
    const ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0);
    const pixels = ctx.getImageData(0, 0, 640, 360).data;
    let right = -1;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i]! > 128 && pixels[i + 1]! > 128 && pixels[i + 2]! > 128) right = Math.max(right, (i / 4) % 640);
    }
    const native = PM.EngineDoc.stats().native;
    return { bounds, right, picked, characters: layout.characters.length,
      boundaries: caret.lines.reduce((n: number, line: any) => n + line.boundaries.length, 0),
      fallbacks: Object.entries(native.fallbacks).filter(([key, count]) => Number(count) > Number(before[key] || 0)),
      status: PM.EngineConnection.status() };
  }, text);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.EngineConnection.status().route)).toBe('engine');
  const covered = await measure('Motion type');
  const uncovered = await measure('Motion type ͸');
  for (const result of [covered, uncovered]) {
    expect(result.status.route).toBe('engine');
    expect(result.status.failures).toEqual([]);
    expect(result.fallbacks, 'geometry stays native').toEqual([]);
    expect(result.picked).toBe(true);
  }
  // The uncovered character keeps its unit and caret stop and draws a box
  // past the covered text: the layer is drawn, not refused.
  expect(uncovered.characters).toBeGreaterThan(covered.characters);
  expect(uncovered.boundaries).toBe(covered.boundaries + 2);
  expect(uncovered.bounds.x1).toBeGreaterThan(covered.bounds.x1 + 10);
  expect(uncovered.right).toBeGreaterThan(covered.right + 10);
  expect(uncovered.bounds.x0).toBeCloseTo(covered.bounds.x0, 6);
  await expectEngineDrewText(session);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
