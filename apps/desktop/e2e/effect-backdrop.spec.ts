import { expect, test } from './helpers/app';

test('effects can sample the accumulated backdrop beneath a layer', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Kernel.registerEffect('backdrop-test', {
      id: 'backdrop-test', label: 'Backdrop Test', group: 'Test', backdrop: true,
      params: [],
      frag: 'vec4 b = texture(u_backdrop, v_st); vec4 s = texture(u_tex, v_st); o = vec4(b.rgb * s.a, s.a);'
    });
    const project = PM.mkProject({ name: 'Backdrop test', w: 100, h: 100, fps: 30, dur: 1, bg: '#0000ff' });
    const background = PM.mkLayer('shape', {
      d: { w: 40, h: 40, shape: 'rect', color: '#ff0000', stroke: 0 },
      p: { 'position.x': 50, 'position.y': 50 }
    }, project);
    const glass = PM.mkLayer('shape', {
      d: { w: 60, h: 60, shape: 'rect', color: '#ffffff', stroke: 0 },
      p: { 'position.x': 50, 'position.y': 50 }
    }, project);
    glass.fx = [PM.mkEffect('backdrop-test')];
    project.layers = [glass, background];
    PM.replaceProject(project);
    const pixels = PM.GL.renderToPixels(0, 100, 100, { mblur: false });
    const sample = (x: number, y: number) => Array.from(pixels.slice((y * 100 + x) * 4, (y * 100 + x) * 4 + 4));
    return { center: sample(50, 50), corner: sample(2, 2), errors: [...PM.GL.errors.values()] };
  });
  expect(result.errors).toEqual([]);
  expect(result.center[0]).toBeGreaterThan(220);
  expect(result.center[2]).toBeLessThan(40);
  expect(result.corner[2]).toBeGreaterThan(220);
});
