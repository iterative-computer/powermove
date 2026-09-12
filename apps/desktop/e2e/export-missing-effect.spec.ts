import { expect, test } from './helpers/app';

test('web export preserves inactive missing effects and their animation', async ({ session }) => {
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    PM.proj = PM.mkProject({ name: 'Missing effect export', w: 160, h: 90, dur: 1 });
    const layer = PM.mkLayer('solid', { name: 'Box' }, PM.proj);
    const effect = { id: 'swap', type: 'word-slide-swap', on: true, missing: true,
      p: { progress: PM.P(0, { kf: [PM.KF(0, 0), PM.KF(.6, 100)] }) } };
    layer.fx.push(effect);
    PM.proj.layers.push(layer);
    const before = PM.serialize();
    const exported = await PM.Export.buildWeb();
    return {
      unchanged: before === PM.serialize(),
      preserved: JSON.stringify(exported.scene.project.layers[0].fx[0]) === JSON.stringify(effect),
      warnings: exported.scene.warnings,
      definitions: exported.scene.effects.length,
      bytes: exported.bytes.length,
    };
  });
  expect(result.unchanged).toBe(true);
  expect(result.preserved).toBe(true);
  expect(result.definitions).toBe(0);
  expect(result.warnings.join('\n')).toContain('word-slide-swap');
  expect(result.bytes).toBeGreaterThan(10000);
});
