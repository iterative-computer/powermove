import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { expect, test, repoRoot, type LaunchedApp } from './helpers/app';

type AlphaFixture = { depth: number; width: number; height: number; frames: number; quality: number; grouped?: boolean; static?: boolean; media?: boolean; coldEffect?: boolean; coldMask?: 'analytic' | 'path' };

async function alphaScene(session: LaunchedApp, fixture: AlphaFixture) {
  return session.page.evaluate(async fixture => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.AutomaticPreview?.cancel(); PM.quality = fixture.quality;
    const project = PM.mkProject({ name: 'Deferred transparent delivery', w: fixture.width, h: fixture.height, dur: 1, fps: fixture.frames });
    project.colorDepth = fixture.depth; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    project.layers = Array.from({ length: 24 }, (_, index) => PM.mkLayer('shape', {
      d: { shape: index % 3 ? 'rect' : 'ellipse', w: fixture.width * .16, h: fixture.height * .24,
        radius: 23, color: index % 2 ? '#a34bf790' : '#2bd773cf' },
      p: { 'position.x': fixture.width * (.09 + index % 8 * .115),
        'position.y': fixture.height * (.2 + Math.floor(index / 8) * .29), opacity: 37 + index % 5 * 11, rotation: index * 7 },
    }, project));
    if (fixture.coldEffect) {
      const blur = PM.mkEffect('blur'); blur.p.amount.v = 13;
      project.layers[0].fx = [blur];
    }
    if (fixture.coldMask) {
      const mask = PM.mkMask('rect', project);
      mask.p.w.v = 211; mask.p.h.v = 173; mask.p.feather.v = 11;
      if (fixture.coldMask === 'path') {
        const property = (values: Record<string, unknown>) =>
          Object.fromEntries(Object.entries(values).map(([key, value]) => [key, PM.P(value)]));
        mask.path = { id: 'cold-alpha-mask', name: 'Cold cubic mask', parent: null,
          p: property({ x: 0, y: 0, rotation: 7, scaleX: 100, scaleY: 100, closed: true,
            fill: '#ffffff', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2,
            strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 0, repeatY: 0, repeatRotation: 0 }),
          vertices: [[-105, -86], [105, -86], [105, 86], [-105, 86]].map(([x, y], index) => ({
            id: `cold-alpha-vertex-${index}`, p: property({ x, y,
              inX: index === 0 ? -23 : 0, inY: index === 2 ? 19 : 0, outX: index === 1 ? 29 : 0, outY: index === 3 ? -17 : 0 }),
          })),
        };
      }
      project.layers[0].masks = [mask];
    }
    if (fixture.grouped) {
      const group = PM.mkLayer('group', { p: { opacity: 73, rotation: 13 } }, project);
      group.mblur = true;
      const glow = PM.mkEffect('glow'); glow.p.radius.v = 15; group.fx = [glow];
      for (const layer of project.layers) { layer.group = group.id; layer.mblur = true; }
      const mask = PM.mkMask('rect', project), property = (values: Record<string, unknown>) =>
        Object.fromEntries(Object.entries(values).map(([key, value]) => [key, PM.P(value)]));
      mask.p.feather.v = 17;
      mask.path = { id: 'alpha-mask-path', name: 'Alpha cubic mask', parent: null,
        p: property({ x: 0, y: 0, rotation: 7, scaleX: 100, scaleY: 100, closed: true,
          fill: '#ffffff', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2,
          strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 0, repeatY: 0, repeatRotation: 0 }),
        vertices: [[-.44, -.36], [.44, -.36], [.44, .36], [-.44, .36]].map(([x, y], index) => ({
          id: `alpha-mask-vertex-${index}`, p: property({ x: x! * fixture.width, y: y! * fixture.height,
            inX: index === 0 ? -77 : 0, inY: index === 2 ? 51 : 0, outX: index === 1 ? 89 : 0, outY: index === 3 ? -43 : 0 }),
        })),
      };
      group.masks = [mask]; project.layers.unshift(group); project.shutter = -.75;
    }
    if (fixture.media) {
      const canvas = document.createElement('canvas'); canvas.width = 96; canvas.height = 64;
      const context = canvas.getContext('2d')!;
      context.fillStyle = '#f6427a'; context.fillRect(0, 0, 96, 64);
      const bitmap = await createImageBitmap(canvas), original = PM.assets.get;
      const asset = { id: 'alpha-immutable-image', type: 'image', format: 'png', w: 96, h: 64, el: bitmap };
      PM.assets.get = (id: string) => id === asset.id ? asset : original.call(PM.assets, id);
      project.layers.push(PM.mkLayer('image', { d: { asset: asset.id, w: 400, h: 300 },
        p: { 'position.x': fixture.width / 2, 'position.y': fixture.height / 2, opacity: 47 } }, project));
      (window as any).__disposeAlphaMedia = () => { PM.assets.get = original; bitmap.close(); };
    }
    PM.proj = project; PM.touch();
    if (!fixture.static) for (const [index, layer] of project.layers.entries()) if (layer.type === 'shape')
      PM.animate(layer, 'rotation', [{ t: 0, v: index * 7 }, { t: 1, v: index * 7 + 35 + index }]);
    if (fixture.grouped) {
      const group = project.layers[0];
      PM.animate(group, 'rotation', [{ t: 0, v: 13 }, { t: 1, v: 31 }]);
      PM.animate(group, 'mask.feather', [{ t: 0, v: 7 }, { t: 1, v: 23 }], { prop: group.masks[0].p.feather });
    }
    await PM.prepareFrame(0);
    if (typeof PM.GL.createAlphaReadback !== 'function') throw new Error('Production asynchronous alpha factory missing');
    const probe = PM.GL.createAlphaReadback(fixture.width, fixture.height), supported = !!probe;
    probe?.dispose();
    // Its structural static plan is valid even while the shader has not yet
    // become available to the retained-frame key. Keep that first frame serial.
    if (fixture.coldEffect) PM.GL.dropProgram('fx:blur');
    return { supported };
  }, fixture);
}

