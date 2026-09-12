import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('path masks preserve video RGB inside the matte across frames', async ({ session }) => {
  await importFixture(session.page, 'h264-aac.mp4');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  const pixels = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    const clip = PM.proj.layers.find((l: any) => l.type === 'video');
    PM.proj.layers = [clip]; clip.from = 0;
    const mask = PM.mkMask('rect', PM.proj);
    mask.p.x.v = 0; mask.p.y.v = 0; mask.p.feather.v = 0;
    mask.path = { id: 'test-path', name: 'Cutout', parent: null,
      p: Object.fromEntries(Object.entries({ x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: true }).map(([k,v]) => [k,PM.P(v)])),
      vertices: [[-100,-100],[100,-100],[100,100],[-100,100]].map(([x,y], i) => ({ id: `v${i}`, p: Object.fromEntries(Object.entries({ x,y,inX:0,inY:0,outX:0,outY:0 }).map(([k,v]) => [k,PM.P(v)])) })) };
    clip.masks = [mask];
    const result: Array<{ source: number[]; masked: number[] }> = [];
    for (const time of [0.25, 1, 1.5]) {
      const sample = async () => {
      const url = await PM.Export.snapshotAsync(time, 320);
      const img = new Image(); img.src = url; await img.decode();
      const canvas = document.createElement('canvas'); canvas.width = img.width; canvas.height = img.height;
      const ctx = canvas.getContext('2d')!; ctx.drawImage(img, 0, 0);
      return [...ctx.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data];
      };
      clip.masks = [];
      const source = await sample();
      clip.masks = [mask];
      result.push({ source, masked: await sample() });
    }
    return result;
  });
  for (const { source, masked } of pixels) {
    expect(Math.max(...source.slice(0, 3))).toBeGreaterThan(220);
    expect(Math.min(...source.slice(0, 3))).toBeLessThan(30);
    for (let channel = 0; channel < 4; channel++) expect(Math.abs(masked[channel]! - source[channel]!)).toBeLessThan(10);
  }
});
