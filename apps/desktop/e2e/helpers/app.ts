import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { expect, test as base, type TestInfo } from '@playwright/test';
import { _electron, type ElectronApplication, type Page } from 'playwright';

/**
 * Page-side service convention for e2e specs: inside each `page.evaluate` or
 * `page.waitForFunction` callback, read extension-owned providers from
 * `PM.Kernel.services` into descriptive locals named `viewer`, `timeline`,
 * `inspector`, `tool`, or `shaderHooks`. For example:
 *
 *   const PM = (window as any).PM;
 *   const viewer = PM.Kernel.services.get('viewer');
 *
 * Playwright callbacks run in the renderer and cannot close over module-side
 * helper functions, so keeping this lookup page-side is intentional.
 */

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
  /** Use production window preferences while retaining an isolated test profile. */
  normalWindow?: boolean;
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
  /** Opens the agent popover under its titlebar launcher. */
  openAgent(): Promise<void>;
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
  diagnostics: RendererDiagnostics,
  normalWindow = false,
): Promise<{ app: ElectronApplication; page: Page }> {
  await ensureBuilt();
  const launchEnv = {
    ...process.env,
    // Offline UI regressions must not depend on the developer's subscription
    // state (or race a real account check that replaces the composer).
    ...(process.env.POWERMOVE_E2E_LIVE ? {} : {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
      POWERMOVE_FAKE_CHATGPT_STATUS: 'connected',
    }),
    ...env,
    POWERMOVE_USER_DATA: userData,
    POWERMOVE_DEVTOOLS: '0',
    POWERMOVE_BACKGROUND_TEST: normalWindow ? '0' : '1',
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
  await page.waitForFunction(() => Boolean((window as any).PM?.app));
  await page.evaluate(async () => { await (window as any).PM.projectReady; });
  return { app, page };
}

export async function launchApp(options: LaunchOptions = {}): Promise<LaunchedApp> {
  const ownsUserData = options.userData === undefined;
  const userData = options.userData
    ? path.resolve(options.userData)
    : await mkdtemp(path.join(os.tmpdir(), 'powermove-e2e-'));
  const env = { ...(options.env ?? {}) };
  if (options.normalWindow) {
    const temporaryRoots = [os.tmpdir(), '/tmp', '/private/tmp'];
    const isolated = temporaryRoots.some(root => {
      const relative = path.relative(root, userData);
      return !!relative && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
    });
    if (!isolated) throw new Error('Normal-window tests require a copied profile in a temporary directory.');
    // Start directly in the editor with ordinary BrowserWindow preferences;
    // no post-launch mutation of throttling/focus can reproduce this path.
    await mkdir(userData, { recursive: true });
    await writeFile(path.join(userData, 'onboarding-v1.json'), '{"version":1}\n', { mode: 0o600 });
  }
  const diagnostics: RendererDiagnostics = { console: [], pageErrors: [] };
  let active = await startElectron(userData, env, diagnostics, options.normalWindow);
  let closed = false;

  const session: LaunchedApp = {
    app: active.app,
    page: active.page,
    userData,
    diagnostics,
    async relaunch() {
      await active.app.close();
      active = await startElectron(userData, env, diagnostics, options.normalWindow);
      session.app = active.app;
      session.page = active.page;
    },
    async openEditor() {
      const { page } = session;
      await page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen && (window as any).PM?.mkProject));
      await page.evaluate(async () => { await (window as any).PM.Kernel.loader.builtinsReady; });
      await page.evaluate(() => {
        const PM = (window as any).PM;
        if (!PM.ProjectsScreen.isOpen) return;
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'E2E composition' }) }));
        PM.ProjectsScreen.hide();
      });
      await page.waitForFunction(() => !(window as any).PM.ProjectsScreen.isOpen);
      await page.waitForSelector('#body .dock', { state: 'visible' });
      await page.waitForFunction(() => {
        const PM = (window as any).PM;
        return Boolean(PM.GL?.gl && PM.Kernel.services.get('viewer')?.stage?.isConnected);
      });
    },
    async openAgent() {
      const { page } = session;
      await page.waitForFunction(() => Boolean((window as any).PM?.AgentShell));
      await page.evaluate(() => { const PM = (window as any).PM; if (!PM.AgentShell.isOpen()) PM.SpatialAssistant?.open?.() ?? PM.AgentShell.open(); });
      await page.locator('#agent-popover #panel-agent').waitFor({ state: 'visible' });
      // Measure the settled popover, not its opening scale.
      await page.waitForFunction(() => document.getElementById('agent-popover')!.getAnimations().length === 0);
    },
    async close() {
      if (closed) return;
      closed = true;
      // A failed close-decision assertion must not leave its Cancel mock
      // preventing cleanup of this disposable test process.
      await active.app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); }).catch(() => undefined);
      await active.app.close().catch(() => undefined);
      if (ownsUserData) await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
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

export const test = base.extend<{ session: LaunchedApp; desktopLaunchOptions: LaunchOptions }>({
  desktopLaunchOptions: [{}, { option: true }],
  session: async ({ desktopLaunchOptions }, use, testInfo) => {
    const session = await launchApp(desktopLaunchOptions);
    try {
      await use(session);
    } finally {
      await session.close();
      await attachDiagnostics(testInfo, session.diagnostics);
    }
  }
});

export { expect };

const MENU_PANEL = '[data-svelte-overlay-menu] .menu-panel';

/** Labels of the menu panel at `level`, in order, separators left out. */
async function menuLabels(page: Page, level: number): Promise<{ label: string; enabled: boolean }[]> {
  const panel = page.locator(`${MENU_PANEL}[data-level="${level}"]`);
  await panel.waitFor({ state: 'visible' });
  return panel.locator('.di').evaluateAll(rows => rows.map(row => ({
    label: (row.querySelector('.label')?.textContent ?? '').trim(),
    enabled: row.getAttribute('aria-disabled') !== 'true'
  })));
}

/** Choose a row of a Powermove menu (overlays/Menu.svelte). Each label after
 * the first is chosen from the submenu the previous one opens. Resolves with
 * the labels of the last menu level shown. (The name predates Powermove
 * drawing its own menus; it no longer involves AppKit.) */
export async function chooseNativeMenu(session: LaunchedApp, label: string | readonly string[], trigger: () => Promise<unknown>): Promise<string[]> {
  const choices = Array.isArray(label) ? [...label] : [label as string];
  await trigger();
  const page = session.page;
  let shown: { label: string; enabled: boolean }[] = [];
  for (const [level, choice] of choices.entries()) {
    shown = await menuLabels(page, level);
    const index = shown.findIndex(row => row.label === choice && row.enabled);
    if (index < 0) throw new Error(`Missing enabled menu item: ${choice}; got ${shown.map(row => row.label).join(', ')}`);
    await page.locator(`${MENU_PANEL}[data-level="${level}"] .di`).nth(index).click();
  }
  await expect(page.locator(MENU_PANEL)).toHaveCount(0);
  return shown.map(row => row.label);
}

/** Read a Powermove menu's rows and whether each can run, then dismiss it without running anything. */
export async function inspectNativeMenu(session: LaunchedApp, trigger: () => Promise<unknown>): Promise<{ label: string; enabled: boolean }[]> {
  await trigger();
  const page = session.page;
  try {
    return await menuLabels(page, 0);
  } finally {
    await page.evaluate(() => (window as any).PM?.closeMenus?.());
  }
}
