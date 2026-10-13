import { expect, test } from './helpers/app';

// A shader layer's vec3 colour @param is bound as a colour whether or not the
// Inspector has ever shown the layer (corpus v0 finding: it was bound with
// uniform1f, raising GL_INVALID_OPERATION and leaving the colour at zero).
test('shader colour parameters render without the Inspector having parsed them', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Shader colour', w: 64, h: 64, dur: 1, bg: '#000000' }));
    const shader = PM.mkLayer('shader', { d: {
      w: 64, h: 64,
      code: 'uniform vec3 uInk; // @param #ff0000\nuniform float uAmount; // @param 1 0 1\nvoid main(){fragColor=vec4(uInk*uAmount,1.0);}',
      uniforms: { uInk: PM.P('#00ff00'), uAmount: PM.P(1) },
    } }, PM.proj);
    // A shader layer's origin is its top-left corner: cover the whole frame.
    shader.p['position.x'].v = 0; shader.p['position.y'].v = 0;
    PM.proj.layers = [shader];
    PM.ProjectIndex.invalidate(); PM.touch();
    const gl = PM.GL.gl;
    const at = 4 * (32 * 64 + 32);
    // Programs may compile in parallel; render until this one has linked.
    let pixels: Uint8Array = new Uint8Array(64 * 64 * 4), error = 0;
    for (let attempt = 0; attempt < 100 && !pixels[at + 3]; attempt++) {
      if (attempt) await new Promise(resolve => setTimeout(resolve, 50));
      while (gl.getError() !== gl.NO_ERROR) { /* drain earlier errors */ }
      pixels = PM.GL.renderToPixels(0, 64, 64, { transparent: true, mblur: false });
      error = gl.getError();
    }
    const shown = PM.UIState.getShaderMeta(shader).udefs.length;
    return { shown, error, pixel: [...pixels.slice(at, at + 4)] };
  });
  expect(result.shown).toBe(0);
  expect(result.error).toBe(0);
  expect(result.pixel).toEqual([0, 255, 0, 255]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
