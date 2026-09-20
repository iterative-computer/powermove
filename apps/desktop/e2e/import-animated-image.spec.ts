import { expect, test } from './helpers/app';
import { exportStillPixels, importFixture } from './helpers/media';

/* An animated GIF, APNG or WebP is a clip, not a still. Chromium decodes the
   frames, the main process encodes them, and everything downstream treats the
   result as ordinary video. */

const FIXTURES = [
  { file: 'animated.gif', container: 'GIF' },
  { file: 'animated-apng.png', container: 'APNG' },
  { file: 'animated.webp', container: 'WebP' },
] as const;

for (const { file, container } of FIXTURES) {
  test(`an animated ${container} imports as a clip that plays every frame in order`, async ({ session }) => {
    test.setTimeout(120_000);
    await session.openEditor();
    const { page } = session;
    // Match the fixture's frame size so an export samples the clip one to one.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', {
        detail: PM.mkProject({ w: 160, h: 90, fps: 30, dur: 2, name: 'Animation test' }),
      }));
      PM.ProjectsScreen.hide();
      PM.setTime(0, { raw: true, force: true });
    });

    await importFixture(page, file);
    await page.waitForFunction(name => (window as any).PM.proj.layers.some((l: any) => l.name === name), file);

    const imported = await page.evaluate(name => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((l: any) => l.name === name);
      const asset = PM.assets.get(layer.d.asset);
      return {
        layers: PM.proj.layers.length, type: layer.type,
        kind: asset.kind, format: asset.format, w: asset.w, h: asset.h, dur: asset.dur,
        imageSequence: asset.imageSequence, hasElement: !!asset.el,
      };
    }, file);
    // Three 100 ms frames, so the clip runs 0.3 s at 10 fps.
    expect(imported).toMatchObject({
      layers: 1, type: 'video', kind: 'video', w: 160, h: 90,
      imageSequence: { fps: 10, frames: 3 }, hasElement: true,
    });
    expect(imported.format).toBe(file.split('.').pop());
    expect(imported.dur).toBeCloseTo(0.3, 2);

    /* Every frame has an opaque left half and a transparent right half, so a
       still export proves both the frame order and that alpha survived. */
    const frames: number[][][] = [];
    for (let frame = 0; frame < 3; frame++) {
      frames.push(await exportStillPixels(session, frame / 10, 160, 90, [[40, 45], [120, 45]]));
    }

    expect(frames).toHaveLength(3);
    frames.forEach(([opaque, transparent], index) => {
      const channel = [0, 1, 2][index]!;
      expect(opaque![channel]).toBeGreaterThan(200);
      expect(opaque![(channel + 1) % 3]).toBeLessThan(60);
      expect(opaque![3]).toBe(255);
      expect(transparent![3]).toBe(0);
    });

    // The converted clip, not the original container, is what gets stored.
    const stored = await page.evaluate(() => {
      const PM = (window as any).PM, asset: any = Object.values(PM.proj.assets)[0];
      return PM.MediaStore.get(asset.storageKey).then((blob: Blob | null) => ({ type: blob?.type, size: blob?.size }));
    });
    expect(stored.type).toBe('video/webm');
    expect(stored.size).toBeGreaterThan(0);
  });
}

test('a still image in an animatable container still imports as an image', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'still.webp');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'still.webp'));
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0];
    return { type: layer.type, kind: PM.assets.get(layer.d.asset).kind };
  })).toEqual({ type: 'image', kind: 'image' });
});
