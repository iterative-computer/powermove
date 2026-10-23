/** Paired synthetic video seek/decode/upload timing for the shipped Electron.
 * Build the desktop app first. The fixture directory and output stay outside Git.
 *   pm-electron-lock node scripts/video-decode-perf.mjs ABSOLUTE_FIXTURES ABSOLUTE_REPORT
 * Samples end after texImage2D, a full-canvas draw, and gl.finish().
 */
import { readFile, writeFile, mkdtemp, rm, access } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const [fixtureDir, reportPath] = process.argv.slice(2);
if (!fixtureDir || !reportPath || !path.isAbsolute(fixtureDir) || !path.isAbsolute(reportPath)
    || path.resolve(reportPath).startsWith(root + path.sep)) {
  throw new Error('usage: video-decode-perf.mjs ABSOLUTE_FIXTURES ABSOLUTE_REPORT_OUTSIDE_REPO');
}
await access(path.join(desktop, 'out/main/index.js'));
const rows = JSON.parse(await readFile(path.join(fixtureDir, 'sources.json'), 'utf8'));
const captured = JSON.parse(await readFile(path.join(fixtureDir, 'chromium.json'), 'utf8'));
const capturedById = new Map(captured.rows.map(row => [row.id, row]));
const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-video-perf-'));
let app;
const report = { endpoint: 'HTMLVideoElement seek or load to WebGL texImage2D, canvas draw, and gl.finish',
  versions: null, rows: [] };