async function alphaOutput(session: LaunchedApp, output: string, hold: 'none' | 'ack' | 'failure' = 'none') {
  await session.app.evaluate(({ ipcMain, dialog }, { output, hold }) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
    const ipc = ipcMain as any, crypto = process.getBuiltinModule('node:crypto');
    const originals = (globalThis as any).__alphaAsyncHandlers ||= new Map(
      ['start', 'write', 'finish', 'cancel'].map(name => [`render:${name}`, ipc._invokeHandlers.get(`render:${name}`)]));
    const hash = crypto.createHash('sha256');
    let frameHash = crypto.createHash('sha256'), frameSize = 0, frameOffset = 0;
    const state = { active: 0, maxActive: 0, bytes: 0, writes: 0, finishes: 0, cancels: 0, repeats: 0,
      held: false, rawHash: '', frames: [] as string[] };
    (globalThis as any).__alphaAsyncWrites = state;
    for (const [channel, original] of originals) {
      ipcMain.removeHandler(channel);
      ipcMain.handle(channel, async (...args: any[]) => {
        state.maxActive = Math.max(state.maxActive, ++state.active);
        try {
          if (channel === 'render:start') frameSize = args[1].width * args[1].height * 4;
          if (channel === 'render:write') {
            state.writes++;
            if (args[1].repeat !== undefined) state.repeats++;
            else {
              const bytes = args[1].data;
              hash.update(bytes); state.bytes += bytes.length;
              for (let at = 0; at < bytes.length;) {
                const count = Math.min(frameSize - frameOffset, bytes.length - at);
                frameHash.update(bytes.subarray(at, at + count)); at += count; frameOffset += count;
                if (frameOffset === frameSize) { state.frames.push(frameHash.digest('hex')); frameHash = crypto.createHash('sha256'); frameOffset = 0; }
              }
            }
            if (state.writes === 1 && hold !== 'none') {
              state.held = true;
              await new Promise<void>(resolve => { (globalThis as any).__releaseAlphaAsync = () => { state.held = false; resolve(); }; });
              if (hold === 'failure') throw new Error('Synthetic earlier alpha write failure');
            }
          }
          if (channel === 'render:finish') state.finishes++;
          if (channel === 'render:cancel') state.cancels++;
          const result = await original(...args);
          if (channel === 'render:finish') state.rawHash = hash.digest('hex');
          return result;
        } finally { state.active--; }
      });
    }
  }, { output, hold });
}

