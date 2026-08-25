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
  const app = await _electron.launch({
    args: ['.'],
    cwd: repoRoot,
    env: {
      ...process.env,
      ...env,
      POWERMOVE_USER_DATA: userData,
      POWERMOVE_DEVTOOLS: '0'
    }
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
  // Never attach to a DevTools window; wait for the app document itself.
  const isAppPage = (p: Page): boolean => p.url().startsWith('app://') || p.url().startsWith('http://localhost');
  let page = app.windows().find(isAppPage);
  while (!page) {
    const next = await app.waitForEvent('window');
    if (isAppPage(next)) page = next;
  }
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