try {
  const env = { ...process.env, POWERMOVE_USER_DATA: userData, POWERMOVE_BACKGROUND_TEST: '1',
    POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '0', POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await _electron.launch({ args: [path.join(desktop, 'out/main/index.js')], cwd: desktop, env, timeout: 120_000 });
  const page = await app.firstWindow();
  await page.waitForFunction(() => Boolean(window.PM?.mkProject && window.PM?.GL), undefined, { timeout: 60_000 });
  report.versions = await app.evaluate(() => process.versions);
  for (const row of rows) {
    // Passing the synthetic file bytes avoids Electron's file URL permissions.
    // ProRes is imported as a browser-playable rendition by the desktop app.
    const playback = capturedById.get(row.id)?.playback;
    const browserFile = playback?.file || row.file;
    const bytes = [...await readFile(path.join(fixtureDir, browserFile))];
    const result = await page.evaluate(async ({ row, bytes }) => {
      const timeout = (promise, label) => {
        let timer;
        return Promise.race([promise, new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error(`${label} timeout`)), 20_000);
        })]).finally(() => clearTimeout(timer));
      };
      const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)]));
      const canvas = new OffscreenCanvas(row.probe.width, row.probe.height);
      const gl = canvas.getContext('webgl2');
      if (!gl) throw new Error('WebGL2 unavailable');
      // The full-range 709 and wide-gamut compatibility readings cross a
      // Canvas2D boundary in today's paused source path before GPU upload.
      const viaCanvas = row.id.includes('2020') || row.id.includes('full') || row.id.startsWith('prores');
      const sourceCanvas = viaCanvas ? new OffscreenCanvas(row.probe.width, row.probe.height) : null;
      const source2d = sourceCanvas?.getContext('2d', { willReadFrequently: true });
      const shader = (type, source) => {
        const s = gl.createShader(type); gl.shaderSource(s, source); gl.compileShader(s);
        if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
        return s;
      };
      const program = gl.createProgram();
      const vert = shader(gl.VERTEX_SHADER, `#version 300 es
        out vec2 uv; void main() { vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2));
        uv=p; gl_Position=vec4(p*2.0-1.0,0.0,1.0); }`);
      const frag = shader(gl.FRAGMENT_SHADER, `#version 300 es
        precision highp float; uniform sampler2D src; in vec2 uv; out vec4 color;
        void main() { color=texture(src,uv); }`);
      gl.attachShader(program, vert); gl.attachShader(program, frag); gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.useProgram(program); gl.uniform1i(gl.getUniformLocation(program, 'src'), 0);
      gl.viewport(0, 0, row.probe.width, row.probe.height);
      const makeVideo = async () => {
        const video = document.createElement('video'); video.muted = true; video.preload = 'auto';
        const start = performance.now();
        try {
          await timeout(new Promise((resolve, reject) => {
            video.onloadeddata = resolve;
            video.onerror = () => reject(new Error(`video error ${video.error?.code}: ${video.error?.message}`));
            video.src = url;
          }), 'video load');
        } catch (error) {
          video.removeAttribute('src'); video.load();
          throw error;
        }
        return { video, loadMs: performance.now() - start, started: start };
      };
      const sample = async (video, time, phase, round, started) => {
        const seekStart = performance.now();
        await timeout(new Promise((resolve, reject) => {
          video.onseeked = resolve;
          video.onerror = () => reject(new Error(`video error ${video.error?.code}: ${video.error?.message}`));
          video.currentTime = time;
        }), 'video seek');
        const seekEnd = performance.now();
        if (viaCanvas) source2d.drawImage(video, 0, 0);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE,
          viaCanvas ? sourceCanvas : video);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.finish();
        const end = performance.now();
        if (gl.getError() !== gl.NO_ERROR) throw new Error('WebGL upload/draw failed');
        const frame = new VideoFrame(video);
        const presentedUs = frame.timestamp;
        frame.close();
        return { phase, round, time, sourcePath: viaCanvas ? 'Canvas2D-to-WebGL' : 'video-to-WebGL', seekMs: seekEnd - seekStart,
          uploadGpuMs: end - seekEnd, totalMs: end - started, presentedUs,
          width: video.videoWidth, height: video.videoHeight };
      };
      const samples = [];
      const close = video => { video.removeAttribute('src'); video.load(); };
      try {
        for (let i = 0; i < 3; i++) {
          const { video, loadMs, started } = await makeVideo();
          try { samples.push({ ...await sample(video, row.times[i % row.times.length], 'cold', i, started), loadMs }); }
          finally { close(video); }
        }
        const { video } = await makeVideo();
        try {
          for (const [i, time] of row.times.entries()) {
            await sample(video, time, 'prime', i, performance.now());
          }
          for (let i = 0; i < 18; i++) {
            const started = performance.now();
            samples.push(await sample(video, row.times[(i + 1) % row.times.length], 'warm', i, started));
          }
        } finally { close(video); }
        return { samples };
      } catch (error) { return { samples, error: String(error) }; }
      finally {
        gl.deleteTexture(texture); gl.deleteProgram(program); gl.deleteShader(vert); gl.deleteShader(frag);
        gl.getExtension('WEBGL_lose_context')?.loseContext(); URL.revokeObjectURL(url);
      }
    }, { row, bytes });
    if (result.samples?.length !== 21) result.error ||= 'Incomplete cold/warm samples';
    for (const sample of result.samples || []) {
      const reference = capturedById.get(row.id)?.frames?.find(frame => frame.time === sample.time);
      if (!reference || sample.presentedUs !== reference.timestamp
          || sample.width !== reference.w || sample.height !== reference.h) {
        result.error ||= `Wrong presented frame at ${sample.time}s`;
      }
    }
    report.rows.push({ id: row.id, file: row.file, browserFile,
      playbackProxy: Boolean(playback?.playbackProxy), load1: os.loadavg()[0], ...result });
    console.log(JSON.stringify({ id: row.id, samples: result.samples?.length, error: result.error }));
    await writeFile(reportPath, JSON.stringify(report, null, 2));
  }
  if (report.rows.some(row => row.error)) throw new Error('Video decode benchmark has failed rows; see report');
} finally {
  try { await app?.close(); } finally { await rm(userData, { recursive: true, force: true }); }
}
