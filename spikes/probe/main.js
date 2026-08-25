'use strict';

const { app, BrowserWindow, ipcMain, protocol, session } = require('electron');
const { execFile } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { Readable } = require('node:stream');

const PROBE_DIR = __dirname;
const FIXTURE_DIR = path.resolve(PROBE_DIR, '..', 'fixtures');
const MODE = (() => {
  const flag = process.argv.find((arg) => arg.startsWith('--mode='));
  const value = flag ? flag.slice('--mode='.length) : 'file';
  return ['file', 'app', 'finder', 'interactive'].includes(value) ? value : 'file';
})();
const APP_MODE = MODE === 'app';
const V3_SCRIPT_HASH = 'sha256-5E7go0oDRQFdIHnVl4OMlf8X7Y3HKlN1+FitlqAk9A8=';
const CSP = `default-src 'none'; script-src 'self' 'unsafe-eval' '${V3_SCRIPT_HASH}'; style-src 'unsafe-inline'; img-src blob: data:; media-src 'self' file: blob:; connect-src 'self' blob:; worker-src blob:; frame-src 'self' about: blob:`;
const STRICT_CSP = "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src blob: data:; media-src 'self' file: blob:; connect-src 'self' blob:; worker-src blob:; frame-src about: blob:";
const RESULT_PATH = path.join(PROBE_DIR, `results-${MODE}.json`);
const PARTIAL_PATH = path.join(PROBE_DIR, `.results-${MODE}.partial.json`);
const PROBE_KEYS = [...'abcdefghijk', 'd2', 'i2'];
const HARD_TIMEOUT_MS = MODE === 'interactive' ? 10 * 60_000 : 90_000;
const CODEX_PATHS = [
  '/opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex',
  '/usr/local/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex',
];

if (APP_MODE) {
  protocol.registerSchemesAsPrivileged([{
    scheme: 'app',
    privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true },
  }]);
}

let mainWindow = null;
let strictWindow = null;
let strictPending = null;
let finished = false;
let hardTimer = null;
const permissionsAsked = [];
const partial = {};
let consoleMessages = [];
const startedAt = Date.now();

function cleanError(error) {
  return String(error && (error.stack || error.message) || error).slice(0, 4000);
}

function memorySnapshot() {
  const value = process.memoryUsage();
  return Object.fromEntries(Object.entries(value).map(([key, bytes]) => [key, Number(bytes)]));
}

function reportObject(finalReason) {
  const report = {
    meta: {
      mode: MODE,
      generatedAt: new Date().toISOString(),
      elapsedMs: Date.now() - startedAt,
      electron: process.versions.electron,
      chrome: process.versions.chrome,
      node: process.versions.node,
      platform: process.platform,
      arch: process.arch,
      finalReason,
      permissionsAsked,
    },
  };
  for (const key of PROBE_KEYS) {
    report[key] = partial[key] || {
      ok: false,
      detail: null,
      error: `probe did not finish before ${finalReason}`,
    };
  }
  return report;
}

function writePartial() {
  try {
    fs.writeFileSync(PARTIAL_PATH, `${JSON.stringify({ mode: MODE, partial }, null, 2)}\n`);
  } catch (error) {
    console.error('Could not checkpoint probe report:', cleanError(error));
  }
}

function finish(finalReason = 'completed') {
  if (finished) return;
  finished = true;
  clearTimeout(hardTimer);
  const report = reportObject(finalReason);
  try {
    fs.writeFileSync(RESULT_PATH, `${JSON.stringify(report, null, 2)}\n`);
    try { fs.unlinkSync(PARTIAL_PATH); } catch (_) { /* absent is fine */ }
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    console.error(`Could not write ${RESULT_PATH}:`, cleanError(error));
  }
  setTimeout(() => app.quit(), 50);
}

function execFileResult(file, args, options = {}) {
  const start = performance.now();
  return new Promise((resolve) => {
    execFile(file, args, { timeout: 30_000, maxBuffer: 32 * 1024 * 1024, ...options }, (error, stdout, stderr) => {
      resolve({
        ok: !error,
        ms: Math.round((performance.now() - start) * 10) / 10,
        stdout: String(stdout || ''),
        stderr: String(stderr || ''),
        error: error ? cleanError(error) : undefined,
      });
    });
  });
}