async function runAlpha(session: LaunchedApp, fixture: AlphaFixture, enabled: boolean, fault: 'none' | 'guard' | 'loss' = 'none') {
  return session.page.evaluate(async ({ fixture, enabled, fault }) => {
    const PM = (window as any).PM, gl = PM.GL.gl, factory = PM.GL.createAlphaReadback;
    const originals = { bufferData: gl.bufferData, deleteBuffer: gl.deleteBuffer, fenceSync: gl.fenceSync, deleteSync: gl.deleteSync,
      getBufferSubData: gl.getBufferSubData, renderProject: PM.GL.renderProject,
      put: CanvasRenderingContext2D.prototype.putImageData, get: CanvasRenderingContext2D.prototype.getImageData,
      draw: CanvasRenderingContext2D.prototype.drawImage };
    const buffers = new Set<WebGLBuffer>(), fences = new Set<WebGLSync>(), targets = new Set<Uint8Array>();
    const surfaces = new Set<HTMLCanvasElement>(), source = PM.serialize(), time = PM.time, quality = PM.quality;
    let renderDepth = 0;
    const stats = { factories: 0, sessions: 0, begins: [] as number[], reads: [] as number[], disposals: 0, ticketDisposals: 0,
      pboAllocations: 0, pboDeletes: 0, pboBytes: 0, maxFences: 0, maxTickets: 0, activeTickets: 0,
      puts: 0, gets: 0, previews: 0, firstColors: 0, partial: false, corrupted: false,
      captures: [] as Array<{ quality: number; transparent: boolean; samples: number; shutter: number; mblur: boolean }> };
    (window as any).__alphaAsyncStats = stats;
    gl.bufferData = function (...args: any[]) {
      const result = originals.bufferData.apply(gl, args);
      if (args[0] === gl.PIXEL_PACK_BUFFER) {
        buffers.add(gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING)); stats.pboAllocations++; stats.pboBytes += args[1];
      }
      return result;
    };
    gl.deleteBuffer = function (buffer: WebGLBuffer) {
      if (buffers.delete(buffer)) stats.pboDeletes++;
      return originals.deleteBuffer.call(gl, buffer);
    };
    gl.fenceSync = function (...args: any[]) {
      const fence = originals.fenceSync.apply(gl, args); if (fence) fences.add(fence);
      stats.maxFences = Math.max(stats.maxFences, fences.size); return fence;
    };
    gl.deleteSync = function (fence: WebGLSync) { fences.delete(fence); return originals.deleteSync.call(gl, fence); };
    gl.getBufferSubData = function (...args: any[]) {
      const result = originals.getBufferSubData.apply(gl, args);
      if (fault === 'guard' && !stats.corrupted && args[0] === gl.PIXEL_PACK_BUFFER) {
        args[2][args[2].length - 1] ^= 1; stats.corrupted = true;
      }
      return result;
    };
    PM.GL.renderProject = function (...args: any[]) {
      const outer = renderDepth++ === 0;
      try {
        if (outer) stats.captures.push({ quality: PM.quality, transparent: args[4]?.transparent === true, samples: args[4]?.mbSamples,
          shutter: args[4]?.shutter, mblur: args[4]?.mblur });
        return originals.renderProject.apply(this, args);
      } finally { renderDepth--; }
    };
    CanvasRenderingContext2D.prototype.putImageData = function (...args: any[]) {
      if (args[0].width === fixture.width && args[0].height === fixture.height) { stats.puts++; surfaces.add(this.canvas); }
      return (originals.put as any).apply(this, args);
    };
    CanvasRenderingContext2D.prototype.getImageData = function (...args: any[]) {
      const data = (originals.get as any).apply(this, args);
      if (args[2] === fixture.width && args[3] === fixture.height) {
        stats.gets++;
        if (stats.gets === 1) {
          const colors = new Set<number>();
          for (let at = 0; at < data.data.length && (colors.size <= 32 || !stats.partial); at += 4) {
            colors.add((data.data[at] << 24) | (data.data[at + 1] << 16) | (data.data[at + 2] << 8) | data.data[at + 3]);
            stats.partial ||= data.data[at + 3] > 0 && data.data[at + 3] < 255;
          }
          stats.firstColors = colors.size;
        }
      }
      return data;
    };
    CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) {
      if (surfaces.has(args[0]) && this.canvas.width < fixture.width) stats.previews++;
      return (originals.draw as any).apply(this, args);
    };
    PM.GL.createAlphaReadback = enabled ? (...args: any[]) => {
      stats.factories++;
      const readback = factory(...args); if (!readback) return readback;
      stats.sessions++;
      return { byteLength: readback.byteLength,
        begin(time: number, options: any) {
          const ticket = readback.begin(time, options); if (!ticket) return null;
          stats.begins.push(time); stats.maxTickets = Math.max(stats.maxTickets, ++stats.activeTickets);
          let disposed = false;
          return { readInto(target: Uint8Array, cancelled: () => boolean) {
            targets.add(target); stats.reads.push(time);
            if (target.length !== fixture.width * (fixture.height + 1) * 4) throw new Error('Incorrect guarded alpha target');
            const reading = ticket.readInto(target, cancelled);
            if (fault === 'loss') (window as any).__alphaAsyncLoss.loseContext();
            return reading;
          }, dispose() {
            if (!disposed) { disposed = true; stats.activeTickets--; stats.ticketDisposals++; }
            ticket.dispose();
          } };
        }, dispose() { stats.disposals++; readback.dispose(); },
      };
    } : undefined;
    try {
      if (fixture.coldMask) {
        const programs = fixture.coldMask === 'path' ? ['pathMaskApply'] : ['masks', 'maskApply'];
        for (const program of programs) {
          PM.GL.dropProgram(program);
          if (PM.GL.progs.has(program)) throw new Error('The mask program must be cold before export');
        }
      }
      const result = await PM.Export.run({ format: 'prores', alpha: true, scale: 1, fps: fixture.frames,
        range: 'all', audio: false, mblur: !!fixture.grouped });
      return { result, ...stats, targets: targets.size, activeBuffers: buffers.size, activeFences: fences.size, surfaces: surfaces.size,
        released: [...surfaces].every(canvas => canvas.width === 0 && canvas.height === 0),
        preserved: source === PM.serialize() && time === PM.time && quality === PM.quality, busy: PM.Export.busy };
    } finally {
      PM.GL.createAlphaReadback = factory;
      gl.bufferData = originals.bufferData; gl.deleteBuffer = originals.deleteBuffer; gl.fenceSync = originals.fenceSync;
      gl.deleteSync = originals.deleteSync; gl.getBufferSubData = originals.getBufferSubData; PM.GL.renderProject = originals.renderProject;
      CanvasRenderingContext2D.prototype.putImageData = originals.put;
      CanvasRenderingContext2D.prototype.getImageData = originals.get;
      CanvasRenderingContext2D.prototype.drawImage = originals.draw;
    }
  }, { fixture, enabled, fault });
}

