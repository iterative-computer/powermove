// Frames: F16 VideoFrames from a WebGL2 RGBA16F OffscreenCanvas (copy path in
// CreateExternalTexture, so Blink leases WebGpuSharedImageWrappers).
const W = 1496, H = 792;
let mode, draws = 0, errors = 0, losses = 0, painter = null, sourceGl = null, source = null;
function makeSource() {
  source = new OffscreenCanvas(W, H);
  sourceGl = source.getContext('webgl2', { alpha: true, premultipliedAlpha: true, preserveDrawingBuffer: true });
  sourceGl.getExtension('EXT_color_buffer_float');
  sourceGl.drawingBufferStorage(sourceGl.RGBA16F, W, H);
}
async function webgpuPainter() {
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error('no adapter');
  const device = await adapter.requestDevice();
  const canvas = new OffscreenCanvas(W, H);
  const ctx = canvas.getContext('webgpu');
  ctx.configure({ device, format: 'rgba16float', alphaMode: 'premultiplied', colorSpace: 'srgb', toneMapping: { mode: 'extended' } });
  const module = device.createShaderModule({ code: `
    @group(0) @binding(0) var t: texture_external;
    @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
      let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u)); return vec4f(p * 2.0 - 1.0, 0.0, 1.0); }
    @fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f { return textureLoad(t, vec2u(p.xy)); }` });
  const pipeline = device.createRenderPipeline({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: 'fs', targets: [{ format: 'rgba16float' }] } });
  let lost = false;
  // Deliberately keep drawing on the old device until `lost` resolves, as any
  // presenter must: the loss is only observable asynchronously.
  device.lost.then(() => { lost = true; });
  return {
    get lost() { return lost; },
    draw(frame) {
      const ext = device.importExternalTexture({ source: frame, colorSpace: 'srgb' });
      const bind = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: [{ binding: 0, resource: ext }] });
      const enc = device.createCommandEncoder();
      const pass = enc.beginRenderPass({ colorAttachments: [{ view: ctx.getCurrentTexture().createView(), loadOp: 'clear', storeOp: 'store', clearValue: [0, 0, 0, 0] }] });
      pass.setPipeline(pipeline); pass.setBindGroup(0, bind); pass.draw(3); pass.end();
      device.queue.submit([enc.finish()]);
      return device.queue.onSubmittedWorkDone();
    }
  };
}
async function webglPainter() {
  const canvas = new OffscreenCanvas(W, H);
  const gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: true, antialias: false });
  gl.getExtension('EXT_color_buffer_float');
  gl.drawingBufferStorage(gl.RGBA16F, W, H);
  const tex = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  let lost = false;
  canvas.addEventListener('webglcontextlost', (e) => { e.preventDefault(); lost = true; });
  return {
    get lost() { return lost || gl.isContextLost(); },
    draw(frame) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, gl.RGBA, gl.HALF_FLOAT, frame);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.flush();
      return Promise.resolve();
    }
  };
}
async function loop() {
  for (;;) {
    try {
      if (!sourceGl || sourceGl.isContextLost()) makeSource();
      if (!painter || painter.lost) { if (painter) losses++; painter = null; painter = await (mode === 'webgl' ? webglPainter() : webgpuPainter()); }
      sourceGl.clearColor(Math.random() * 2, 0.25, 0.5, 1); sourceGl.clear(sourceGl.COLOR_BUFFER_BIT);
      const frame = new VideoFrame(source, { timestamp: draws });
      const done = painter.draw(frame);
      draws++;
      // Close after completion (or loss), as the presenter does.
      done.then(() => frame.close(), () => frame.close());
    } catch (e) { errors++; painter = null; await new Promise((r) => setTimeout(r, 50)); }
    if (draws % 4 === 0) postMessage({ draws, errors, losses });
    await new Promise((r) => setTimeout(r, 0));
  }
}
onmessage = (e) => { mode = e.data.mode; loop(); };