function resolveOnPath(command, envPath = process.env.PATH || '') {
  for (const directory of envPath.split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(directory, command);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch (_) { /* keep looking */ }
  }
  return null;
}

async function probeCodex() {
  const start = performance.now();
  const current = resolveOnPath('codex');
  const login = await execFileResult('/bin/zsh', ['-ilc', 'command -v codex; echo PATH=$PATH']);
  return {
    ok: true,
    ms: Math.round((performance.now() - start) * 10) / 10,
    detail: {
      path: process.env.PATH || null,
      codexBinary: process.env.CODEX_BINARY || null,
      currentPathResolution: current,
      hardcoded: CODEX_PATHS.map((candidate) => {
        let exists = false;
        let executable = false;
        try { exists = fs.existsSync(candidate); fs.accessSync(candidate, fs.constants.X_OK); executable = true; } catch (_) { /* report false */ }
        return { path: candidate, exists, executable };
      }),
      loginShell: login,
    },
  };
}

function collectNames(value, names) {
  if (Array.isArray(value)) {
    for (const item of value) collectNames(item, names);
  } else if (value && typeof value === 'object') {
    if (typeof value.family === 'string') names.add(value.family);
    for (const child of Object.values(value)) collectNames(child, names);
  }
}

async function probeSystemFonts() {
  const profiler = await execFileResult('/usr/sbin/system_profiler', ['SPFontsDataType', '-json'], { timeout: 60_000 });
  let familyCount = null;
  let parseError;
  if (profiler.ok) {
    try {
      const names = new Set();
      collectNames(JSON.parse(profiler.stdout), names);
      familyCount = names.size;
    } catch (error) {
      parseError = cleanError(error);
    }
  }
  const directories = ['/System/Library/Fonts', '/Library/Fonts', path.join(app.getPath('home'), 'Library', 'Fonts')];
  const fallbackStart = performance.now();
  const fallback = directories.map((directory) => {
    try {
      return { directory, files: fs.readdirSync(directory).sort() };
    } catch (error) {
      return { directory, files: [], error: cleanError(error) };
    }
  });
  return {
    ok: profiler.ok && !parseError,
    ms: profiler.ms,
    detail: {
      systemProfiler: { ok: profiler.ok, ms: profiler.ms, familyCount, error: profiler.error || parseError },
      fallback: {
        ms: Math.round((performance.now() - fallbackStart) * 10) / 10,
        fileCount: new Set(fallback.flatMap((entry) => entry.files)).size,
        directories: fallback.map((entry) => ({ directory: entry.directory, count: entry.files.length, error: entry.error })),
      },
    },
    error: profiler.ok && !parseError ? undefined : (profiler.error || parseError),
  };
}

const mainStaticPromise = Promise.all([probeSystemFonts(), probeCodex()]).then(([fonts, codex]) => ({ fonts, codex }));