function decoded(filePath: string) {
  return execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'),
    ['-v', 'error', '-i', filePath, '-pix_fmt', 'rgba', '-f', 'framehash', '-hash', 'sha256', 'pipe:1'],
    { encoding: 'utf8', maxBuffer: 1024 * 1024 });
}

for (const depth of [16, 32]) for (const fixture of [
  { depth, width: 1920, height: 1080, frames: 24, quality: .5 },
  { depth, width: 3840, height: 2160, frames: 12, quality: .25 },
  { depth, width: 1920, height: 1080, frames: 12, quality: .5, grouped: true },
]) test(`native async alpha ${depth}-bit ${fixture.width}×${fixture.height}${fixture.grouped ? ' grouped path/FX with negative shutter' : ''} preserves current quality and every delivered pixel`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { supported } = await alphaScene(session, fixture);
  let referenceRaw: string | undefined, referenceDecoded: string | undefined;
  for (const enabled of [false, true]) {
    const output = path.join(session.userData, `alpha-async-${enabled}.mov`);
    await alphaOutput(session, output);
    const result = await runAlpha(session, fixture, enabled);
    expect(result.result).toEqual({ cancelled: false }); expect(result.preserved).toBe(true);
    expect(result).toMatchObject({ puts: fixture.frames, gets: fixture.frames, previews: fixture.frames,
      surfaces: 1, released: true, activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
    expect(result.partial).toBe(true); expect(result.firstColors).toBeGreaterThan(32);
    expect(result.captures).toEqual(Array.from({ length: fixture.frames }, () => ({ quality: fixture.quality,
      transparent: true, samples: 20, shutter: fixture.grouped ? -.75 : .5, mblur: !!fixture.grouped })));
    if (enabled && supported) {
      expect(result).toMatchObject({ sessions: 1, disposals: 1, pboAllocations: 1, pboDeletes: 1,
        pboBytes: fixture.width * (fixture.height + 1) * 4, targets: 1, maxTickets: 1, maxFences: 1 });
      expect(result.begins).toEqual(Array.from({ length: fixture.frames }, (_, index) => index / fixture.frames));
      expect(result.reads).toEqual(result.begins);
    } else expect(result).toMatchObject({ sessions: 0, disposals: 0, targets: 0, pboAllocations: 0 });
    const main = await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites);
    expect(main).toMatchObject({ active: 0, maxActive: 1, bytes: fixture.width * fixture.height * 4 * fixture.frames,
      finishes: 1, cancels: 0, repeats: 0 });
    const frames = decoded(output), records = frames.split('\n').filter(line => line && !line.startsWith('#'));
    expect(records).toHaveLength(fixture.frames);
    expect(new Set(records.map(line => line.split(',').at(-1))).size).toBeGreaterThan(1);
    for (const record of records) expect(Number(record.split(',')[4])).toBe(fixture.width * fixture.height * 4);
    if (referenceRaw === undefined) { referenceRaw = main.rawHash; referenceDecoded = frames; }
    else { expect(main.rawHash).toBe(referenceRaw); expect(frames).toBe(referenceDecoded); }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const depth of [16, 32]) test(`native ${depth}-bit alpha guard corruption recaptures the same timestamp without cancellation`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const fixture = { depth, width: 1920, height: 1080, frames: 12, quality: .5 };
  test.skip(!(await alphaScene(session, fixture)).supported, 'Validated asynchronous alpha readback unavailable');
  const hashes: string[] = [], outputs: string[] = [];
  for (const guard of [false, true]) {
    const output = path.join(session.userData, `alpha-guard-${guard}.mov`); outputs.push(output);
    await alphaOutput(session, output);
    const result = await runAlpha(session, fixture, guard, guard ? 'guard' : 'none');
    expect(result.result).toEqual({ cancelled: false });
    expect(result).toMatchObject({ preserved: true, activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
    if (guard) {
      expect(result.corrupted).toBe(true); expect(result.begins).toEqual([0]); expect(result.reads).toEqual([0]);
      expect(result).toMatchObject({ sessions: 1, disposals: 1, pboAllocations: 1, pboDeletes: 1, targets: 1 });
      expect(result.captures).toHaveLength(fixture.frames + 1);
      // The rejected raw frame never reaches the Canvas or encoder.
      expect(result.puts).toBe(fixture.frames); expect(result.gets).toBe(fixture.frames);
    }
    const main = await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites);
    expect(main).toMatchObject({ finishes: 1, cancels: 0, bytes: fixture.width * fixture.height * 4 * fixture.frames });
    hashes.push(main.rawHash);
  }
  expect(hashes[1]).toBe(hashes[0]); expect(decoded(outputs[1]!)).toBe(decoded(outputs[0]!));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('native alpha discards a queued frame after render quality changes during the previous encoder ACK', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const fixture = { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5 };
  test.skip(!(await alphaScene(session, fixture)).supported, 'Validated asynchronous alpha readback unavailable');
  // Keep the same literal project/time so only the quality stamp invalidates
  // the queued frame. Quality can affect raster choices without changing every
  // scene's pixels, so capture quality and ticket disposal are checked too.
  await session.page.evaluate(() => {
    const PM = (window as any).PM, blur = PM.mkEffect('blur'); blur.p.amount.v = 13;
    PM.proj.layers[0].fx = [blur]; PM.touch();
  });
  const referenceFrames: string[][] = [], referenceDecoded: string[][] = [];
  for (const quality of [.5, .25]) {
    await session.page.evaluate(quality => { (window as any).PM.quality = quality; }, quality);
    const output = path.join(session.userData, `quality-reference-${quality}.mov`);
    await alphaOutput(session, output); expect((await runAlpha(session, fixture, false)).result).toEqual({ cancelled: false });
    referenceFrames.push(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.frames));
    referenceDecoded.push(decoded(output).split('\n').filter(line => line && !line.startsWith('#')));
  }
  await session.page.evaluate(() => { (window as any).PM.quality = .5; });
  const output = path.join(session.userData, 'alpha-quality-edit.mov');
  await alphaOutput(session, output, 'ack');
  const running = runAlpha(session, fixture, true);
  try {
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.held)).toBe(true);
    await expect.poll(() => session.page.evaluate(() => (window as any).__alphaAsyncStats.begins.length)).toBe(2);
    await session.page.evaluate(() => { (window as any).PM.quality = .25; });
    await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync());
    const result = await running;
    expect(result.result).toEqual({ cancelled: false });
    expect(result.begins.slice(0, 2)).toEqual([0, 1 / fixture.frames]);
    expect(result.reads[0]).toBe(0); expect(result.reads.filter(time => time === 1 / fixture.frames)).toHaveLength(0);
    expect(result.captures).toHaveLength(fixture.frames + 1);
    expect(result.captures.slice(0, 2).every(capture => capture.quality === .5)).toBe(true);
    expect(result.captures.slice(2).every(capture => capture.quality === .25)).toBe(true);
    expect(result).toMatchObject({ activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
    const main = await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites);
    expect(main.frames).toEqual([referenceFrames[0]![0], ...referenceFrames[1]!.slice(1)]);
    expect(decoded(output).split('\n').filter(line => line && !line.startsWith('#')))
      .toEqual([referenceDecoded[0]![0], ...referenceDecoded[1]!.slice(1)]);
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync?.());
    await running;
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const failure of [false, true]) test(`native alpha drains the previous write before ${failure ? 'reporting its error' : 'cancelling'} and retries`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const fixture = { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5 };
  test.skip(!(await alphaScene(session, fixture)).supported, 'Validated asynchronous alpha readback unavailable');
  await alphaOutput(session, path.join(session.userData, 'alpha-held.mov'), failure ? 'failure' : 'ack');
  const running = runAlpha(session, fixture, true);
  try {
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.held)).toBe(true);
    await expect.poll(() => session.page.evaluate(() => (window as any).__alphaAsyncStats.begins.length)).toBe(2);
    await session.page.evaluate(() => { (window as any).PM.Export.cancel = true; });
    await session.page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    expect(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites)).toMatchObject({ held: true, writes: 1, finishes: 0, cancels: 0 });
    await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync());
    const result = await running;
    if (failure) expect(result.result.error).toContain('Synthetic earlier alpha write failure');
    else expect(result.result).toEqual({ cancelled: true });
    expect(result).toMatchObject({ activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
    expect(result.reads).toEqual([0]);
    expect(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites)).toMatchObject({ held: false, writes: failure ? 1 : 2, finishes: 0, cancels: 1 });
  } finally {
    await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync?.());
    await running;
  }
  await alphaOutput(session, path.join(session.userData, 'alpha-held-retry.mov'));
  expect((await runAlpha(session, fixture, true)).result).toEqual({ cancelled: false });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const depth of [16, 32]) test(`native asynchronous ${depth}-bit alpha rejects real GPU loss before delivery and recovers`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const fixture = { depth, width: 1920, height: 1080, frames: 12, quality: .5 };
  test.skip(!(await alphaScene(session, fixture)).supported, 'Validated asynchronous alpha readback unavailable');
  test.skip(!await session.page.evaluate(() => {
    (window as any).__alphaAsyncLoss = (window as any).PM.GL.gl.getExtension('WEBGL_lose_context');
    return !!(window as any).__alphaAsyncLoss;
  }), 'GPU reset extension unavailable');
  await alphaOutput(session, path.join(session.userData, 'alpha-async-loss.mov'));
  const failed = await runAlpha(session, fixture, true, 'loss');
  expect(failed.result).toEqual({ error: 'The GPU could not capture the export frame' });
  expect(failed).toMatchObject({ sessions: 1, disposals: 1, activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
  expect(failed.reads).toEqual([0]);
  expect(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites)).toMatchObject({ writes: 0, finishes: 0, cancels: 1 });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.GL.contextLost)).toBe(true);
  await session.page.evaluate(() => (window as any).__alphaAsyncLoss.restoreContext());
  await expect.poll(() => session.page.evaluate(() => !!(window as any).PM.GL.gl && !(window as any).PM.GL.contextLost)).toBe(true);
  const outputs: string[] = [], hashes: string[] = [];
  for (const enabled of [false, true]) {
    const output = path.join(session.userData, `alpha-async-recovered-${enabled}.mov`); outputs.push(output);
    await alphaOutput(session, output);
    expect((await runAlpha(session, fixture, enabled)).result).toEqual({ cancelled: false });
    hashes.push(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.rawHash));
  }
  expect(hashes[1]).toBe(hashes[0]); expect(decoded(outputs[1]!)).toBe(decoded(outputs[0]!));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('native alpha keeps short, static, small, 8-bit and media captures on their existing route', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  for (const fixture of [
    { depth: 16, width: 1920, height: 1080, frames: 2, quality: .5 },
    { depth: 32, width: 1920, height: 1080, frames: 12, quality: .25, static: true },
    { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5, static: true, coldEffect: true },
    { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5, static: true, coldMask: 'analytic' as const },
    { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5, static: true, coldMask: 'path' as const },
    { depth: 16, width: 640, height: 360, frames: 12, quality: .5 },
    { depth: 8, width: 1920, height: 1080, frames: 12, quality: .5 },
    { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5, media: true },
  ]) {
    await alphaScene(session, fixture);
    try {
      await alphaOutput(session, path.join(session.userData, `alpha-fallback-${fixture.depth}-${fixture.width}-${fixture.frames}-${!!fixture.static}-${!!fixture.media}-${!!fixture.coldEffect}-${fixture.coldMask || 'none'}.mov`));
      const result = await runAlpha(session, fixture, true);
      expect(result.result).toEqual({ cancelled: false });
      expect(result).toMatchObject({ factories: 0, sessions: 0, pboAllocations: 0, targets: 0, activeBuffers: 0, activeFences: 0, busy: false });
      expect(result.preserved).toBe(true);
    } finally { await session.page.evaluate(() => { (window as any).__disposeAlphaMedia?.(); (window as any).__disposeAlphaMedia = undefined; }); }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const change of ['default', 'provider'] as const) test(`native alpha discards stale GPU pixels after an effect ${change} changes during the previous ACK`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const fixture = { depth: 16, width: 1920, height: 1080, frames: 12, quality: .5 };
  test.skip(!(await alphaScene(session, fixture)).supported, 'Validated asynchronous alpha readback unavailable');
  await session.page.evaluate(change => {
    const PM = (window as any).PM, blur = PM.mkEffect('blur');
    blur.p.amount.v = 13; PM.proj.layers[0].fx = [blur];
    const definition = structuredClone(PM.Kernel.effects.get('blur'));
    const parameter = PM.Kernel.effects.get('blur').params.find((param: any) => param.k === 'amount'), oldDefault = parameter.def;
    if (change === 'default') delete blur.p.amount;
    const output = 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));';
    if (!definition.frag.includes(output)) throw new Error('Builtin blur output changed');
    definition.frag = definition.frag.replace(output, 'o=gaussian(sigma,repeatEdges,inverse(u_layerUV));o.rgb*=vec3(.1,.8,.3);');
    let registration: any;
    (window as any).__changeAlphaEffect = () => {
      if (change === 'default') parameter.def = 89;
      else registration = PM.Kernel.registerEffect('effects-basic', definition);
    };
    (window as any).__restoreAlphaEffect = () => {
      parameter.def = oldDefault; registration?.dispose(); registration = undefined;
    };
    PM.touch();
  }, change);
  try {
    const raw: string[][] = [], frames: string[][] = [];
    for (const changed of [false, true]) {
      await session.page.evaluate(changed => {
        if (changed) (window as any).__changeAlphaEffect();
        else (window as any).__restoreAlphaEffect();
      }, changed);
      const output = path.join(session.userData, `alpha-effect-reference-${changed}.mov`);
      await alphaOutput(session, output);
      expect((await runAlpha(session, fixture, false)).result).toEqual({ cancelled: false });
      raw.push(await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.frames));
      frames.push(decoded(output).split('\n').filter(line => line && !line.startsWith('#')));
    }
    expect(raw[0]![1]).not.toBe(raw[1]![1]);
    await session.page.evaluate(() => (window as any).__restoreAlphaEffect());
    const output = path.join(session.userData, 'alpha-effect-edit.mov');
    await alphaOutput(session, output, 'ack');
    const running = runAlpha(session, fixture, true);
    try {
      await expect.poll(() => session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites.held)).toBe(true);
      await expect.poll(() => session.page.evaluate(() => (window as any).__alphaAsyncStats.begins.length)).toBe(2);
      await session.page.evaluate(() => (window as any).__changeAlphaEffect());
      await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync());
      const result = await running;
      expect(result.result).toEqual({ cancelled: false });
      expect(result.reads[0]).toBe(0); expect(result.reads.filter(time => time === 1 / fixture.frames)).toHaveLength(0);
      expect(result).toMatchObject({ activeBuffers: 0, activeFences: 0, activeTickets: 0, busy: false });
      const main = await session.app.evaluate(() => (globalThis as any).__alphaAsyncWrites);
      expect(main.frames).toEqual([raw[0]![0], ...raw[1]!.slice(1)]);
      expect(decoded(output).split('\n').filter(line => line && !line.startsWith('#'))).toEqual([frames[0]![0], ...frames[1]!.slice(1)]);
    } finally {
      await session.app.evaluate(() => (globalThis as any).__releaseAlphaAsync?.());
      await running;
    }
  } finally { await session.page.evaluate(() => (window as any).__restoreAlphaEffect?.()); }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
