import { expect, test } from './helpers/app';

test('a new keyframe uses the displayed frame when playback pauses between frames', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Keyframe frame accuracy', fps: 30, dur: 4 }));
    const layer = PM.mkLayer('solid', { dur: 4 }); PM.proj.layers.push(layer);
    PM.play();
    // Animation clocks run continuously; the compositor displays frame 47 here.
    PM.setTime(1.59, { raw: true, force: true }); PM.pause();
    const key = PM.setKey(layer, 'position.x', PM.time, 120);
    return { time: PM.time, keyTime: key.t, frame: Math.round(key.t * 30) };
  });
  expect(result.time).toBeCloseTo(47 / 30, 9);
  expect(result.keyTime).toBeCloseTo(result.time, 9);
  expect(result.frame).toBe(47);
});