async function fixtureDescription() {
  const manifestPath = path.join(FIXTURE_DIR, 'fixtures.json');
  if (!fs.existsSync(manifestPath)) {
    return { manifest: { skipped: 'fixture missing', path: manifestPath }, fixtures: [] };
  }
  try {
    const parsed = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
    if (!Array.isArray(parsed)) throw new Error('fixtures.json must contain an array');
    const fixtures = parsed.map((entry) => {
      const file = String(entry.file || '');
      const fixturePath = path.resolve(FIXTURE_DIR, file);
      const inside = fixturePath === FIXTURE_DIR || fixturePath.startsWith(`${FIXTURE_DIR}${path.sep}`);
      const exists = inside && Boolean(file) && fs.existsSync(fixturePath);
      return {
        ...entry,
        exists,
        skipped: exists ? undefined : 'fixture missing',
        url: exists ? (APP_MODE
          ? `app://probe/fixtures/${file.split('/').map(encodeURIComponent).join('/')}`
          : pathToFileURL(fixturePath).href) : null,
        fixturePath,
      };
    });
    const ffprobe = resolveOnPath('ffprobe') || '/opt/homebrew/bin/ffprobe';
    await Promise.all(fixtures.map(async (fixture) => {
      if (!fixture.exists || !['vp09', 'av01'].includes(fixture.video)) return;
      const args = [
        '-v', 'error', '-select_streams', 'v:0',
        '-show_entries', 'stream=color_range,color_space,color_primaries,color_transfer',
        '-of', 'json', fixture.fixturePath,
      ];
      const result = await execFileResult(ffprobe, args);
      let parsedOutput = null;
      try { parsedOutput = JSON.parse(result.stdout); } catch (_) { /* preserve raw output below */ }
      fixture.ffprobeColor = {
        executable: ffprobe,
        args: args.slice(0, -1).concat(fixture.file),
        ok: result.ok,
        stdout: result.stdout,
        stderr: result.stderr,
        error: result.error,
        parsed: parsedOutput,
        stream: {
          color_range: parsedOutput && parsedOutput.streams && parsedOutput.streams[0] && parsedOutput.streams[0].color_range || null,
          color_space: parsedOutput && parsedOutput.streams && parsedOutput.streams[0] && parsedOutput.streams[0].color_space || null,
          color_primaries: parsedOutput && parsedOutput.streams && parsedOutput.streams[0] && parsedOutput.streams[0].color_primaries || null,
          color_transfer: parsedOutput && parsedOutput.streams && parsedOutput.streams[0] && parsedOutput.streams[0].color_transfer || null,
        },
      };
      console.log(`[ffprobe color] ${fixture.file}\n${result.stdout || result.stderr || result.error || '(no output)'}`);
    }));
    return {
      manifest: { ok: true, path: manifestPath, count: parsed.length },
      fixtures: fixtures.map(({ fixturePath: _fixturePath, ...fixture }) => fixture),
    };
  } catch (error) {
    return { manifest: { ok: false, path: manifestPath, error: cleanError(error) }, fixtures: [] };
  }
}

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.mp4': 'video/mp4',
  '.mov': 'video/quicktime',
  '.webm': 'video/webm',
  '.m4a': 'audio/mp4',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.ogg': 'audio/ogg',
};

function safeServedPath(url) {
  const parsed = new URL(url);
  if (parsed.hostname !== 'probe') return null;
  let pathname;
  try { pathname = decodeURIComponent(parsed.pathname); } catch (_) { return null; }
  if (pathname === '/') pathname = '/index.html';
  const isFixture = pathname.startsWith('/fixtures/');
  const root = isFixture ? FIXTURE_DIR : PROBE_DIR;
  const relative = isFixture ? pathname.slice('/fixtures/'.length) : pathname.slice(1);
  const target = path.resolve(root, relative);
  if (!(target === root || target.startsWith(`${root}${path.sep}`))) return null;
  return target;
}

async function serveAppRequest(request) {
  const target = safeServedPath(request.url);
  if (!target) return new Response('Forbidden', { status: 403 });
  let stat;
  try { stat = fs.statSync(target); } catch (_) { return new Response('Not found', { status: 404 }); }
  if (!stat.isFile()) return new Response('Not found', { status: 404 });

  const headers = new Headers({
    'Content-Type': MIME[path.extname(target).toLowerCase()] || 'application/octet-stream',
    'Accept-Ranges': 'bytes',
  });
  if (target.endsWith('index.html')) headers.set('Content-Security-Policy', CSP);
  if (target.endsWith('index-strict.html')) headers.set('Content-Security-Policy', STRICT_CSP);
  if (target.endsWith('index.html')) {
    const source = fs.readFileSync(target, 'utf8').replace(/\s*<meta[^>]+data-file-csp[^>]*>/, '');
    headers.set('Content-Length', String(Buffer.byteLength(source)));
    return request.method === 'HEAD' ? new Response(null, { status: 200, headers }) : new Response(source, { status: 200, headers });
  }
  if (target.endsWith('sandbox.html')) {
    const variant = new URL(request.url).searchParams.get('variant');
    headers.set('Content-Security-Policy', variant === 'V1b'
      ? "default-src 'none'; script-src 'self' 'unsafe-eval'; worker-src blob:; connect-src 'none'"
      : "default-src 'none'; script-src app://probe/sandbox.js 'unsafe-eval'; worker-src blob:; connect-src 'none'");
    const source = fs.readFileSync(target, 'utf8').replace(/\s*<meta[^>]+data-file-csp[^>]*>/, '');
    headers.set('Content-Length', String(Buffer.byteLength(source)));
    return request.method === 'HEAD' ? new Response(null, { status: 200, headers }) : new Response(source, { status: 200, headers });
  }

  let start = 0;
  let end = stat.size - 1;
  let status = 200;
  const range = request.headers.get('range');
  if (range) {
    const match = /^bytes=(\d*)-(\d*)$/i.exec(range.trim());
    if (!match) return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${stat.size}` } });
    if (match[1]) start = Number(match[1]);
    if (match[2]) end = Number(match[2]);
    if (!match[1] && match[2]) { start = Math.max(0, stat.size - Number(match[2])); end = stat.size - 1; }
    if (start > end || start >= stat.size) return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${stat.size}` } });
    end = Math.min(end, stat.size - 1);
    status = 206;
    headers.set('Content-Range', `bytes ${start}-${end}/${stat.size}`);
  }
  headers.set('Content-Length', String(Math.max(0, end - start + 1)));
  if (request.method === 'HEAD') return new Response(null, { status, headers });
  const body = Readable.toWeb(fs.createReadStream(target, { start, end }));
  return new Response(body, { status, headers });
}

