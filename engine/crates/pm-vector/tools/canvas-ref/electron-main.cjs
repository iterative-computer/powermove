/* Hidden Electron host for the Canvas2D references (run.ts). Never shows a
   window, never takes focus, never appears in the Dock.

   electron electron-main.cjs --url=http://127.0.0.1:<port>/ */
const { app, BrowserWindow } = require('electron');

const args = Object.fromEntries(process.argv.slice(2).filter(a => a.startsWith('--')).map(a => {
  const [key, ...rest] = a.slice(2).split('=');
  return [key, rest.join('=') || 'true'];
}));

if (process.platform === 'darwin') {
  app.setActivationPolicy?.('prohibited');
  app.dock?.hide();
}

app.whenReady().then(async () => {
  if (process.platform === 'darwin') app.setActivationPolicy?.('prohibited');
  const base = args.url;
  const win = new BrowserWindow({
    show: false,
    focusable: false,
    skipTaskbar: true,
    width: 800,
    height: 600,
    x: -20000,
    y: -20000,
    paintWhenInitiallyHidden: true,
    webPreferences: { backgroundThrottling: false, sandbox: true, contextIsolation: true },
  });
  win.webContents.on('console-message', (_e, level, message) => { if (level >= 2) console.error(`[page] ${message}`); });
  // GPU facts once the page has rendered (before that, the GPU process may not
  // have reported and every feature reads as disabled).
  win.webContents.on('did-finish-load', () => {
    const report = async () => {
      try {
        const gpu = { featureStatus: app.getGPUFeatureStatus(), info: await app.getGPUInfo('basic'), chrome: process.versions.chrome, electron: process.versions.electron };
        await fetch(new URL('gpu', base), { method: 'POST', body: JSON.stringify(gpu) });
      } catch (error) {
        console.error(`gpu info failed: ${error && error.stack || error}`);
      }
    };
    setTimeout(report, 500);
  });
  await win.loadURL(base);
});
