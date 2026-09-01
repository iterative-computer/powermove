import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.describe('@export-mp4 H.264 delivery', () => {
  test('exports a playable MP4 with H.264 video and AAC audio', async ({ session }) => {
    test.setTimeout(45_000);
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      PM.assets.clear();
      PM.proj = PM.mkProject({ name: 'E2E MP4', w: 64, h: 64, fps: 30, dur: 1, bg: '#000000' });
      PM.proj.work = [0, 1];
      PM.time = 0;
      PM.sel.layers = [];
      PM.Edit.apply([{
        type: 'add_layer',
        layerType: 'solid',
        name: 'Red',
        from: 0,
        duration: 1,
        content: { color: '#FF0000', w: 64, h: 64 },
      }], { label: 'E2E red solid', origin: 'e2e' });
      PM.bus.emit('project');
      PM.bus.emit('layers');
    });
    await importFixture(page, 'tone.wav');
    await page.waitForFunction(() => (window as any).PM.proj.layers.some(
      (layer: any) => layer.type === 'audio' && layer.name === 'tone.wav',
    ));

    const exported = await page.evaluate(async () => {
      const PM = (window as any).PM;
      return await new Promise<{
        bytes: number[];
        name: string;
        type: string;
        width: number;
        height: number;
        duration: number;
        presentedFrames: number;
      }>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('Timed out waiting for exported MP4')), 35_000);
        PM.download = async (blob: Blob, name: string) => {
          const video = document.createElement('video');
          const url = URL.createObjectURL(blob);
          video.muted = true;
          video.src = url;
          try {
            await new Promise<void>((ready, failed) => {
              video.onloadedmetadata = () => ready();
              video.onerror = () => failed(new Error('Exported MP4 did not load'));
            });
            let presentedFrames = 0;
            const count = () => { presentedFrames += 1; };
            video.requestVideoFrameCallback?.(count);
            await video.play();
            await new Promise<void>((ended, failed) => {
              video.onended = () => ended();
              video.onerror = () => failed(new Error('Exported MP4 did not play'));
            });
            clearTimeout(timer);
            resolve({
              bytes: Array.from(new Uint8Array(await blob.arrayBuffer())),
              name,
              type: blob.type,
              width: video.videoWidth,
              height: video.videoHeight,
              duration: video.duration,
              presentedFrames,
            });
          } catch (error) {
            clearTimeout(timer);
            reject(error);
          } finally {
            URL.revokeObjectURL(url);
          }
        };
        void PM.Export.run({
          format: 'mp4',
          scale: 1,
          fps: 30,
          range: 'all',
          quality: 'draft',
          mblur: false,
          alpha: false,
          audio: true,
        });
      });
    });

    const bytes = Uint8Array.from(exported.bytes);
    const signatures = new TextDecoder('latin1').decode(bytes);
    expect(exported.name).toBe('E2E MP4.mp4');
    expect(exported.type).toBe('video/mp4');
    expect(String.fromCharCode(...bytes.subarray(4, 8))).toBe('ftyp');
    expect(signatures).toContain('avc1');
    expect(signatures).toContain('mp4a');
    expect(exported).toMatchObject({ width: 64, height: 64 });
    expect(exported.duration).toBeGreaterThan(0.7);
    expect(exported.duration).toBeLessThan(1.5);
    expect(exported.presentedFrames).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
