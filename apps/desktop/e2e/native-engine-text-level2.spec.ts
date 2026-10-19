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
      if (item.id !== 'text-level2/stagger' || time > 0) expect(result.picked, `${item.id} pick`).toBe(true);
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
