import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { build } from 'esbuild';
import { expect, it } from 'vitest';
import { IPC } from '../shared/ipc';

interface CompiledUpdater { modulePath: string; updaterPath: string; directory: string }

async function withUpdaterModule(verify: (module: CompiledUpdater) => void): Promise<void> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-updater-startup-'));
  try {
    const require = createRequire(import.meta.url);
    const updaterPath = require.resolve('electron-updater');
    const modulePath = path.join(directory, 'updates.cjs');
    await build({
      entryPoints: [path.join(import.meta.dirname, 'updates.ts')],
      outfile: modulePath, bundle: true, platform: 'node', format: 'cjs',
      alias: { 'electron-updater': updaterPath }, external: ['electron', updaterPath]
    });
    verify({ modulePath, updaterPath, directory });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

function electronProcess(module: CompiledUpdater, packaged: boolean, inspect: string): unknown {
  const { modulePath, updaterPath, directory } = module;
  // A fresh process avoids both Vitest's package mock and Node's module
  // cache. Electron alone is stubbed; loading the updater is real.
  return JSON.parse(execFileSync(process.execPath, ['-e', `
        const Module = require('node:module');
        const { EventEmitter } = require('node:events');
        Object.defineProperty(process, 'platform', { value: 'darwin' });
        const originalLoad = Module._load;
        let quits = 0;
        const app = Object.assign(new EventEmitter(), {
          isPackaged: ${packaged}, getVersion: () => '1.1.3',
          getName: () => 'Powermove', getPath: () => ${JSON.stringify(directory)},
          getAppPath: () => ${JSON.stringify(directory)}, whenReady: () => Promise.resolve(),
          quit: () => { quits++; }
        });
        const handlers = new Map();
        let menuItems = 0;
        const electron = {
          app, autoUpdater: new EventEmitter(), BrowserWindow: { getAllWindows: () => [] },
          dialog: { showMessageBox: () => Promise.resolve({ response: 0 }) },
          ipcMain: { handle: (channel, handler) => handlers.set(channel, handler) },
          Menu: { setApplicationMenu: () => {} },
          MenuItem: class { constructor(options) { Object.assign(this, options); } }
        };
        Module._load = function(request, parent, isMain) {
          if (request === 'electron') return electron;
          if (request === 'electron-updater') return originalLoad.call(this, ${JSON.stringify(updaterPath)}, parent, isMain);
          return originalLoad.call(this, request, parent, isMain);
        };
        require(${JSON.stringify(modulePath)}).installUpdates({ items: [{ submenu: { insert: () => menuItems++ } }] });
        (async () => {
          ${inspect}
          app.emit('will-quit');
        })().catch(error => { console.error(error); process.exit(1); });
  `], { encoding: 'utf8', timeout: 10_000 }));
}

it('keeps the real update runtime out of both packaged and development startup', async () => {
  await withUpdaterModule((module) => {
    for (const packaged of [true, false]) {
      const result = electronProcess(module, packaged, `
        console.log(JSON.stringify({ updaterLoaded: !!require.cache[${JSON.stringify(module.updaterPath)}], handlers: handlers.size, menuItems }));
      `);
      expect(result).toEqual({ updaterLoaded: false, handlers: packaged ? 3 : 0, menuItems: packaged ? 1 : 0 });
    }
  });
});

it('loads the real CommonJS updater on demand and retains normal-quit installation', async () => {
  await withUpdaterModule((module) => {
    const result = electronProcess(module, true, `
      const updater = require(${JSON.stringify(module.updaterPath)}).autoUpdater;
      updater.logger = null;
      let checks = 0;
      // Exercise the real module namespace and instance without network I/O.
      updater.checkForUpdates = async () => {
        checks++;
        updater.emit('update-not-available', { version: '1.1.3' });
        return null;
      };
      await handlers.get(${JSON.stringify(IPC.updateCheck)})();
      const checked = handlers.get(${JSON.stringify(IPC.updateStatus)})();
      updater.emit('update-available', { version: '1.1.4' });
      electron.autoUpdater.emit('update-downloaded');
      const staged = handlers.get(${JSON.stringify(IPC.updateStatus)})();
      handlers.get(${JSON.stringify(IPC.updateInstall)})();
      console.log(JSON.stringify({ checks, checked, staged, quits,
        autoInstall: updater.autoInstallOnAppQuit, relaunch: updater.autoRunAppAfterInstall }));
    `);
    expect(result).toEqual({
      checks: 1, checked: { status: 'idle', current: '1.1.3', version: null },
      staged: { status: 'ready', current: '1.1.3', version: '1.1.4' },
      quits: 1, autoInstall: true, relaunch: true
    });
  });
});
