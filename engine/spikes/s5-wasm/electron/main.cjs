// S5 Electron harness: measures the wasm evaluator in a sandboxed,
// context-isolated Electron renderer under the editor CSP plus
// 'wasm-unsafe-eval'. Windows stay hidden and never take focus.
//
//   env -u ELECTRON_RUN_AS_NODE \
//     ../../../apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron \
//     electron/main.cjs [isolated|plain|nowasm|all]
//
// scripts/run-all.sh runs one mode per Electron process: each mode gets a
// fresh renderer process and fresh process-memory numbers.
//
// Modes: `plain` is the editor's headers with the plan's CSP change, the
// configuration S5 is judged in; `isolated` adds COOP/COEP only for 5 us
// timers; `nowasm` is today's CSP, the negative control. Results go to
// results/electron-<mode>.local.json.
//
// CommonJS so `require('electron')` resolves to Electron's built-in module
// rather than the npm package of the same name.
const { app, BrowserWindow, protocol } = require('electron');
const { readFile, writeFile, mkdir } = require('node:fs/promises');
const { execFileSync } = require('node:child_process');
const path = require('node:path');
const os = require('node:os');

const root = path.resolve(__dirname, '..');

// apps/desktop/src/main/security-policy.ts CONTENT_SECURITY_POLICY, with the
// plan's single change (section 19): script-src gains 'wasm-unsafe-eval'.
const EDITOR_CSP = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data: http: https:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob: http: https: ws: wss:; worker-src 'self' blob:; frame-src 'self' about: blob: app:";
const WASM_CSP = EDITOR_CSP.replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'");

const MODES = {
  isolated: { csp: WASM_CSP, isolate: true },
  plain: { csp: WASM_CSP, isolate: false },
  nowasm: { csp: EDITOR_CSP, isolate: false },
};

