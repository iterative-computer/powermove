import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

// Export reads supported clips in order with WebCodecs instead of seeking a
// <video> per frame (issue #185). Every exported pixel must match the seeking
// path, including B-frame reordering, an edit list, sparse keyframes and two
// layers sharing one clip at different times.
test('sequential video decoding exports exactly the frames seeking produces', async ({ session }) => {
  test.setTimeout(240_000);
  await session.openEditor();
  const clip = path.join(session.userData, 'bframes.mp4');
  execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=30',
    '-t', '4', '-c:v', 'libx264', '-bf', '3', '-g', '50', '-keyint_min', '50', '-sc_threshold', '0', '-pix_fmt', 'yuv420p', '-y', clip]);
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Sequential decode', w: 320, h: 180, fps: 30, dur: 2 }) }));
    PM.ProjectsScreen.hide();
  });
  const chooser = session.page.waitForEvent('filechooser');
  await session.page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(clip);
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'video'));
  await session.page.evaluate(() => {
    const PM = (window as any).PM, project = PM.proj;
    const video = project.layers.find((layer: any) => layer.type === 'video');
    PM.Edit.apply({ type: 'set_layer', target: video.id, patch: { from: 0 } });
    const inset = PM.mkLayer('video', { d: { ...video.d }, from: .5, dur: 1.5, p: { 'position.x': 240, 'position.y': 120, 'scale.x': 40, 'scale.y': 40 } }, project);
    project.layers.unshift(inset); PM.ProjectIndex?.invalidate?.(); PM.touch();
  });
  await session.app.evaluate(({ ipcMain }) => {
    const { createHash } = process.getBuiltinModule('node:crypto') as typeof import('node:crypto');
    const state = { hash: createHash('sha256'), bytes: 0, digests: [] as string[] };
    (globalThis as any).__sequentialExport = state;
    for (const name of ['start', 'write', 'finish', 'cancel']) ipcMain.removeHandler(`render:${name}`);
    ipcMain.handle('render:start', () => { state.hash = createHash('sha256'); state.bytes = 0; return 'sequential-test'; });
    ipcMain.handle('render:write', (_event, request) => {
      if (request.audio || request.repeat !== undefined) throw new Error('Unexpected write');
      state.hash.update(request.data); state.bytes += request.data.byteLength;
    });
    ipcMain.handle('render:finish', () => { state.digests.push(state.hash.digest('hex')); return { path: '/test/sequential.mp4' }; });
    ipcMain.handle('render:cancel', () => {});
  });
  const exportOnce = (withoutWebCodecs: boolean) => session.page.evaluate(async withoutWebCodecs => {
    const PM = (window as any).PM, decoder = (window as any).VideoDecoder;
    if (withoutWebCodecs) (window as any).VideoDecoder = undefined;
    try {
      const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'draft', audio: false, mblur: false });
      return { result, video: PM.Export.lastStats?.video };
    } finally { (window as any).VideoDecoder = decoder; }
  }, withoutWebCodecs);
  const sequential = await exportOnce(false);
  const seeking = await exportOnce(true);
  expect(sequential.result).toEqual({ cancelled: false });
  expect(seeking.result).toEqual({ cancelled: false });
  expect(sequential.video).toMatchObject({ seeked: 0, fallbacks: [] });
  expect(sequential.video.decoded).toBeGreaterThanOrEqual(60);
  expect(seeking.video).toMatchObject({ decoded: 0 });
  const state = await session.app.evaluate(() => { const s = (globalThis as any).__sequentialExport; return { digests: s.digests, bytes: s.bytes }; });
  expect(state.bytes).toBe(60 * 320 * 180 * 4);
  expect(state.digests).toHaveLength(2);
  expect(state.digests[0]).toBe(state.digests[1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
