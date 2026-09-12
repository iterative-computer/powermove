import { execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { expect, test as base, type TestInfo } from '@playwright/test';
import { _electron, type ElectronApplication, type Page } from 'playwright';

const execFileAsync = promisify(execFile);
export const repoRoot = path.resolve(__dirname, '../..');
const mainEntry = path.join(repoRoot, 'out/main/index.js');
let buildPromise: Promise<void> | undefined;

export type ConsoleRecord = {
  type: string;
  text: string;
  location: string;
};

export type RendererDiagnostics = {
  console: ConsoleRecord[];
  pageErrors: string[];
};

export type LaunchOptions = {
  userData?: string;
  env?: Record<string, string>;
};

export type LaunchedApp = {
  app: ElectronApplication;
  page: Page;
  readonly userData: string;
  readonly diagnostics: RendererDiagnostics;
  relaunch(): Promise<void>;
  /** A fresh profile boots to the Projects home screen. Specs that exercise
   * editor panels open an empty composition first, the way a user would. */
  openEditor(): Promise<void>;
  close(): Promise<void>;
};

/** The Playwright session builds exactly once in global setup. Direct helper users
 * can rely on an existing out/ tree, or call this when the entry does not exist. */
export function ensureBuilt(force = false): Promise<void> {
  if (!buildPromise) {
    buildPromise = (async () => {
      if (!force) {
        try {
          await import('node:fs/promises').then(fs => fs.access(mainEntry));
          return;
        } catch {
          // A direct helper invocation needs a build when out/ is absent.
        }
      }
      await execFileAsync('npm', ['run', 'build'], {
        cwd: repoRoot,
        env: process.env,
        maxBuffer: 10 * 1024 * 1024
      });
    })();
  }
  return buildPromise;
}

async function startElectron(
  userData: string,
  env: Record<string, string>,
  diagnostics: RendererDiagnostics
): Promise<{ app: ElectronApplication; page: Page }> {
  await ensureBuilt();
  const launchEnv = {
    ...process.env,
    ...env,
    POWERMOVE_USER_DATA: userData,
    POWERMOVE_DEVTOOLS: '0',
    POWERMOVE_BACKGROUND_TEST: '1',
  };
  // Codex and some Node launchers set this for their own Electron subprocesses.
  // Passing it through makes Electron run as plain Node, which rejects the
  // Chromium debugging flag Playwright needs before our app can even start.
  delete launchEnv.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({
    args: ['.'],
    cwd: repoRoot,
    env: launchEnv
  });
  // Native dialogs are substituted in the hidden harness, never shown on the user's desktop.
  // Individual tests can supply chosen paths or decisions while retaining real file IPC.
  await app.evaluate(({ dialog }) => {
    dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' });
    dialog.showOpenDialog = async () => ({ canceled: true, filePaths: [] });
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const instrumented = new WeakSet<Page>();
  const instrument = (target: Page): void => {
    if (instrumented.has(target)) return;
    instrumented.add(target);
    target.on('console', message => {
      const location = message.location();
      diagnostics.console.push({
        type: message.type(),
        text: message.text(),
        location: location.url
          ? `${location.url}:${location.lineNumber ?? 0}:${location.columnNumber ?? 0}`
          : ''
      });
    });
    target.on('pageerror', error => diagnostics.pageErrors.push(error.stack || error.message));
  };
  app.on('window', instrument);
  app.windows().forEach(instrument);
  // Hidden windows are announced before navigation. Waiting for a second
  // window here would hang forever. DevTools is disabled in background mode.
  const page = await app.firstWindow();
  await page.waitForURL(url => url.protocol === 'app:' || url.hostname === 'localhost');
  instrument(page);
  await page.waitForLoadState('domcontentloaded');
  await page.waitForFunction(() => Boolean((window as any).PM));
  return { app, page };
}

export async function launchApp(options: LaunchOptions = {}): Promise<LaunchedApp> {
  const ownsUserData = options.userData === undefined;
  const userData = options.userData
    ? path.resolve(options.userData)
    : await mkdtemp(path.join(os.tmpdir(), 'powermove-e2e-'));
  const env = { ...(options.env ?? {}) };
  const diagnostics: RendererDiagnostics = { console: [], pageErrors: [] };
  let active = await startElectron(userData, env, diagnostics);
  let closed = false;

  const session: LaunchedApp = {
    app: active.app,
    page: active.page,
    userData,
    diagnostics,
    async relaunch() {
      await active.app.close();
      active = await startElectron(userData, env, diagnostics);
      session.app = active.app;
      session.page = active.page;
    },
    async openEditor() {
      const { page } = session;
      await page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen && (window as any).PM?.mkProject));
      await page.evaluate(() => {
        const PM = (window as any).PM;
        if (!PM.ProjectsScreen.isOpen) return;
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'E2E composition' }) }));
        PM.ProjectsScreen.hide();
      });
      await page.waitForFunction(() => !(window as any).PM.ProjectsScreen.isOpen);
      await page.waitForSelector('#body .dock', { state: 'visible' });
    },
    async close() {
      if (closed) return;
      closed = true;
      await active.app.close().catch(() => undefined);
      if (ownsUserData) await rm(userData, { recursive: true, force: true });
    }
  };
  return session;
}

async function attachDiagnostics(testInfo: TestInfo, diagnostics: RendererDiagnostics): Promise<void> {
  const consoleText = diagnostics.console
    .map(record => `[${record.type}] ${record.text}${record.location ? ` (${record.location})` : ''}`)
    .join('\n');
  await testInfo.attach('renderer-console', {
    body: Buffer.from(consoleText || '(no renderer console messages)'),
    contentType: 'text/plain'
  });
  await testInfo.attach('renderer-pageerrors', {
    body: Buffer.from(diagnostics.pageErrors.join('\n\n') || '(no renderer page errors)'),
    contentType: 'text/plain'
  });
}

export const test = base.extend<{ session: LaunchedApp }>({
  session: async ({}, use, testInfo) => {
    const session = await launchApp();
    try {
      await use(session);
    } finally {
      await session.close();
      await attachDiagnostics(testInfo, session.diagnostics);
    }
  }
});

export { expect };