const TYPES = { '.html': 'text/html', '.mjs': 'text/javascript', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' };

protocol.registerSchemesAsPrivileged([{ scheme: 's5', privileges: { standard: true, secure: true, supportFetchAPI: true } }]);
app.commandLine.appendSwitch('js-flags', '--expose-gc');
// The harness window is hidden, so Chromium would background its renderer
// process (lower CPU priority, efficiency cores on Apple Silicon). An editor
// window is visible; these switches keep the hidden renderer at foreground
// priority. S5_ALLOW_BACKGROUNDING=1 measures the backgrounded case instead.
const backgrounding = process.env.S5_ALLOW_BACKGROUNDING === '1';
if (!backgrounding) {
  app.commandLine.appendSwitch('disable-renderer-backgrounding');
  app.commandLine.appendSwitch('disable-background-timer-throttling');
  app.commandLine.appendSwitch('disable-backgrounding-occluded-windows');
}
// Never activate, never show a Dock icon, never touch the user's profile.
app.setPath('userData', path.join(os.tmpdir(), 'powermove-s5-electron'));
app.dock?.hide();
app.setActivationPolicy?.('prohibited');

// Physical footprint (what Activity Monitor calls Memory). `footprint` needs
// task access to the process; where it is denied or hangs, this is null and
// the working set from getAppMetrics and ps RSS remain.
let footprintWorks = true;
function footprint(pid) {
  if (!footprintWorks) return null;
  try {
    const out = execFileSync('/usr/bin/footprint', ['-p', String(pid), '-f', 'bytes'], { encoding: 'utf8', timeout: 5000 });
    const m = /Footprint:\s*([\d,]+)\s*B/i.exec(out) || /phys_footprint:\s*([\d,]+)/i.exec(out) || /([\d,]+) B\s+\(/.exec(out);
    return m ? Number(m[1].replace(/,/g, '')) : null;
  } catch {
    footprintWorks = false;
    return null;
  }
}

function rssKB(pid) {
  try {
    return Number(execFileSync('/bin/ps', ['-o', 'rss=', '-p', String(pid)], { encoding: 'utf8', timeout: 5000 }).trim()) || null;
  } catch {
    return null;
  }
}

function processMemory(win) {
  const pid = win.webContents.getOSProcessId();
  const metric = app.getAppMetrics().find((m) => m.pid === pid);
  return { pid, workingSetKB: metric?.memory?.workingSetSize ?? null, peakWorkingSetKB: metric?.memory?.peakWorkingSetSize ?? null, rssKB: rssKB(pid), footprintBytes: footprint(pid) };
}

async function runMode(name) {
  const mode = MODES[name];
  const win = new BrowserWindow({
    show: false, focusable: false, skipTaskbar: true, width: 320, height: 240, x: -10000, y: -10000,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false, backgroundThrottling: false },
  });
  const consoleLines = [];
  win.webContents.on('console-message', (event) => {
    consoleLines.push(`${event.level}: ${event.message}`);
    process.stderr.write(`[s5 ${name} console] ${event.level}: ${event.message}\n`);
  });
  const log = (what) => process.stderr.write(`[s5 ${name}] ${what}\n`);
  const loadBefore = os.loadavg();
  log('loading');
  await win.loadURL(`s5://${name}/electron/index.html`);
  log('loaded');
  const prefs = win.webContents.getLastWebPreferences?.() ?? {};
  const result = { mode: name, rendererBackgroundingAllowed: backgrounding, csp: mode.csp, crossOriginIsolationHeaders: mode.isolate, electron: process.versions.electron, chrome: process.versions.chrome, webPreferences: { sandbox: prefs.sandbox, contextIsolation: prefs.contextIsolation, nodeIntegration: prefs.nodeIntegration } };
  result.memory = { pageLoaded: processMemory(win) };
  log('init');
  result.env = await win.webContents.executeJavaScript('window.s5.init()');
  log(`wasm ${result.env.wasm}`);
  result.memory.wasmInstantiated = processMemory(win);
  if (result.env.wasm === 'ok') {
    const running = win.webContents.executeJavaScript('window.s5.run()');
    let done = false;
    let results = null;
    running.then((r) => { results = r; done = true; }, (e) => { results = { error: String(e) }; done = true; });
    while (!done) {
      const stage = await win.webContents.executeJavaScript('window.s5.waiting');
      if (stage) {
        await win.webContents.executeJavaScript('globalThis.gc && globalThis.gc()');
        result.memory[stage] = processMemory(win);
        await win.webContents.executeJavaScript('window.s5.resume()');
      }
      await new Promise((r) => setTimeout(r, 20));
    }
    result.results = results;
  }
  result.loadavg = { before: loadBefore, after: os.loadavg(), cpus: os.cpus().length };
  result.console = consoleLines.slice(0, 50);
  win.destroy();
  return result;
}

const writeLine = (line) => new Promise((resolve) => process.stdout.write(`${line}\n`, resolve));

app.whenReady().then(async () => {
  protocol.handle('s5', async (request) => {
    const url = new URL(request.url);
    const mode = MODES[url.host];
    const rel = decodeURIComponent(url.pathname).replace(/^\/+/, '');
    const allowed = /^(electron|js|pkg)\/[\w.-]+$/.test(rel);
    if (!mode || !allowed) return new Response('not found', { status: 404 });
    try {
      const body = await readFile(path.join(root, rel));
      const headers = { 'content-type': TYPES[path.extname(rel)] ?? 'application/octet-stream', 'content-security-policy': mode.csp };
      if (mode.isolate) Object.assign(headers, { 'cross-origin-opener-policy': 'same-origin', 'cross-origin-embedder-policy': 'require-corp', 'cross-origin-resource-policy': 'same-origin' });
      return new Response(body, { headers });
    } catch {
      return new Response('not found', { status: 404 });
    }
  });
  const which = process.argv.at(-1);
  const modes = MODES[which] ? [which] : ['nowasm', 'plain', 'isolated'];
  await mkdir(path.join(root, 'results'), { recursive: true });
  const all = {};
  let failed = false;
  for (const name of modes) {
    try {
      all[name] = await runMode(name);
    } catch (error) {
      // A mode that cannot run is reported, never left out silently.
      all[name] = { mode: name, error: String(error?.stack ?? error) };
      failed = true;
    }
    if (all[name].results?.error) failed = true;
    await writeFile(path.join(root, `results/electron-${name}.local.json`), JSON.stringify(all[name], null, 2));
    // Written with a callback: app.exit() does not wait for stdout to drain.
    await writeLine(`mode ${name}: ${all[name].error ? `error (${all[name].error.split('\n')[0]})` : `wasm ${all[name].env?.wasm}${all[name].env?.wasmError ? ` (${all[name].env.wasmError})` : ''}`}`);
  }
  app.exit(failed ? 1 : 0);
});
