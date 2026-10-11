/* Hidden Electron capture host for the S4 corpus. Never shows a window, never
   takes focus, never appears in the Dock.

   electron capture/electron-main.cjs --mode=measure --corpus=out/corpus.json --out=out/canvas2d.json [--probe-fonts=out/platform-fonts.json]
   electron capture/electron-main.cjs --mode=render --scenes=out/scenes.json --out=out/render-canvas2d */

const { app, BrowserWindow, ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');

const args = Object.fromEntries(process.argv.slice(2).filter(a => a.startsWith('--')).map(a => {
  const [key, ...rest] = a.slice(2).split('=');
  return [key, rest.join('=') || 'true'];
}));
const mode = args.mode || 'measure';

if (process.platform === 'darwin') {
  app.setActivationPolicy?.('prohibited');
  app.dock?.hide();
}
app.disableHardwareAcceleration && args.cpu === 'true' && app.disableHardwareAcceleration();

function fail(message) {
  console.error(message);
  app.exit(1);
}

app.whenReady().then(async () => {
  if (process.platform === 'darwin') app.setActivationPolicy?.('prohibited');
  const win = new BrowserWindow({
    show: false,
    focusable: false,
    skipTaskbar: true,
    width: 1024,
    height: 768,
    x: -20000,
    y: -20000,
    paintWhenInitiallyHidden: true,
    webPreferences: { nodeIntegration: true, contextIsolation: false, sandbox: false, backgroundThrottling: false },
  });
  const timeout = setTimeout(() => fail('capture timed out'), Number(args.timeout || 900) * 1000);
  ipcMain.handle('config', () => ({
    corpusPath: path.resolve(args.corpus || ''),
    scenesPath: path.resolve(args.scenes || ''),
    outPath: path.resolve(args.out || ''),
    probeFonts: !!args['probe-fonts'],
  }));
  ipcMain.handle('failed', (_event, message) => fail(message));
  ipcMain.handle('measured', async (_event, count) => {
    console.log(`measured ${count} cases`);
    if (args['probe-fonts']) {
      try { await probeFonts(win, path.resolve(args['probe-fonts'])); }
      catch (error) { return fail(`font probe failed: ${error && error.stack || error}`); }
    }
    clearTimeout(timeout);
    app.exit(0);
  });
  ipcMain.handle('rendered', (_event, count) => {
    console.log(`rendered ${count} scenes`);
    clearTimeout(timeout);
    app.exit(0);
  });
  const page = mode === 'render' ? 'render.html' : 'measure.html';
  await win.loadFile(path.join(__dirname, '..', 'out', 'capture', page));
});

/* Ask Blink which platform fonts rendered each case's text (DOM spans styled
   with the same font string as the canvas). */
async function probeFonts(win, outPath) {
  const dbg = win.webContents.debugger;
  dbg.attach('1.3');
  await dbg.sendCommand('DOM.enable');
  await dbg.sendCommand('CSS.enable');
  const { root } = await dbg.sendCommand('DOM.getDocument', { depth: 0 });
  const { nodeIds } = await dbg.sendCommand('DOM.querySelectorAll', { nodeId: root.nodeId, selector: 'span[data-case]' });
  const out = {};
  for (const nodeId of nodeIds) {
    const { attributes } = await dbg.sendCommand('DOM.getAttributes', { nodeId });
    const id = attributes[attributes.indexOf('data-case') + 1];
    const { fonts } = await dbg.sendCommand('CSS.getPlatformFontsForNode', { nodeId });
    out[id] = fonts;
  }
  fs.writeFileSync(outPath, JSON.stringify(out));
  console.log(`probed fonts for ${nodeIds.length} cases`);
}
