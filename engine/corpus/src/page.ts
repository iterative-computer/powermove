/**
 * Renderer-side capture helpers. `installCorpusPage` is serialized into the
 * legacy app's renderer with `page.evaluate`, so it must stay self-contained:
 * no imports and no references to anything outside its own body.
 */
export function installCorpusPage(): void {
  const w = window as any;
  if (w.__corpus) return;
  const PM = w.PM;

  const toBase64 = (bytes: Uint8Array): string => {
    let out = '';
    for (let i = 0; i < bytes.length; i += 0x8000) out += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + 0x8000)));
    return btoa(out);
  };

  /** JSON-safe copy: non-finite numbers become strings, live objects become tags. */
  const plain = (value: any, depth = 0, seen = new WeakSet<object>()): any => {
    if (value === null || value === undefined) return value ?? null;
    if (typeof value === 'number') return Number.isFinite(value) ? (Object.is(value, -0) ? 0 : value) : String(value);
    if (typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'function') return undefined;
    if (typeof value !== 'object') return String(value);
    if (depth > 24) return '[depth]';
    if (seen.has(value)) return '[cycle]';
    if (ArrayBuffer.isView(value)) return Array.from(value as any, (v: any) => plain(v, depth + 1, seen));
    const proto = Object.getPrototypeOf(value);
    if (!Array.isArray(value) && proto !== Object.prototype && proto !== null) {
      if (value instanceof Map) return plain(Object.fromEntries(value), depth + 1, seen);
      return `[${proto?.constructor?.name || 'object'}]`;
    }
    seen.add(value);
    let out: any;
    if (Array.isArray(value)) out = value.map(item => { const v = plain(item, depth + 1, seen); return v === undefined ? null : v; });
    else {
      out = {};
      for (const key of Object.keys(value)) { const v = plain(value[key], depth + 1, seen); if (v !== undefined) out[key] = v; }
    }
    seen.delete(value);
    return out;
  };

  const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
  const isProperty = (value: any) => !!value && typeof value === 'object' && Array.isArray(value.kf) && 'v' in value;
  const helpers = () => (globalThis as any).__powermove_runtime?.powermove;

  /** `legacy/core/retiming.ts` sourceTime, which the renderer does not expose on PM. */
  const integrals = new WeakMap<object, { version: number; step: number; sums: number[] }>();
  const sourceTime = (layer: any, time: number): number => {
    const d = PM.resolveContent(layer, time), local = time - layer.from;
    if (d.timeRemap && isProperty(layer.d.sourceTime)) return Number(PM.evP(layer, layer.d.sourceTime, time, 'c.sourceTime')) || 0;
    const speed = layer.d.speed;
    if (!isProperty(speed) || (!speed.kf.length && !speed.expr)) return local * Number(d.speed ?? 1) + Number(d.trim || 0);
    const value = (t: number) => Number(isProperty(speed) ? PM.evP(layer, speed, t, 'c.speed') : speed);
    const end = Math.max(0, local), step = 1 / (Math.max(30, PM.proj.fps) * 2), version = PM.animVersion();
    let cached = integrals.get(speed);
    if (!cached || cached.version !== version || cached.step !== step) { cached = { version, step, sums: [0] }; integrals.set(speed, cached); }
    const full = Math.floor(end / step);
    for (let i = cached.sums.length - 1; i < full; i++) cached.sums.push(cached.sums[i]! + value(layer.from + (i + .5) * step) * step);
    const remainder = end - full * step;
    return cached.sums[full]! + value(layer.from + full * step + remainder / 2) * remainder + Number(d.trim || 0);
  };

  /** `legacy/gl/raster.ts` resolvedTextContent, which the renderer does not expose on PM. */
  const resolvedTextContent = (layer: any, time: number) => {
    const d = PM.resolveContent(layer, time);
    const value = (key: string) => {
      const source = d?.[key];
      if (!source || typeof source !== 'object' || Array.isArray(source)) return Number(source) || 0;
      return Number(PM.evP(layer, source, time, 'c.' + key)) || 0;
    };
    const boxWidth = Math.max(0, value('boxWidth')), boxHeight = Math.max(0, value('boxHeight'));
    const resolved: any = { ...d, boxWidth, boxHeight, paragraph: boxWidth > 0 };
    for (const key of Object.keys(d ?? {})) {
      if (!key.startsWith('fontAxis.')) continue;
      const source = d[key];
      if (!source || typeof source !== 'object' || !Array.isArray(source.kf)) continue;
      resolved[key] = Number(PM.evP(layer, source, time, 'c.' + key));
    }
    return resolved;
  };

  /**
   * Animator and range-style state and the posed glyphs, through the bundled
   * `legacy/core/text-animation.ts` (inject.ts), called exactly as raster.ts
   * calls it. Glyph matrices are relative to each glyph's baseline origin.
   * raster.ts later shifts glyphs by range-font advance differences measured
   * with Canvas2D; those shifts are rasterization and are not included.
   */
  const textAnimation = (layer: any, time: number) => {
    const T = w.__corpusTextAnimation;
    if (!T) throw new Error('text-animation bundle not installed');
    const d = resolvedTextContent(layer, time), layout = PM.textLayout(layer, time);
    const animation = T.textAnimationState(PM, layer, time, layout);
    const glyphs = T.animatedGlyphs(PM, layer, time, d, layout, animation);
    // The resolved style per glyph: base typography plus every key a range style sets.
    const styleKeys = new Set(['font', 'weight', 'size', 'color', 'italic', 'tracking', ...animation.styles.flatMap((range: any) => Object.keys(range.p || {}))]);
    return {
      animators: animation.animators, styles: animation.styles,
      glyphs: glyphs.map((g: any) => ({
        index: g.index, text: g.text, line: g.line, word: g.word, x: g.x, y: g.y, w: g.w,
        matrix: g.matrix, opacity: g.opacity, blur: g.blur, color: g.color, tracking: g.tracking, trackingShift: g.trackingShift ?? 0,
        style: Object.fromEntries([...styleKeys].map(key => [key, g.style?.[key] ?? null])),
      })),
    };
  };

  const layerState = (layer: any, time: number, comp: any) => {
    const channels: Record<string, any> = {};
    const errors: Record<string, string> = {};
    let props: any[] = [];
    try { props = PM.allProps(layer); } catch (error: any) { errors['*allProps'] = String(error?.message || error); }
    for (const { key, prop } of props) {
      try {
        channels[key] = plain(key in (layer.p || {}) ? PM.ev(layer, key, time) : PM.evP(layer, prop, time, key));
        const message = PM.expressionErrors.get(prop);
        if (message) errors[key] = message;
      } catch (error: any) { errors[key] = String(error?.message || error); }
    }
    const safe = (label: string, fn: () => any) => { try { return plain(fn()); } catch (error: any) { errors[`*${label}`] = String(error?.message || error); return null; } };
    const threeD = !!layer.threeD;
    const state: any = {
      id: layer.id, name: layer.name, type: layer.type, index: comp.layers.indexOf(layer),
      definition: layer.type === 'extension' ? layer.d?.definition ?? null : undefined,
      active: safe('active', () => PM.active(layer, time)),
      channels,
      layerFields: safe('layerFields', () => ({
        blend: isProperty(layer.blend) ? PM.evP(layer, layer.blend, time, 'l.blend') : layer.blend,
        mblur: isProperty(layer.mblur) ? PM.evP(layer, layer.mblur, time, 'l.mblur') : layer.mblur,
        on: isProperty(layer.on) ? PM.evP(layer, layer.on, time, 'l.on') : layer.on,
        matteMode: isProperty(layer.matteMode) ? PM.evP(layer, layer.matteMode, time, 'l.matteMode') : layer.matteMode ?? null,
        matteSource: layer.matteSource ?? null, parent: layer.parent ?? null, group: layer.group ?? null, threeD, solo: !!layer.solo,
      })),
      content: safe('content', () => PM.resolveContent(layer, time)),
      localMatrix: layer.type === 'audio' ? null : safe('localMatrix', () => PM.localMatrix(layer, time)),
      worldMatrix: layer.type === 'audio' ? null : safe('worldMatrix', () => PM.worldMatrix(layer, time)),
      worldOpacity: layer.type === 'audio' ? null : safe('worldOpacity', () => PM.worldOpacity(layer, time)),
      planeMatrix: threeD ? safe('planeMatrix', () => helpers()?.planeMatrix(PM, layer, time)) : null,
      bounds: layer.type === 'audio' ? null : safe('bounds', () => PM.GL.bounds(layer, time)),
    };
    if (layer.type === 'video' || layer.type === 'precomp' || layer.type === 'audio') state.sourceTime = safe('sourceTime', () => sourceTime(layer, time));
    if (layer.type === 'text') {
      state.textLayout = safe('textLayout', () => PM.textLayout(layer, time));
      state.textRaster = safe('textRaster', () => { const g = PM.textRasterGeometry(layer, 1, time); return { w: g?.w, h: g?.h, anchorX: g?.anchorX, anchorY: g?.anchorY, density: g?.density, selection: g?.selection }; });
      if (layer.d?.animators?.length || layer.d?.styles?.length) state.textAnimation = safe('textAnimation', () => textAnimation(layer, time));
    }
    if (layer.type === 'captions') state.captionBounds = safe('captionBounds', () => PM.captionBounds(layer, time));
    if (layer.type === 'shape' && layer.d?.paths?.length) {
      state.paths = safe('paths', () => layer.d.paths.map((path: any) => ({
        id: path.id, values: helpers()?.pathValues(PM, layer, path, time), matrix: helpers()?.groupMatrix(PM, layer, path, time),
      })));
    }
    if (Object.keys(errors).length) state.errors = errors;
    return state;
  };

  const compositionState = (comp: any, time: number, depth: number): any => {
    PM.beginEval(time);
    const layers = comp.layers.map((layer: any) => layerState(layer, time, comp));
    const nested: any[] = [];
    if (depth < 4) for (const layer of comp.layers) {
      if (layer.type !== 'precomp' || !PM.active(layer, time)) continue;
      const sub = PM.compOf?.(layer) || PM.proj.comps?.[layer.d?.comp];
      if (!sub) continue;
      const at = sourceTime(layer, time);
      PM.scope.push(sub);
      try { nested.push({ layer: layer.id, comp: layer.d.comp, time: at, state: compositionState(sub, at, depth + 1) }); }
      finally { PM.scope.pop(); PM.beginEval(time); }
    }
    return { id: comp.compId ?? comp.id, w: comp.w, h: comp.h, fps: comp.fps, time, layers, precomps: nested };
  };

  const corpus = {
    errors: [] as string[],

    async open(doc: any): Promise<{ hydrated: any }> {
      await PM.Kernel.loader.builtinsReady;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: JSON.parse(JSON.stringify(doc)) }));
      if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide();
      for (let i = 0; i < 200 && PM.proj?.id !== doc.id; i++) await sleep(25);
      if (PM.proj?.id !== doc.id) throw new Error(`Project ${doc.id} did not open`);
      await PM.projectAssetsReady;
      PM.pause();
      return { hydrated: JSON.parse(JSON.stringify(PM.proj)) };
    },

    /** Import the files chosen into `#corpus-media` through the app's asset path. */
    async importMedia(): Promise<Record<string, { id: string; kind: string; error?: string }>> {
      const input = document.getElementById('corpus-media') as HTMLInputElement;
      const out: Record<string, any> = {};
      for (const file of Array.from(input.files || [])) {
        try {
          const asset = await PM.assets.add(file, { newAsset: true, silent: true });
          out[file.name] = { id: asset.id, kind: asset.kind };
        } catch (error: any) { out[file.name] = { id: '', kind: '', error: String(error?.message || error) }; }
      }
      input.value = '';
      return out;
    },

    mediaInput(): void {
      let input = document.getElementById('corpus-media') as HTMLInputElement | null;
      if (!input) {
        input = document.createElement('input');
        input.type = 'file'; input.multiple = true; input.id = 'corpus-media';
        input.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none;left:-10px;top:-10px';
        document.body.appendChild(input);
      }
    },

    /** Replace `media:<name>` references with the ids the app assigned; returns the project to reopen. */
    rewriteMedia(map: Record<string, string>): any {
      let count = 0;
      const visit = (value: any): any => {
        if (typeof value === 'string' && value.startsWith('media:')) {
          const id = map[value.slice(6)];
          if (id) { count++; return id; }
          return value;
        }
        if (Array.isArray(value)) { for (let i = 0; i < value.length; i++) value[i] = visit(value[i]); return value; }
        if (value && typeof value === 'object') { for (const key of Object.keys(value)) value[key] = visit(value[key]); return value; }
        return value;
      };
      const project = JSON.parse(JSON.stringify(PM.proj));
      project.layers = visit(project.layers);
      project.comps = visit(project.comps);
      return { project, count };
    },

    async waitForMedia(timeoutMs = 30000): Promise<string[]> {
      const deadline = performance.now() + timeoutMs;
      const pending = () => {
        const out: string[] = [];
        if (PM.assets.loading?.size) out.push(`loading: ${[...PM.assets.loading].join(', ')}`);
        for (const [id, error] of PM.assets.errors ?? []) out.push(`error ${id}: ${error}`);
        for (const comp of [PM.proj, ...Object.values<any>(PM.proj.comps || {})]) for (const layer of comp.layers) {
          if (layer.type !== 'video') continue;
          const asset = PM.assets.get?.(layer.d.asset);
          if (!asset) out.push(`video ${layer.name}: asset ${layer.d.asset} not loaded`);
          else if (asset.el && asset.el.readyState < 2) out.push(`video ${layer.name}: readyState ${asset.el.readyState}`);
        }
        return out;
      };
      let last = pending();
      while (last.some(item => !item.startsWith('error')) && performance.now() < deadline) { await sleep(100); last = pending(); }
      await document.fonts?.ready;
      return last;
    },

    /**
     * Render a small frame until three consecutive renders agree (async shader compiles,
     * model loads, fonts). Export.busy is held as the exporter holds it, so the viewer's
     * paused-preview video sync cannot seek shared media elements under prepareFrame.
     */
    async settle(time: number, timeoutMs = 15000): Promise<{ stable: boolean; renders: number; prepareError?: string }> {
      const deadline = performance.now() + timeoutMs;
      const w = Math.max(2, Math.round(PM.proj.w / 4)), h = Math.max(2, Math.round(PM.proj.h / 4));
      let previous = '', streak = 0, renders = 0, prepareError: string | undefined;
      PM.pause();
      PM.Export.busy = true;
      try {
        while (performance.now() < deadline) {
          try { await PM.prepareFrame(time); prepareError = undefined; }
          catch (error: any) { prepareError = String(error?.message || error); }
          const pixels = PM.GL.renderToPixels(time, w, h, { opaque: true, mblur: false });
          PM.preparedVideoFrames = null;
          renders++;
          let hash = 2166136261;
          for (let i = 0; i < pixels.length; i++) hash = Math.imul(hash ^ pixels[i], 16777619);
          const key = String(hash >>> 0);
          streak = key === previous ? streak + 1 : 0;
          previous = key;
          if (streak >= 2) return { stable: true, renders, ...(prepareError ? { prepareError } : {}) };
          await sleep(120);
        }
        return { stable: false, renders, ...(prepareError ? { prepareError } : {}) };
      } finally {
        PM.preparedVideoFrames = null;
        PM.Export.busy = false;
      }
    },

    /** One sample: opaque RGBA8 (export path), optional transparent float RGBA and straight-alpha RGBA8, evaluated state. */
    async capture(args: { time: number; width: number; height: number; alpha: boolean; evaluate: boolean }) {
      const { time, width, height } = args;
      const gl = PM.GL.gl;
      const shutter = PM.proj.shutter || .5;
      const quality = PM.quality;
      const out: any = { time, width, height, glErrors: [], warnings: [] };
      PM.pause();
      PM.Export.busy = true;
      try {
        PM.quality = 1;
        // The exporter stops when prepareFrame throws (for example on a composition cycle);
        // the corpus records the error and still captures what the compositor draws.
        try { await PM.prepareFrame(time); }
        catch (error: any) { out.prepareError = String(error?.message || error); }
        const opaqueOptions = { mblur: true, mbSamples: 16, shutter, opaque: true, topDownOpaque: true };
        const opaque = PM.GL.renderToPixels(time, width, height, opaqueOptions);
        if (!opaque || opaque.length !== width * height * 4) throw new Error('Opaque capture returned no pixels');
        out.opaque = { options: opaqueOptions, base64: toBase64(opaque) };
        if (args.alpha) {
          const read = gl.readPixels.bind(gl);
          let floats: Float32Array | null = null;
          gl.readPixels = (...params: any[]) => {
            const result = read(...params);
            if (params[5] === gl.FLOAT && params[6] instanceof Float32Array) floats = new Float32Array(params[6]);
            return result;
          };
          const alphaOptions = { transparent: true, mblur: true, mbSamples: 20, shutter };
          let px: Uint8Array;
          try { px = PM.GL.renderToPixels(time, width, height, alphaOptions); }
          finally { gl.readPixels = read; }
          const captured = floats as Float32Array | null;
          if (!captured || captured.length !== width * height * 4) throw new Error('Transparent capture did not read float pixels');
          // Top-down rows, as the 8-bit outputs.
          const flipped = new Float32Array(captured.length), rowFloats = width * 4;
          for (let y = 0; y < height; y++) flipped.set(captured.subarray((height - 1 - y) * rowFloats, (height - y) * rowFloats), y * rowFloats);
          // Straight alpha, top-down: the exporter's alphaFrame conversion (exporter.ts).
          const straight = new Uint8Array(width * height * 4);
          for (let y = 0; y < height; y++) {
            let si = (height - 1 - y) * width * 4, di = y * width * 4;
            for (let x = 0; x < width; x++, si += 4, di += 4) {
              const a = px[si + 3]!;
              if (a === 255) { straight[di] = px[si]!; straight[di + 1] = px[si + 1]!; straight[di + 2] = px[si + 2]!; straight[di + 3] = 255; }
              else if (a === 0) { straight[di] = straight[di + 1] = straight[di + 2] = straight[di + 3] = 0; }
              else {
                straight[di] = Math.min(255, (px[si]! * 255 / a) | 0);
                straight[di + 1] = Math.min(255, (px[si + 1]! * 255 / a) | 0);
                straight[di + 2] = Math.min(255, (px[si + 2]! * 255 / a) | 0);
                straight[di + 3] = a;
              }
            }
          }
          out.alpha = { options: alphaOptions, floatBase64: toBase64(new Uint8Array(flipped.buffer)), straightBase64: toBase64(straight) };
        }
        out.glErrors = [...(PM.GL.errors?.entries?.() ?? [])].map(([key, value]: any) => `${key}: ${value}`);
        const glError = gl.getError();
        if (glError) out.glErrors.push(`gl.getError() = ${glError}`);
        if (args.evaluate) {
          const previousTime = PM.time;
          try {
            PM.time = time;
            const state = compositionState(PM.proj, time, 0);
            let scene3d = null;
            try {
              const hasScene = [PM.proj, ...Object.values<any>(PM.proj.comps || {})].some((comp: any) => comp.layers.some((l: any) => /^powermove\.3d\.(object|light|camera)$/.test(l.d?.definition || '')));
              if (hasScene) scene3d = plain(PM.Kernel.api('powermove.engine-corpus').scene3d.describe());
            } catch (error: any) { out.warnings.push(`scene3d.describe: ${error?.message || error}`); }
            const shaderMeta: Record<string, any> = {};
            for (const comp of [PM.proj, ...Object.values<any>(PM.proj.comps || {})]) for (const layer of comp.layers) {
              if (layer.type !== 'shader' && layer.type !== 'extension') continue;
              try { const meta = PM.UIState?.getShaderMeta?.(layer); if (meta) shaderMeta[layer.id] = plain(meta); } catch { /* no meta */ }
            }
            out.evaluated = { format: 'powermove-corpus-eval', version: 1, time, composition: state, scene3d, shaderMeta };
          } finally { PM.time = previousTime; PM.beginEval(NaN); }
        }
      } finally {
        PM.preparedVideoFrames = null;
        PM.Export.busy = false;
        PM.quality = quality;
      }
      return out;
    },

    shaderPresets(): Record<string, string> { return { ...PM.SHADER_PRESETS }; },

    environment(): any {
      const gl = PM.GL.gl;
      const info = gl.getExtension('WEBGL_debug_renderer_info');
      return {
        userAgent: navigator.userAgent,
        renderer: info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
        vendor: info ? gl.getParameter(info.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
        version: gl.getParameter(gl.VERSION),
        shadingLanguage: gl.getParameter(gl.SHADING_LANGUAGE_VERSION),
        maxTextureSize: gl.getParameter(gl.MAX_TEXTURE_SIZE),
        floatColorBuffer: !!gl.getExtension('EXT_color_buffer_float'),
        devicePixelRatio: window.devicePixelRatio,
        appVersion: PM.version ?? null,
      };
    },
  };
  w.__corpus = corpus;
}

