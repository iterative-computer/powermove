const HDR_GAIN = 3.5;

function halfToFloat(value) {
  const sign = value & 0x8000 ? -1 : 1;
  const exponent = value >> 10 & 0x1f;
  const fraction = value & 0x3ff;
  if (exponent === 0) return sign * 2 ** -14 * (fraction / 1024);
  if (exponent === 31) return fraction ? NaN : sign * Infinity;
  return sign * 2 ** (exponent - 15) * (1 + fraction / 1024);
}

async function provesExtendedFloat(device) {
  const texture = device.createTexture({
    size: [1, 1],
    format: 'rgba16float',
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC
  });
  const buffer = device.createBuffer({
    size: 256,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ
  });
  try {
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [{
        view: texture.createView(),
        loadOp: 'clear',
        storeOp: 'store',
        clearValue: { r: 2, g: .25, b: 0, a: 1 }
      }]
    });
    pass.end();
    encoder.copyTextureToBuffer(
      { texture },
      { buffer, bytesPerRow: 256, rowsPerImage: 1 },
      [1, 1]
    );
    device.queue.submit([encoder.finish()]);
    await buffer.mapAsync(GPUMapMode.READ);
    const value = halfToFloat(new Uint16Array(buffer.getMappedRange())[0]);
    buffer.unmap();
    return value > 1;
  } finally {
    texture.destroy();
    buffer.destroy();
  }
}

const shader = /* wgsl */`
  struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
  }

  @vertex fn vertexMain(@builtin(vertex_index) index: u32) -> VertexOut {
    var positions = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    var output: VertexOut;
    output.position = vec4f(positions[index], 0., 1.);
    output.uv = vec2f((output.position.x + 1.) * .5, (1. - output.position.y) * .5);
    return output;
  }

  @group(0) @binding(0) var sourceTexture: texture_2d<f32>;
  @group(0) @binding(1) var sourceSampler: sampler;

  fn srgbToLinear(value: vec3f) -> vec3f {
    let low = value / 12.92;
    let high = pow((value + vec3f(.055)) / 1.055, vec3f(2.4));
    return select(low, high, value > vec3f(.04045));
  }

  @fragment fn fragmentMain(input: VertexOut) -> @location(0) vec4f {
    let sample = textureSample(sourceTexture, sourceSampler, input.uv);
    let straight = select(vec3f(0.), sample.rgb / max(sample.a, .00001), sample.a > .00001);
    let emissive = srgbToLinear(clamp(straight, vec3f(0.), vec3f(1.))) * ${HDR_GAIN};
    return vec4f(emissive * sample.a, sample.a);
  }
`;

