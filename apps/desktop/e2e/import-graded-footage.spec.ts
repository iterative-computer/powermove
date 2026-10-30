import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
// Left half red, right half 75% grey, as sRGB before any HDR or log encoding.
const PATTERN = ['-f', 'lavfi', '-i', 'color=c=red:s=160x90:r=30:d=1[a];color=c=0xBFBFBF:s=160x90:r=30:d=1[b];[a][b]hstack'];

/** Import one clip, place it over the whole composition and read both halves. */
async function importAndSample(session: any, source: string) {
  const { page } = session;
  await page.evaluate(() => {
    document.querySelector('#graded-source')?.remove();
    const input = document.createElement('input'); input.type = 'file'; input.id = 'graded-source'; document.body.appendChild(input);
  });
  await page.locator('#graded-source').setInputFiles([source]);
  return page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    PM.proj.layers.splice(0);
    await PM.importFiles([...((document.querySelector('#graded-source') as HTMLInputElement).files!)]);
    const layer = PM.proj.layers.find((candidate: any) => candidate.type === 'video');
    const asset = PM.assets.get(layer.d.asset);
    PM.perf.auto = false; PM.quality = 1;
    PM.proj.w = 320; PM.proj.h = 90;
    layer.from = 0; layer.dur = 1; layer.mblur = false;
    layer.d.w = PM.P(320); layer.d.h = PM.P(90);
    layer.p['position.x'].v = 160; layer.p['position.y'].v = 45;
    layer.p['scale.x'].v = 100; layer.p['scale.y'].v = 100;
    PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.GL.previewViewport = null; PM.GL.resize(320, 90);
    PM.setTime(.5, { force: true });
    await new Promise(resolve => setTimeout(resolve, 600));
    PM.GL.render(PM.time, {});
    const gl = PM.GL.gl, { width, height } = PM.GL.canvas;
    const read = (x: number) => {
      const pixel = new Uint8Array(4);
      gl.readPixels(Math.floor(width * x), Math.floor(height / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
      return [...pixel.slice(0, 3)];
    };
    const stored = await PM.MediaStore.get(PM.proj.assets[asset.id]);
    const head = new Uint8Array(await stored.slice(0, 8).arrayBuffer());
    const container = new TextDecoder().decode(head.slice(4, 8)) === 'ftyp' ? 'mp4' : head[0] === 0x1a && head[1] === 0x45 ? 'webm' : 'other';
    return { proxy: asset.playbackProxy === true, durable: PM.proj.assets[asset.id]?.playbackProxy === true,
      container, red: read(.25), grey: read(.75), canvas: [width, height] };
  });
}

test('HDR footage is tone mapped once so the editor shows SDR levels', async ({ session }) => {
  await session.openEditor();
  const source = path.join(session.userData, 'hlg.mov');
  execFileSync(ffmpeg, ['-v', 'error', ...PATTERN.slice(0, 3), PATTERN[3]!,
    '-vf', 'format=gbrp,zscale=rangein=full:tin=iec61966-2-1:t=arib-std-b67:pin=bt709:p=bt2020:min=gbr:m=bt2020nc:r=tv:npl=203,format=yuv420p10le',
    '-c:v', 'libx265', '-tag:v', 'hvc1', '-x265-params', 'log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc',
    '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67', '-colorspace', 'bt2020nc', source]);
  const result = await importAndSample(session, source);
  expect(result.proxy).toBe(true);
  expect(result.durable).toBe(true);
  // The GPU helper writes HEVC in MP4; FFmpeg would have written WebM.
  expect(result.container).toBe('mp4');
  // Chromium's own HLG conversion shows this red near 170 and the grey near 140.
  expect(result.red[0]).toBeGreaterThan(235);
  expect(Math.max(result.red[1]!, result.red[2]!)).toBeLessThan(30);
  for (const channel of result.grey) expect(Math.abs(channel - 191), JSON.stringify(result)).toBeLessThan(10);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('BT.2020 log footage keeps its code values for grading', async ({ session }) => {
  await session.openEditor();
  const source = path.join(session.userData, 'log.mov');
  // A flat, log-like picture tagged the way wide-gamut camera ProRes is.
  execFileSync(ffmpeg, ['-v', 'error', ...PATTERN.slice(0, 3), PATTERN[3]!,
    '-vf', 'eq=contrast=0.55:saturation=0.45,format=gbrp,zscale=rangein=full:min=gbr:m=bt2020nc:r=tv,format=yuv422p10le',
    '-c:v', 'prores_ks', '-profile:v', '3', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-colorspace', 'bt2020nc', source]);
  const expected = [...execFileSync(ffmpeg, ['-v', 'error', '-ss', '0.5', '-i', source, '-frames:v', '1',
    '-vf', 'zscale=m=bt709:r=tv,format=yuv444p,scale=in_color_matrix=bt709:in_range=tv:out_range=full,format=rgb24', '-f', 'rawvideo', '-'])];
  const at = (x: number) => expected.slice((45 * 320 + x) * 3, (45 * 320 + x) * 3 + 3);
  const result = await importAndSample(session, source);
  expect(result.proxy).toBe(true);
  expect(result.container).toBe('webm');
  // Gamut conversion to sRGB would push the flat red's green channel to zero.
  for (const [actual, wanted] of [[result.red, at(80)], [result.grey, at(240)]] as const) {
    actual.forEach((channel, index) => expect(Math.abs(channel - wanted[index]!), JSON.stringify({ result, wanted })).toBeLessThan(4));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