/**
 * The twelve pixel-hash scenarios of e2e/rendering-performance.spec.ts
 * ("shared renderer preserves pixels across grouped previews and captures"),
 * reproduced statement for statement up to the timing measurements, which are
 * not part of the corpus. Returns the raw `renderToPixels` bytes per label.
 */
export function renderingPerformanceScenarios(): { images: Record<string, string>; flatMatches: boolean; errors: string[]; glError: number } {
  const PM = (window as any).PM, GL = PM.GL, gl = GL.gl;
  PM.pause(); PM.agentFrameCapture = true;
  const project = PM.mkProject({ name: 'Render performance', w: 640, h: 360, fps: 30, dur: 3, bg: '#182439' });
  for (let i = 0; i < 180; i++) {
    const group = PM.mkLayer('group', {}, project);
    const child = PM.mkLayer('shape', {
      d: { shape: i % 2 ? 'ellipse' : 'rect', w: 40, h: 32, radius: 7, color: i % 2 ? '#ee7733' : '#44aadd' },
      p: { 'position.x': 20 + i % 18 * 35, 'position.y': 20 + Math.floor(i / 18) * 34, opacity: 65 },
    }, project);
    child.group = group.id;
    project.layers.push(group, child);
  }
  PM.proj = project; PM.touch();
  const images: Record<string, Uint8Array> = {};
  const capture = (label: string, time: number, opaque: boolean, w = 320, h = 180) => {
    images[label] = GL.renderToPixels(time, w, h, { opaque, mblur: true, mbSamples: 3 });
  };
  capture('groups', 0, true);
  const groupedPixels = images.groups!;
  const groupedLayers = project.layers;
  project.layers = project.layers.filter((layer: any) => layer.type !== 'group');
  const owners = project.layers.map((layer: any) => layer.group);
  project.layers.forEach((layer: any) => { layer.group = null; }); PM.touch();
  const flatPixels = GL.renderToPixels(0, 320, 180, { opaque: true, mblur: true, mbSamples: 3 });
  const flatMatches = groupedPixels.every((value: number, i: number) => value === flatPixels[i]);
  project.layers.forEach((layer: any, i: number) => { layer.group = owners[i]; });
  project.layers = groupedLayers; PM.touch();
  const group = project.layers[0], child = project.layers[1];
  group.p.opacity.v = 43;
  group.blend = 'screen';
  PM.animate(child, 'rotation', [{ t: 0, v: 0 }, { t: 2, v: 130 }]);
  child.mblur = true; PM.touch();
  capture('styled', .75, true);
  capture('alpha', .75, false);
  capture('resize', .25, true, 160, 90);
  capture('reverse', 0, true);
  const mask = PM.mkMask('rect', project);
  mask.p.x.v = 28; mask.p.y.v = 25; mask.p.w.v = 40; mask.p.h.v = 35;
  group.masks.push(mask); PM.touch();
  capture('mask', .75, true);
  group.solo = true; PM.touch(); capture('solo', 1, true);
  group.solo = false; group.masks = []; group.blend = 'normal'; group.p.opacity.v = 100; child.mblur = false; PM.touch();
  project.backgroundFill = { type: 'none', stops: [], angle: 0 }; PM.touch();
  capture('transparent-background', .75, false);
  group.fx.push(PM.mkEffect('blur')); PM.touch(); capture('group-effect', .75, false);
  group.fx = [];
  group.transitionIn = { type: 'crossfade', dur: .5, p: {} }; PM.touch(); capture('transition', .25, true);
  group.transitionIn = null;
  group.matteSource = project.layers[3].id; PM.touch(); capture('matte', .75, false);
  group.matteSource = null;
  const outer = PM.mkLayer('group', {}, project); outer.p.opacity.v = 70;
  group.group = outer.id; project.layers.unshift(outer); PM.touch(); capture('nested-boundary', .75, false);
  const encoded: Record<string, string> = {};
  for (const [label, bytes] of Object.entries(images)) {
    let text = '';
    for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + 0x8000)));
    encoded[label] = btoa(text);
  }
  return { images: encoded, flatMatches, errors: [...GL.errors].map(String), glError: gl.getError() };
}
