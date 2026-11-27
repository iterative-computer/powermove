/** Count legacy WebGL composition work for the same captured projects as the
 * native color-depth counter example. The caller owns the Electron lease.
 * Each sample uses a fresh app profile and WebGL context.
 */
import path from 'node:path';
import { readFileSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';

const directory = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || directory === repoRoot || directory.startsWith(repoRoot + path.sep))
  throw new Error('Supply the external seven-project parity capture directory');
const manifest = JSON.parse(readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
const rows: unknown[] = [];
for (const sample of manifest.samples) {
  if (sample.depth === 32) continue;
  if (![8, 16].includes(sample.depth) || !/^[a-z0-9-]+$/.test(sample.id)
    || sample.width !== 320 || sample.height !== 180 || sample.time !== 0.5)
    throw new Error(`Invalid counter sample: ${sample.id}`);
  const doc = JSON.parse(readFileSync(path.join(directory, `${sample.id}.json`), 'utf8'));
  const app = await launchLegacyApp();
  try {
    const row = await app.evaluate(async ({ doc, sample }) => {
      const PM = (window as any).PM;
      if (!PM?.GL?.gl) throw new Error('Legacy WebGL compositor unavailable');
      const GL = PM.GL, gl = GL.gl;
      const priorAgentCapture = PM.agentFrameCapture;
      const priorFramebuffer = gl.getParameter(gl.FRAMEBUFFER_BINDING);
      const priorViewport = gl.getParameter(gl.VIEWPORT);
      const originals = new Map<string, (...args: any[]) => any>();
      let counts: Record<string, number> = {};
      let drawFramebuffer = priorFramebuffer;
      const add = (key: string, n = 1) => { counts[key] = (counts[key] ?? 0) + n; };
      const positive = (value: unknown, label: string) => {
        if (!Number.isSafeInteger(value) || (value as number) <= 0)
          throw new Error(`Cannot count ${label}: ${String(value)}`);
        return value as number;
      };
      const dimensions = (source: any): [number, number] => {
        const width = source?.videoWidth || source?.naturalWidth || source?.displayWidth || source?.width;
        const height = source?.videoHeight || source?.naturalHeight || source?.displayHeight || source?.height;
        return [positive(width, 'image width'), positive(height, 'image height')];
      };
      const payloadBytes = (value: any): number => {
        if (value == null || typeof value === 'number') return 0; // Null storage or a buffer-size/offset argument.
        if (ArrayBuffer.isView(value)) return value.byteLength;
        if (value instanceof ArrayBuffer) return value.byteLength;
        if (Array.isArray(value)) return value.length * 4;
        throw new Error(`Cannot count upload payload: ${Object.prototype.toString.call(value)}`);
      };
      const uniformArrayBytes = (args: any[], matrix: boolean): number => {
        const data = args[matrix ? 2 : 1];
        const offset = args[matrix ? 3 : 2] ?? 0;
        const length = args[matrix ? 4 : 3] ?? (data?.length - offset);
        if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length)
          || length < 0 || offset + length > data?.length)
          throw new Error('Cannot count uniform array payload');
        return length * 4;
      };
      const imageBytes = (source: any): number => {
        if (source == null) return 0;
        if (typeof source === 'number') throw new Error('Pixel-unpack buffer offset needs separate byte accounting');
        if (ArrayBuffer.isView(source)) return source.byteLength;
        const [width, height] = dimensions(source);
        // DOM image/canvas/video sources enter texImage2D as RGBA8 pixels.
        return width * height * 4;
      };
      const storageBytesPerPixel = (format: number): number => {
        const formats = new Map<number, number>([
          [gl.RGBA, 4], [gl.RGBA8, 4], [gl.RGBA8UI, 4],
          [gl.RGBA16F, 8], [gl.RGBA32F, 16],
          [gl.RGB, 3], [gl.RGB8, 3], [gl.R8, 1], [gl.RG8, 2],
          [gl.R16F, 2], [gl.RG16F, 4], [gl.R32F, 4], [gl.RG32F, 8],
          [gl.DEPTH_COMPONENT16, 2], [gl.DEPTH_COMPONENT24, 4],
          [gl.DEPTH24_STENCIL8, 4],
        ]);
        const bytes = formats.get(format);
        if (!bytes) throw new Error(`Cannot count texture storage format 0x${format.toString(16)}`);
        return bytes;
      };
      const hook = (name: string, observe: (args: any[]) => void) => {
        const original = gl[name];
        if (typeof original !== 'function') throw new Error(`Missing WebGL method ${name}`);
        originals.set(name, original);
        gl[name] = function (...args: any[]) { observe(args); return original.apply(this, args); };
      };
      try {
        PM.pause();
        // replaceProject can schedule a preview; block it before replacing the
        // document so the first measured render really owns the first targets.
        PM.agentFrameCapture = true;
        PM.replaceProject(doc);
        PM.setTime(sample.time, { raw: true, force: true });
        await PM.projectAssetsReady;
        await PM.prepareFrame(sample.time);
        if (GL.gl !== gl) throw new Error('WebGL context changed during sample setup');
        if (PM.proj?.colorDepth !== sample.depth || PM.proj?.w !== sample.width || PM.proj?.h !== sample.height)
          throw new Error(`Wrong project depth/dimensions: ${PM.proj?.colorDepth}, ${PM.proj?.w}x${PM.proj?.h}`);
        const initialCache = {
          workingTargets: GL.pool.length,
          sourceTextures: GL.texes.size,
          programs: GL.progs.size,
        };
        if (GL.pool.some((f: any) => f.w === sample.width && f.h === sample.height && f.colorDepth === sample.depth))
          throw new Error('Cold render would reuse a pre-existing 320x180 working target');
        if (GL.texes.size !== 0) throw new Error('Cold render would reuse pre-existing source textures');
        if (gl.getError() !== gl.NO_ERROR) throw new Error('WebGL error before cold render');

        for (const name of ['drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced'])
          hook(name, () => add('drawCalls'));
        hook('clear', () => add('clearCalls'));
        hook('bindFramebuffer', args => {
          if ((args[0] === gl.FRAMEBUFFER || args[0] === gl.DRAW_FRAMEBUFFER) && args[1] !== drawFramebuffer) {
            add('framebufferTargetChanges');
            drawFramebuffer = args[1];
          }
        });
        hook('texImage2D', args => {
          if (args.length !== 6 && args.length !== 9)
            throw new Error(`Cannot count texImage2D overload with ${args.length} arguments`);
          const sized = args.length >= 9;
          const source = sized ? args[8] : args[5];
          const [width, height] = sized
            ? [positive(args[3], 'texture width'), positive(args[4], 'texture height')]
            : dimensions(source);
          add('textureAllocBytes', width * height * storageBytesPerPixel(args[2]));
          add('imageUploadBytes', imageBytes(source));
        });
        hook('texStorage2D', args => {
          let width = positive(args[3], 'storage width'), height = positive(args[4], 'storage height');
          const levels = positive(args[1], 'storage levels');
          for (let level = 0; level < levels; level++) {
            add('textureAllocBytes', width * height * storageBytesPerPixel(args[2]));
            width = Math.max(1, Math.floor(width / 2));
            height = Math.max(1, Math.floor(height / 2));
          }
        });
        hook('texSubImage2D', args => {
          if (args.length !== 7 && args.length !== 9)
            throw new Error(`Cannot count texSubImage2D overload with ${args.length} arguments`);
          add('imageUploadBytes', imageBytes(args.length === 9 ? args[8] : args[6]));
        });
        hook('renderbufferStorage', args =>
          add('renderbufferAllocBytes', positive(args[2], 'renderbuffer width')
            * positive(args[3], 'renderbuffer height') * storageBytesPerPixel(args[1])));
        hook('bufferData', args => {
          if (args.length !== 3) throw new Error('Cannot count bufferData slice overload');
          add('bufferUploadBytes', payloadBytes(args[1]));
        });
        hook('bufferSubData', args => {
          if (args.length !== 3) throw new Error('Cannot count bufferSubData slice overload');
          add('bufferUploadBytes', payloadBytes(args[2]));
        });
        // WebGL setter payload differs from wgpu buffer alignment and packing;
        // keep it as a separate component rather than calling it equal bytes.
        for (const [name, lanes] of Object.entries({
          uniform1f: 1, uniform1i: 1, uniform1ui: 1,
          uniform2f: 2, uniform2i: 2, uniform2ui: 2,
          uniform3f: 3, uniform3i: 3, uniform3ui: 3,
          uniform4f: 4, uniform4i: 4, uniform4ui: 4,
        })) hook(name, () => add('uniformPayloadBytes', lanes * 4));
        for (const name of [
          'uniform1fv', 'uniform2fv', 'uniform3fv', 'uniform4fv',
          'uniform1iv', 'uniform2iv', 'uniform3iv', 'uniform4iv',
          'uniform1uiv', 'uniform2uiv', 'uniform3uiv', 'uniform4uiv',
          'uniformMatrix2fv', 'uniformMatrix3fv', 'uniformMatrix4fv',
          'uniformMatrix2x3fv', 'uniformMatrix2x4fv', 'uniformMatrix3x2fv',
          'uniformMatrix3x4fv', 'uniformMatrix4x2fv', 'uniformMatrix4x3fv',
        ]) hook(name, args => add('uniformPayloadBytes', uniformArrayBytes(args, name.startsWith('uniformMatrix'))));

        const draw = () => {
          let frame: any;
          PM.beginEval(sample.time);
          PM.scope.push(PM.proj);
          try {
            frame = GL.renderProject(PM.proj, sample.time, sample.width, sample.height,
              { mblur: false, exporting: true });
            gl.finish();
            if (!frame || frame.w !== sample.width || frame.h !== sample.height || frame.colorDepth !== sample.depth)
              throw new Error('Wrong composition target depth/dimensions');
            if (GL.colorDepth !== sample.depth) throw new Error('Wrong actual WebGL working depth');
            if (gl.getError() !== gl.NO_ERROR) throw new Error('WebGL error during composition');
          } finally {
            PM.scope.pop();
            // Match renderToPixels' target recycle, without readback or presentation.
            if (frame) frame.busy = false;
          }
        };
        counts = {};
        const coldStart = performance.now();
        draw();
        const coldMs = performance.now() - coldStart;
        const cold = counts;
        if (!((cold.drawCalls ?? 0) > 0) || !((cold.textureAllocBytes ?? 0) > 0))
          throw new Error('Cold render did not draw and allocate a fresh working texture');
        counts = {};
        draw(); draw(); // Warmups are deliberately unmeasured.
        counts = {};
        const timings: number[] = [];
        for (let n = 0; n < 12; n++) {
          const start = performance.now();
          draw();
          timings.push(performance.now() - start);
        }
        const warm12 = counts;
        if (!((warm12.drawCalls ?? 0) > 0)) throw new Error('Warm renders had no draw calls');
        timings.sort((a, b) => a - b);
        return { id: sample.id, depth: sample.depth, workingFormat: GL.colorDepth,
          initialCache, cold, warm12, coldMs, p50Ms: timings[6] };
      } finally {
        for (const [name, original] of originals) gl[name] = original;
        PM.agentFrameCapture = priorAgentCapture;
        gl.bindFramebuffer(gl.FRAMEBUFFER, priorFramebuffer);
        gl.viewport(...priorViewport);
        PM.beginEval(PM.time);
      }
    }, { doc, sample });
    rows.push(row);
    console.log(`${sample.id}: ${JSON.stringify(row)}`);
    if (app.pageErrors.length) throw new Error(app.pageErrors.join('; '));
  } finally {
    await app.close();
  }
}
if (rows.length !== 14) throw new Error('Expected all seven projects at 8/16 bits');
writeFileSync(path.join(directory, 'today-counters.json'), JSON.stringify({
  coldFrames: 1, warmupFrames: 2, warmFrames: 12,
  endpoint: 'composition target GPU complete, recycled; no readback/presentation', samples: rows,
}, null, 2));