export async function createOnboardingHdrOutput(svg, onError = () => undefined) {
  if (!navigator.gpu || !matchMedia('(dynamic-range: high)').matches) return null;
  let device;
  try {
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
    if (!adapter) return null;
    device = await adapter.requestDevice();
    if (!await provesExtendedFloat(device)) { device.destroy(); return null; }
    const canvas = document.createElement('canvas');
    canvas.id = 'onboarding-hdr';
    canvas.setAttribute('aria-hidden', 'true');
    const vectorCanvas = document.createElement('canvas');
    const maskCanvas = document.createElement('canvas');
    const layerCanvas = document.createElement('canvas');
    const precise2d = { colorSpace: 'srgb', colorType: 'float16' };
    const context2d = (surface) => {
      try { return surface.getContext('2d', precise2d) || surface.getContext('2d'); }
      catch { return surface.getContext('2d'); }
    };
    const vectorContext = context2d(vectorCanvas);
    const maskContext = context2d(maskCanvas);
    const layerContext = context2d(layerCanvas);
    if (!vectorContext || !maskContext || !layerContext) { device.destroy(); return null; }
    const float16Canvas = [vectorContext, maskContext, layerContext]
      .every((context2d) => context2d.getContextAttributes?.().colorType === 'float16');
    const context = canvas.getContext('webgpu');
    if (!context) { device.destroy(); return null; }
    const configuration = {
      device,
      format: 'rgba16float',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
      alphaMode: 'premultiplied',
      colorSpace: 'srgb',
      toneMapping: { mode: 'extended' }
    };
    context.configure(configuration);
    const applied = context.getConfiguration?.();
    if (applied && (applied.format !== 'rgba16float' || applied.toneMapping?.mode !== 'extended')) {
      throw new Error('Extended HDR canvas configuration was not applied');
    }
    const shaderModule = device.createShaderModule({ code: shader });
    device.pushErrorScope('validation');
    const pipelineDescriptor = {
      layout: 'auto',
      vertex: { module: shaderModule, entryPoint: 'vertexMain' },
      fragment: {
        module: shaderModule,
        entryPoint: 'fragmentMain',
        targets: [{
          format: 'rgba16float',
          blend: {
            color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
            alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' }
          }
        }]
      },
      primitive: { topology: 'triangle-list' }
    };
    const pipeline = device.createRenderPipelineAsync
      ? await device.createRenderPipelineAsync(pipelineDescriptor)
      : device.createRenderPipeline(pipelineDescriptor);
    const pipelineError = await device.popErrorScope();
    if (pipelineError) throw new Error(`Could not create HDR render pipeline: ${pipelineError.message}`);
    const sampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear' });
    const evidence = {
      format: applied?.format ?? configuration.format,
      toneMapping: applied?.toneMapping?.mode ?? configuration.toneMapping.mode,
      alphaMode: applied?.alphaMode ?? configuration.alphaMode,
      extendedFloat: true,
      sourcePrecision: float16Canvas ? 'float16' : 'unorm8',
      gain: HDR_GAIN
    };
    const statistics = { frames: 0, pixelWidth: 0, pixelHeight: 0, configuration: evidence };
    let sourceTexture, bindGroup, dead = false, requested = false, drawing = null, firstFrame = false;
    const pathCache = new Map();

    const resize = () => {
      const width = Math.max(1, Math.round(window.innerWidth * devicePixelRatio));
      const height = Math.max(1, Math.round(window.innerHeight * devicePixelRatio));
      if (canvas.width === width && canvas.height === height && sourceTexture) return;
      canvas.width = width; canvas.height = height;
      vectorCanvas.width = width; vectorCanvas.height = height;
      maskCanvas.width = width; maskCanvas.height = height;
      layerCanvas.width = width; layerCanvas.height = height;
      statistics.pixelWidth = width; statistics.pixelHeight = height;
      sourceTexture?.destroy();
      sourceTexture = device.createTexture({
        size: [width, height],
        format: float16Canvas ? 'rgba16float' : 'rgba8unorm',
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT
      });
      bindGroup = device.createBindGroup({
        layout: pipeline.getBindGroupLayout(0),
        entries: [
          { binding: 0, resource: sourceTexture.createView() },
          { binding: 1, resource: sampler }
        ]
      });
    };

    const cachedPath = (data) => {
      let path = pathCache.get(data);
      if (!path) { path = new Path2D(data); pathCache.set(data, path); }
      return path;
    };
    const matrixFrom = (value) => {
      const numbers = String(value || '').match(/-?\d*\.?\d+(?:e[+-]?\d+)?/gi)?.map(Number);
      return numbers?.length === 6 ? numbers : [1, 0, 0, 1, 0, 0];
    };
    const rgbaStop = (stop) => {
      const value = stop.getAttribute('stop-color')?.replace('#', '') || '000000';
      const [r, g, b] = [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16));
      const opacity = Number(stop.dataset.hdrOpacity ?? stop.getAttribute('stop-opacity') ?? 1);
      return `rgba(${r}, ${g}, ${b}, ${opacity})`;
    };
    const makeGradient = (node, fit) => {
      const x1 = Number(node.getAttribute('x1')), y1 = Number(node.getAttribute('y1'));
      const x2 = Number(node.getAttribute('x2')), y2 = Number(node.getAttribute('y2'));
      const start = [fit.x + x1 * fit.width, fit.y + y1 * fit.height];
      const vector = [(x2 - x1) * fit.width, (y2 - y1) * fit.height];
      const lengthSquared = vector[0] ** 2 + vector[1] ** 2;
      const stops = [...node.querySelectorAll('stop')].map((stop) => ({
        offset: Number(stop.getAttribute('offset')), color: rgbaStop(stop)
      }));
      if (node.getAttribute('spreadMethod') !== 'reflect' || lengthSquared < 1e-6) {
        const gradient = layerContext.createLinearGradient(start[0], start[1], start[0] + vector[0], start[1] + vector[1]);
        stops.forEach((stop) => gradient.addColorStop(stop.offset, stop.color));
        return gradient;
      }
      const corners = [[0, 0], [canvas.width, 0], [0, canvas.height], [canvas.width, canvas.height]];
      const values = corners.map(([x, y]) => ((x - start[0]) * vector[0] + (y - start[1]) * vector[1]) / lengthSquared);
      const first = Math.floor(Math.min(...values)), last = Math.ceil(Math.max(...values));
      const span = Math.max(1, last - first);
      const gradient = layerContext.createLinearGradient(
        start[0] + vector[0] * first, start[1] + vector[1] * first,
        start[0] + vector[0] * last, start[1] + vector[1] * last
      );
      const repeated = [];
      for (let segment = first; segment < last; segment++) {
        for (const stop of stops) {
          const position = segment + (Math.abs(segment) % 2 ? 1 - stop.offset : stop.offset);
          repeated.push({ offset: Math.max(0, Math.min(1, (position - first) / span)), color: stop.color });
        }
      }
      repeated.sort((left, right) => left.offset - right.offset)
        .forEach((stop) => gradient.addColorStop(stop.offset, stop.color));
      return gradient;
    };
    const setPathTransform = (context2d, transform, fit) => {
      const [a, b, c, d, e, f] = matrixFrom(transform);
      context2d.setTransform(fit.scale * a, fit.scale * b, fit.scale * c, fit.scale * d,
        fit.x + fit.scale * e, fit.y + fit.scale * f);
    };
    const rasterizeVectorFrame = () => {
      const viewBox = svg.viewBox.baseVal;
      const scale = Math.min(canvas.width / viewBox.width, canvas.height / viewBox.height);
      const fit = {
        scale, width: viewBox.width * scale, height: viewBox.height * scale,
        x: (canvas.width - viewBox.width * scale) / 2,
        y: (canvas.height - viewBox.height * scale) / 2
      };
      vectorContext.setTransform(1, 0, 0, 1, 0, 0);
      vectorContext.clearRect(0, 0, canvas.width, canvas.height);
      for (const output of svg.querySelectorAll('[data-layer-id]')) {
        if (output.getAttribute('display') === 'none') continue;
        const maskId = output.getAttribute('mask')?.match(/#([^\)]+)/)?.[1];
        const index = maskId?.match(/(\d+)$/)?.[1];
        const glow = svg.querySelector(`#onboarding-bloom-${index} path`);
        const hollow = svg.querySelector(`#onboarding-hollow-${index} path`);
        const gradientNode = svg.querySelector(`#onboarding-gradient-${index}`);
        const blur = svg.querySelector(`#onboarding-blur-${index} feGaussianBlur`);
        const slope = svg.querySelector(`#onboarding-blur-${index} feFuncA`);
        const rect = output.querySelector('rect');
        if (!glow || !hollow || !gradientNode || !blur || !slope || !rect) continue;
        const data = glow.getAttribute('d');
        if (!data) continue;
        const path = cachedPath(data);
        maskContext.setTransform(1, 0, 0, 1, 0, 0);
        maskContext.globalCompositeOperation = 'source-over'; maskContext.globalAlpha = 1;
        maskContext.filter = 'none'; maskContext.clearRect(0, 0, canvas.width, canvas.height);
        maskContext.save();
        maskContext.filter = `blur(${Number(blur.getAttribute('stdDeviation')) * scale}px)`;
        maskContext.fillStyle = '#fff'; maskContext.globalAlpha = Number(glow.getAttribute('opacity') ?? 1);
        setPathTransform(maskContext, glow.getAttribute('transform'), fit); maskContext.fill(path); maskContext.restore();

        layerContext.setTransform(1, 0, 0, 1, 0, 0);
        layerContext.filter = 'none'; layerContext.globalAlpha = 1;
        layerContext.globalCompositeOperation = 'source-over'; layerContext.clearRect(0, 0, canvas.width, canvas.height);
        layerContext.globalCompositeOperation = 'lighter';
        const strength = Math.max(0, Number(slope.getAttribute('slope')));
        const whole = Math.floor(strength), fraction = strength - whole;
        for (let pass = 0; pass < whole; pass++) layerContext.drawImage(maskCanvas, 0, 0);
        if (fraction > 1e-4) { layerContext.globalAlpha = fraction; layerContext.drawImage(maskCanvas, 0, 0); }
        layerContext.globalAlpha = 1; layerContext.globalCompositeOperation = 'source-in';
        layerContext.fillStyle = makeGradient(gradientNode, fit); layerContext.fillRect(0, 0, canvas.width, canvas.height);
        layerContext.globalCompositeOperation = 'destination-out';
        layerContext.globalAlpha = 1; layerContext.fillStyle = '#000000';
        setPathTransform(layerContext, hollow.getAttribute('transform'), fit); layerContext.fill(path);
        vectorContext.globalAlpha = Number(rect.getAttribute('opacity') ?? 1);
        vectorContext.globalCompositeOperation = 'source-over'; vectorContext.drawImage(layerCanvas, 0, 0);
      }
      vectorContext.globalAlpha = 1; vectorContext.globalCompositeOperation = 'source-over';
    };

    const draw = async () => {
      resize();
      rasterizeVectorFrame();
      if (dead) return;
      device.queue.copyExternalImageToTexture(
        { source: vectorCanvas },
        { texture: sourceTexture, premultipliedAlpha: true },
        [canvas.width, canvas.height]
      );
      const encoder = device.createCommandEncoder();
      const pass = encoder.beginRenderPass({
        colorAttachments: [{
          view: context.getCurrentTexture().createView(),
          loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 0 }
        }]
      });
      pass.setPipeline(pipeline); pass.setBindGroup(0, bindGroup); pass.draw(3); pass.end();
      device.queue.submit([encoder.finish()]);
      statistics.frames += 1;
      if (!firstFrame) {
        firstFrame = true;
        svg.insertAdjacentElement('afterend', canvas);
        document.body.classList.add('onboarding-hdr-active');
      }
    };

    const fail = (error) => {
      if (dead) return;
      dead = true;
      document.body.classList.remove('onboarding-hdr-active');
      canvas.remove(); sourceTexture?.destroy(); device.destroy();
      onError(error instanceof Error ? error : new Error(String(error)));
    };
    const pump = async () => {
      try {
        while (requested && !dead) { requested = false; await draw(); }
      } catch (error) { fail(error); }
      finally { drawing = null; }
    };
    const present = () => {
      if (dead) return Promise.resolve();
      requested = true;
      drawing ||= pump();
      return drawing;
    };
    const probeExtendedScene = async () => {
      await present();
      if (dead || !sourceTexture || !bindGroup) return { max: 0, extended: false };
      const width = 64, height = 36, bytesPerRow = width * 8;
      const texture = device.createTexture({
        size: [width, height], format: 'rgba16float',
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC
      });
      const buffer = device.createBuffer({
        size: bytesPerRow * height,
        usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ
      });
      try {
        const encoder = device.createCommandEncoder();
        const pass = encoder.beginRenderPass({
          colorAttachments: [{
            view: texture.createView(), loadOp: 'clear', storeOp: 'store',
            clearValue: { r: 0, g: 0, b: 0, a: 0 }
          }]
        });
        pass.setPipeline(pipeline); pass.setBindGroup(0, bindGroup); pass.draw(3); pass.end();
        encoder.copyTextureToBuffer(
          { texture }, { buffer, bytesPerRow, rowsPerImage: height }, [width, height]
        );
        device.queue.submit([encoder.finish()]);
        await buffer.mapAsync(GPUMapMode.READ);
        const values = new Uint16Array(buffer.getMappedRange());
        let max = 0;
        for (let index = 0; index < values.length; index += 4) {
          max = Math.max(max, halfToFloat(values[index]), halfToFloat(values[index + 1]), halfToFloat(values[index + 2]));
        }
        buffer.unmap();
        return { max, extended: max > 1 };
      } finally {
        texture.destroy(); buffer.destroy();
      }
    };
    void device.lost.then((info) => fail(new Error(`HDR device lost: ${info.message || info.reason}`)));
    return {
      canvas,
      evidence,
      statistics,
      present,
      probeExtendedScene,
      destroy() {
        if (dead) return;
        dead = true; requested = false;
        document.body.classList.remove('onboarding-hdr-active');
        canvas.remove(); sourceTexture?.destroy(); device.destroy();
      }
    };
  } catch (error) {
    device?.destroy();
    onError(error instanceof Error ? error : new Error(String(error)));
    return null;
  }
}