function webPreferences() {
  return {
    preload: path.join(PROBE_DIR, 'preload.js'),
    contextIsolation: true,
    sandbox: true,
    nodeIntegration: false,
    autoplayPolicy: 'no-user-gesture-required',
  };
}

async function loadProbePage(window, strict = false) {
  const file = strict ? 'index-strict.html' : 'index.html';
  if (APP_MODE) return window.loadURL(`app://probe/${file}`);
  return window.loadFile(path.join(PROBE_DIR, file));
}

function createStrictWindow() {
  if (strictPending) return strictPending.promise;
  let resolvePending;
  const promise = new Promise((resolve) => { resolvePending = resolve; });
  strictPending = { promise, resolve: resolvePending };
  strictWindow = new BrowserWindow({ width: 640, height: 480, show: false, webPreferences: webPreferences() });
  const timeout = setTimeout(() => {
    if (!strictPending) return;
    strictPending.resolve({ ok: false, detail: null, error: 'strict CSP window timed out' });
    strictPending = null;
    if (strictWindow && !strictWindow.isDestroyed()) strictWindow.destroy();
  }, 15_000);
  strictWindow.once('closed', () => { clearTimeout(timeout); strictWindow = null; });
  loadProbePage(strictWindow, true).catch((error) => {
    clearTimeout(timeout);
    if (strictPending) {
      strictPending.resolve({ ok: false, detail: null, error: cleanError(error) });
      strictPending = null;
    }
  });
  return promise;
}

function installPermissions() {
  const ses = session.defaultSession;
  ses.setPermissionRequestHandler((webContents, permission, callback, details) => {
    const granted = permission === 'local-fonts';
    permissionsAsked.push({
      kind: 'request', permission, granted,
      url: webContents.getURL(),
      requestingUrl: details && details.requestingUrl,
      mediaTypes: details && details.mediaTypes,
      at: new Date().toISOString(),
    });
    callback(granted);
  });
  ses.setPermissionCheckHandler((webContents, permission, requestingOrigin, details) => {
    const granted = permission === 'local-fonts';
    permissionsAsked.push({
      kind: 'check', permission, granted,
      url: webContents ? webContents.getURL() : null,
      requestingOrigin,
      embeddingOrigin: details && details.embeddingOrigin,
      at: new Date().toISOString(),
    });
    return granted;
  });
}

