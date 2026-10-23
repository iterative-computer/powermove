// Standalone check of the presentation painters across GPU process resets
// (docs/engine/scene3d.md, round 10). A worker draws F16 VideoFrames
// continuously; main kills the GPU process every 2.5 s and counts renderer
// crashes. Electron 44: MODE=webgpu crashes in Blink's
// WebGpuSharedImageWrapperCache after the raster context is lost;
// MODE=webgl (the viewer's painter) does not. Run again on every Electron
// upgrade, under pm-electron-lock:
//   MODE=webgpu KILLS=20 <electron> scripts/gpu-reset-painter-repro/main.js
// Prints one JSON line per reset and a final {"event":"done",...,"crashes":n}.
const { app, BrowserWindow } = require('electron');
const path = require('node:path');
const MODE = process.env.MODE || 'webgpu';
const KILLS = Number(process.env.KILLS || 20);
app.commandLine.appendSwitch('disable-gpu-process-crash-limit');
app.commandLine.appendSwitch('enable-unsafe-webgpu');
app.disableDomainBlockingFor3DAPIs();
const events = [];
const log = (o) => { const line = JSON.stringify({ t: Date.now(), ...o }); events.push(line); console.log(line); };
app.on('render-process-gone', (_e, _wc, d) => log({ event: 'renderer-gone', ...d }));
app.on('child-process-gone', (_e, d) => { if (d.type !== 'GPU') log({ event: 'child-gone', ...d }); });
const within = (p, ms) => Promise.race([p, new Promise((r) => setTimeout(() => r({ timeout: true }), ms))]);
app.whenReady().then(async () => {
  const win = new BrowserWindow({ show: false, width: 800, height: 600, webPreferences: { backgroundThrottling: false } });
  let crashes = 0;
  const load = () => within(win.loadFile(path.join(__dirname, 'page.html'), { query: { mode: MODE } }).catch(() => {}), 10000);
  win.webContents.on('render-process-gone', () => { crashes++; setTimeout(() => { void load(); }, 200); });
  await load();
  await new Promise((r) => setTimeout(r, 3000));
  for (let i = 0; i < KILLS; i++) {
    const seen = crashes;
    const before = await within(win.webContents.executeJavaScript('window.stats()').catch((e) => ({ error: String(e) })), 2000);
    const gpu = app.getAppMetrics().find((m) => m.type === 'GPU');
    if (gpu) process.kill(gpu.pid, 'SIGKILL');
    await new Promise((r) => setTimeout(r, 2500));
    if (crashes > seen) await new Promise((r) => setTimeout(r, 3000));
    const after = await within(win.webContents.executeJavaScript('window.stats()').catch((e) => ({ error: String(e) })), 2000);
    log({ event: 'kill', i: i + 1, mode: MODE, before, after, crashedThisIteration: crashes - seen, crashes });
  }
  log({ event: 'done', mode: MODE, kills: KILLS, crashes });
  app.exit(0);
});
