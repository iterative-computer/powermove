/** Capture today's float32 shader/effect project and replay its native
 * fallback requests in the real runner. Build color_depth_fallback first,
 * prepare the runner with color-depth.ts --prepare outside the Electron lock,
 * then run this script under pm-electron-lock. Outputs stay outside Git.
 */
import { mkdirSync, readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { homedir } from 'node:os';
import { launchLegacyApp } from './app';
import { newProject } from './doc';
import { repoRoot } from './catalog';

const out = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || out === repoRoot || out.startsWith(repoRoot + path.sep)) throw new Error('Supply external output directory');
const binary = path.resolve(process.argv[3] ?? path.join(repoRoot, 'engine/target/debug/examples/color_depth_fallback'));
if (!existsSync(binary)) throw new Error('Build color_depth_fallback before taking Electron lock');
mkdirSync(out, { recursive: true }); mkdirSync(path.join(out, 'pixels'), { recursive: true });
const bundle = readFileSync(path.join(out, 'fallback.bundle.js'), 'utf8');
const builder = newProject('depth/fallback', { name: 'Float32 fallback parity', w: 64, h: 32, dur: 1, backgroundFill: { type: 'none', angle: 0, stops: [{ id: 'none', color: '#000000', position: 0 }] } });
builder.root.colorDepth = 32;
const shader = 'void main(){fragColor=vec4(0.5001234+v_uv.x*0.1234567,0.2001234+v_uv.y*0.2345678,0.7001234,1.0);}';
const layer = builder.add(builder.layer('shader', { d: { code: shader, w: 64, h: 32, uniforms: {} } }));
layer.fx.push({ id: 'depth-effect', type: 'depth.float32', on: builder.ch(true), p: {} });
const app = await launchLegacyApp();
try {
  await app.evaluateScript(`(()=>{${bundle};globalThis.DepthFallback=DepthFallback;return true;})()`);
  const capture = await app.evaluate(async (doc: any) => {
    const PM = (window as any).PM;
    const fragment = PM.GLSL_PRE + '\nvoid main(){o=texture(u_tex,v_st);o.rgb=o.rgb*0.918273+vec3(0.0003123,0.0004321,0.0005678);}';
    const def = { id: 'depth.float32', label: 'Depth transport', group: 'Test', params: [], rawShader: true, passes: 3, frag: fragment };
    const registration = PM.Kernel.effects.register('depth-gate', def);
    try {
      PM.replaceProject(doc); PM.pause(); await PM.projectAssetsReady; await PM.prepareFrame(0);
      const gl = PM.GL.gl, read = gl.readPixels.bind(gl);
      for (let i = 0; i < 32 && gl.getError(); i++);
      let raw: Float32Array | undefined;
      gl.readPixels = (...args: any[]) => { const result = read(...args); if (args[5] === gl.FLOAT) raw = new Float32Array(args[6]); return result; };
      try { PM.GL.renderToPixels(0, 64, 32, { mblur: false, exporting: true }); } finally { gl.readPixels = read; }
      const glError = gl.getError();
      if (!raw || PM.GL.colorDepth !== 32 || glError) throw new Error(`Missing float32 legacy reference: read=${!!raw}, depth=${PM.GL.colorDepth}, error=${glError}, programs=${JSON.stringify(Array.from(PM.GL.errors))}`);
      const pixels = new Float32Array(raw.length);
      for (let y = 0; y < 32; y++) pixels.set(raw.subarray((31-y)*64*4,(32-y)*64*4),y*64*4);
      const glsl = { source: fragment, bodyLineOffset: 0, bodyUtf16Len: fragment.length, bodyUtf8Len: new TextEncoder().encode(fragment).length };
      return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels), definition: { id: def.id, kind: 'effect', version: 0, origin: 'store', renderer: null, glsl, params: [], flags: { passes: 3, keepOrig: false, backdrop: false, rawShader: true, viewportSafe: false, viewportPadding: [], clamp: false } } };
    } finally { registration.dispose(); }
  }, builder.root);
  writeFileSync(path.join(out, 'project.json'), JSON.stringify(capture.doc));
  writeFileSync(path.join(out, 'definition.json'), JSON.stringify(capture.definition));
  writeFileSync(path.join(out, 'reference.f32'), new Uint8Array(new Float32Array(capture.pixels).buffer));
  for (let round = 0; round < 8; round++) {
    const ffmpegRoot = process.env.PM_FFMPEG_DIR ?? path.join(homedir(), 'Library/Caches/powermove-engine/ffmpeg/out');
    const native = Bun.spawn([binary, out], {
      stdout: 'pipe', stderr: 'pipe',
      env: { ...process.env, DYLD_LIBRARY_PATH: [path.join(ffmpegRoot, 'lib'), process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(':') },
    });
    const [code, stdout, stderr] = await Promise.all([native.exited, new Response(native.stdout).text(), new Response(native.stderr).text()]);
    writeFileSync(path.join(out, `round-${round}.log`), stdout + stderr);
    if (code === 0) { console.log(stdout.trim()); break; }
    if (code !== 2) throw new Error(`Native gate failed (${code}): ${stderr}`);
    const files = readdirSync(path.join(out, 'requests')).filter(name => name.endsWith('.json'));
    let answered = 0;
    for (const name of files) {
      const request = JSON.parse(readFileSync(path.join(out, 'requests', name), 'utf8'));
      const target = path.join(out, 'pixels', request.key + '.f32');
      if (existsSync(target)) continue;
      const textures = Array.from({ length: request.textures ?? 0 }, (_, index) => readFileSync(path.join(out, 'requests', `${request.key}-${index}.f32`)).toString('base64'));
      const result = await app.evaluate(async ({ request, textures }) => {
        const runner = new (window as any).DepthFallback.ShaderFallbackRunner();
        try {
          const inputs = textures.map((text: string) => { const b = atob(text); return new Float32Array(Uint8Array.from(b, c => c.charCodeAt(0)).buffer); });
          const result = await runner.render(request, inputs);
          const pixels = result.rgba32f;
          if (!(pixels instanceof Float32Array)) throw new Error('Runner did not return float32');
          return Array.from(new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength));
        } finally { runner.dispose(); }
      }, { request, textures });
      writeFileSync(target, new Uint8Array(result)); answered++;
    }
    if (!answered) throw new Error('Pending gate made no progress');
    if (round === 7) throw new Error('Fallback gate did not settle');
  }
} finally { await app.close(); }
