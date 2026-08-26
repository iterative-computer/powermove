import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';
import { inspectWebM } from './helpers/webm';

test.describe('@export-webm frame-exact WebM export', () => {
  test('exports VP8/VP9 video with Opus audio and frame blocks', async ({ session }) => {
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.assets.clear();
      PM.proj = PM.mkProject({ name: 'E2E WebM', w: 64, h: 64, fps: 30, dur: 1, bg: '#000000' });
      PM.proj.work = [0, 1];
      PM.time = 0;
      PM.sel.layers = [];
      PM.Edit.apply([{
        type: 'add_layer',
        layerType: 'solid',
        name: 'Red',
        from: 0,
        duration: 1,
        content: { color: '#FF0000', w: 64, h: 64 }
      }], { label: 'E2E red solid', origin: 'e2e' });
      PM.bus.emit('project');
      PM.bus.emit('layers');
    });

    await importFixture(page, 'tone.wav');
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return PM.proj.layers.some((layer: any) => layer.type === 'audio' && layer.name === 'tone.wav');
    });
    const opusAvailable = await page.evaluate(() => (window as any).PM.Audio.supportsOpus());

    const numbers = await page.evaluate(async () => {
      const PM = (window as any).PM;
      return await new Promise<number[]>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('Timed out waiting for exported WebM')), 25_000);
        PM.download = async (blob: Blob) => {
          clearTimeout(timer);
          resolve(Array.from(new Uint8Array(await blob.arrayBuffer())));
        };
        void PM.Export.run({
          format: 'webm',
          scale: 1,
          fps: 30,
          range: 'all',
          quality: 'draft',
          mblur: false,
          alpha: false,
          audio: true
        });
      });
    });
    const bytes = Uint8Array.from(numbers);
    expect([...bytes.subarray(0, 4)]).toEqual([0x1a, 0x45, 0xdf, 0xa3]);

    const inspected = inspectWebM(bytes);
    expect(inspected.hasTracks).toBe(true);
    expect(inspected.videoCodecs.some(codec => codec === 'V_VP9' || codec === 'V_VP8')).toBe(true);
    expect(inspected.simpleBlocks).toBeGreaterThanOrEqual(20);

    test.fixme(
      !opusAvailable && !inspected.audioCodecs.includes('A_OPUS'),
      'This runtime has no Opus AudioEncoder; waiting on the Phase 0 Opus capability'
    );
    expect(inspected.audioCodecs).toContain('A_OPUS');
  });
});
