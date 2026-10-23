import { expect, test } from './helpers/app';

test('custom shader colors render before selection and survive reopening without inspector metadata', async ({ session }) => {
  await session.openEditor();
  const setup = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Restored shader colors', w: 128, h: 72, dur: 4 });
    const layer = PM.mkLayer('shader', { d: { w: 128, h: 72,
      code: 'uniform vec3 uHueColor; // @param #F04700\nvoid main(){ fragColor = vec4(uHueColor, 1.0); }',
      uniforms: { uHueColor: PM.P('#F04700') },
    } }, project);
    project.layers = [layer];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide(); PM.pause(); PM.sel.layers = []; PM.agentFrameCapture = true;
    PM.UIState.setShaderMeta(PM.proj.layers[0], { udefs: [] });
    const pixels = PM.GL.renderToPixels(0, 128, 72, { mblur: false });
    return { id: project.id, pixel: [...pixels.slice((36 * 128 + 64) * 4, (36 * 128 + 64) * 4 + 4)] };
  });
  expect(setup.pixel).toEqual([240, 71, 0, 255]);
  await session.page.evaluate(async () => { await (window as any).PM.flushProject(); });
  await session.relaunch();
  await session.page.waitForFunction(id => (window as any).PM?.proj?.id === id && Boolean((window as any).PM.GL?.gl), setup.id);
  const reopened = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    PM.UIState.setShaderMeta(PM.proj.layers[0], { udefs: [] });
    const pixels = PM.GL.renderToPixels(0, 128, 72, { mblur: false });
    return [...pixels.slice((36 * 128 + 64) * 4, (36 * 128 + 64) * 4 + 4)];
  });
  expect(reopened).toEqual(setup.pixel);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