ipcMain.on('probe:mode', (event) => { event.returnValue = MODE; });
ipcMain.handle('probe:store-snapshot', async () => ({ version: 1, padding: 's'.repeat(200 * 1024) }));
ipcMain.handle('probe:fixtures', async () => fixtureDescription());
ipcMain.handle('probe:console-clear', async () => { consoleMessages = []; return true; });
ipcMain.handle('probe:console-messages', async () => consoleMessages.slice());
ipcMain.handle('probe:main-static', async () => mainStaticPromise);
ipcMain.handle('probe:memory', async () => memorySnapshot());
ipcMain.handle('probe:permission-log', async () => permissionsAsked.slice());
ipcMain.handle('probe:synthetic-click', async () => {
  if (!mainWindow || mainWindow.isDestroyed()) throw new Error('main window unavailable');
  const bounds = mainWindow.getContentBounds();
  const x = Math.max(10, Math.floor(bounds.width / 2));
  const y = Math.max(10, Math.floor(bounds.height / 2));
  mainWindow.webContents.sendInputEvent({ type: 'mouseMove', x, y });
  mainWindow.webContents.sendInputEvent({ type: 'mouseDown', x, y, button: 'left', clickCount: 1 });
  mainWindow.webContents.sendInputEvent({ type: 'mouseUp', x, y, button: 'left', clickCount: 1 });
  return { x, y };
});
ipcMain.handle('probe:focus-window', async () => {
  if (!mainWindow || mainWindow.isDestroyed()) throw new Error('main window unavailable');
  if (process.platform === 'darwin') app.focus({ steal: true });
  mainWindow.show();
  mainWindow.focus();
  return mainWindow.isFocused();
});
ipcMain.handle('probe:capture-page', async () => {
  if (!mainWindow || mainWindow.isDestroyed()) throw new Error('main window unavailable');
  const samples = [];
  for (let index = 0; index < 5; index += 1) {
    const start = performance.now();
    const image = await mainWindow.webContents.capturePage();
    const png = image.toPNG();
    const size = image.getSize();
    samples.push({ index, ms: Math.round((performance.now() - start) * 10) / 10, pngBytes: png.byteLength, width: size.width, height: size.height });
  }
  const timings = samples.map((sample) => sample.ms).sort((a, b) => a - b);
  return { samples, minMs: timings[0], medianMs: timings[Math.floor(timings.length / 2)] };
});
ipcMain.handle('probe:run-strict', async () => createStrictWindow());
ipcMain.handle('probe:strict-submit', async (_event, result) => {
  if (strictPending) {
    strictPending.resolve(result);
    strictPending = null;
  }
  setTimeout(() => { if (strictWindow && !strictWindow.isDestroyed()) strictWindow.destroy(); }, 0);
  return true;
});
ipcMain.handle('probe:big', async (_event, payload, sentAtEpochMs) => {
  const before = memorySnapshot();
  const enteredAt = Date.now();
  const byteLength = payload && Number(payload.byteLength);
  const after = memorySnapshot();
  return {
    byteLength,
    receiptMs: Number.isFinite(sentAtEpochMs) ? enteredAt - sentAtEpochMs : null,
    handlerMs: Date.now() - enteredAt,
    before,
    after,
  };
});
ipcMain.handle('probe:big-reverse', async (_event, byteLength) => {
  const size = Number(byteLength);
  const before = memorySnapshot();
  const payload = new Uint8Array(size);
  if (size) { payload[0] = 17; payload[size - 1] = 29; }
  const afterAllocation = memorySnapshot();
  return { payload, main: { byteLength: size, before, afterAllocation } };
});
ipcMain.handle('probe:checkpoint', async (_event, key, result) => {
  if (PROBE_KEYS.includes(key)) {
    partial[key] = result;
    writePartial();
  }
  return true;
});
ipcMain.on('probe:complete', (_event, results) => {
  if (results && typeof results === 'object') {
    for (const key of PROBE_KEYS) if (results[key]) partial[key] = results[key];
  }
  finish('completed');
});

app.whenReady().then(async () => {
  installPermissions();
  if (APP_MODE) await protocol.handle('app', serveAppRequest);
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    show: true,
    backgroundColor: '#111827',
    webPreferences: webPreferences(),
  });
  mainWindow.webContents.on('render-process-gone', (_event, details) => {
    finish(`renderer exited: ${details.reason} (${details.exitCode})`);
  });
  mainWindow.webContents.on('console-message', (_event, ...args) => {
    const details = args[0] && typeof args[0] === 'object'
      ? args[0]
      : { level: args[0], message: args[1], lineNumber: args[2], sourceId: args[3] };
    consoleMessages.push({
      level: details.level,
      message: String(details.message || ''),
      lineNumber: details.lineNumber,
      sourceId: details.sourceId,
      at: new Date().toISOString(),
    });
    if (consoleMessages.length > 200) consoleMessages = consoleMessages.slice(-200);
  });
  mainWindow.on('closed', () => {
    mainWindow = null;
    if (!finished) finish('main window closed');
  });
  hardTimer = setTimeout(() => finish(`hard timeout after ${HARD_TIMEOUT_MS}ms`), HARD_TIMEOUT_MS);
  try {
    await loadProbePage(mainWindow, false);
  } catch (error) {
    finish(`page load failed: ${cleanError(error)}`);
  }
}).catch((error) => finish(`startup failed: ${cleanError(error)}`));

app.on('window-all-closed', () => { if (!finished) finish('all windows closed'); });
process.on('uncaughtException', (error) => finish(`uncaught exception: ${cleanError(error)}`));
process.on('unhandledRejection', (error) => finish(`unhandled rejection: ${cleanError(error)}`));
