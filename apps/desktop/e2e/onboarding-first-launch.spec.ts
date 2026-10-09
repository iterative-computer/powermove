import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { _electron, type ElectronApplication, type Page } from 'playwright';
import { expect, test, repoRoot } from './helpers/app';

async function findPage(app: ElectronApplication, predicate: (url: string) => boolean): Promise<Page> {
  await expect.poll(() => app.windows().some(page => predicate(page.url())), { timeout: 20_000 }).toBe(true);
  return app.windows().find(page => predicate(page.url()))!;
}

test('a loading editor can be closed and returns setup to a retryable state', async () => {
  test.setTimeout(45_000);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-first-launch-stalled-'));
  const env = { ...process.env as Record<string, string>, POWERMOVE_USER_DATA: userData,
    POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '0' };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({ args: ['.'], cwd: repoRoot, env });
  const appPid = await app.evaluate(() => process.pid);
  try {
    const animation = await findPage(app, url => url.endsWith('/onboarding/animation.html'));
    await animation.waitForLoadState('domcontentloaded');
    await animation.evaluate(() => (window as any).onboarding.animationComplete());
    const welcome = await findPage(app, url => url.endsWith('/onboarding/welcome.html'));
    await welcome.getByRole('button', { name: 'Begin', exact: true }).click();
    await welcome.locator('#timeline-step').getByRole('button', { name: 'Continue', exact: true }).click();
    await welcome.locator('#agent-step').getByRole('button', { name: 'Continue', exact: true }).click();
    // An entry module that never arrives reproduces a blank editor whose
    // DOM has not finished loading. Keep the existing welcome page intact.
    await app.evaluate(({ protocol }) => {
      protocol.unhandle('app');
      protocol.handle('app', request => request.url === 'app://powermove/'
        ? new Response('<!doctype html><html><body><script type="module" src="./blocked.js"></script></body></html>', { headers: { 'Content-Type': 'text/html' } })
        : new Promise<Response>(() => {}));
    });
    await welcome.getByRole('button', { name: 'Start fresh', exact: true }).click();
    const editor = await findPage(app, url => url === 'app://powermove/');
    const closed = editor.waitForEvent('close', { timeout: 8000 });
    await app.evaluate(({ BrowserWindow }) => {
      const editor = BrowserWindow.getAllWindows().find(window => window.webContents.getURL() === 'app://powermove/')!;
      editor.show(); editor.close();
    });
    await closed;
    await expect(welcome.getByRole('button', { name: 'Start fresh', exact: true })).toBeEnabled();
    await expect(welcome.getByRole('status')).toContainText('Could not open');
  } finally {
    await Promise.race([app.close().catch(() => undefined), new Promise(resolve => setTimeout(resolve, 3000))]);
    try { process.kill(appPid); } catch { /* Already closed. */ }
    await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});

for (const { action, delayedFonts } of [
  { action: 'Start fresh', delayedFonts: false },
  { action: 'Bring my workspace', delayedFonts: false },
  { action: 'Start fresh', delayedFonts: true }
]) {
  test(`first launch with ordinary window settings: ${action}${delayedFonts ? ' while fonts stay loading' : ''} opens a usable, closable editor`, async () => {
    test.setTimeout(90_000);
    const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-first-launch-'));
    const env: Record<string, string> = { ...process.env as Record<string, string>,
      POWERMOVE_USER_DATA: userData, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '0',
      ...(process.platform === 'win32' ? {} : { CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh') }) };
    delete env.ELECTRON_RUN_AS_NODE;
    const app = await _electron.launch({ args: ['.'], cwd: repoRoot, env });
    const appPid = await app.evaluate(() => process.pid);
    const errors: string[] = [];
    app.on('window', page => page.on('pageerror', error => errors.push(String(error))));
    try {
      await app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); });
      const animation = await findPage(app, url => url.endsWith('/onboarding/animation.html'));
      await animation.waitForLoadState('domcontentloaded');
      await animation.evaluate(() => (window as any).onboarding.animationComplete());
      const welcome = await findPage(app, url => url.endsWith('/onboarding/welcome.html'));
      await welcome.getByRole('button', { name: 'Begin', exact: true }).click();
      await welcome.locator('#timeline-step').getByRole('button', { name: 'Continue', exact: true }).click();
      // An unconfigured local provider keeps the import handoff in its
      // connection gate; this regression never starts a real agent run.
      await welcome.getByText('API or local model', { exact: true }).click();
      await welcome.locator('#agent-step').getByRole('button', { name: 'Continue', exact: true }).click();
      if (delayedFonts) await app.evaluate(({ app }) => {
        app.on('web-contents-created', (_event, contents) => {
          contents.once('did-finish-load', () => {
            if (contents.getURL() === 'app://powermove/') void contents.executeJavaScript("Object.defineProperty(document.fonts, 'ready', { value: new Promise(() => {}) })");
          });
        });
      });
      await welcome.getByRole('button', { name: action, exact: true }).click();
      const editor = await findPage(app, url => url === 'app://powermove/');
      await editor.waitForFunction(() => Boolean((window as any).PM?.AgentUI?.state && document.querySelector('#titlebar')), undefined, { timeout: 25_000 });
      await expect.poll(() => welcome.isClosed()).toBe(true);
      if (delayedFonts) expect(await editor.evaluate(() => Promise.race([
        document.fonts.ready.then(() => 'loaded'), new Promise(resolve => setTimeout(() => resolve('pending'), 25))
      ]))).toBe('pending');
      const saved = JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'));
      expect(saved.workspaceImport).toBe(action === 'Start fresh' ? null : 'after-effects');
      const native = await app.evaluate(({ BrowserWindow }) => {
        const window = BrowserWindow.getAllWindows().find(window => window.webContents.getURL() === 'app://powermove/')!;
        return { visible: window.isVisible(), throttling: window.webContents.getBackgroundThrottling() };
      });
      expect(native).toEqual({ visible: true, throttling: true });
      const closed = editor.waitForEvent('close', { timeout: 10_000 });
      await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(window => window.webContents.getURL() === 'app://powermove/')!.close());
      await closed;
      expect(errors).toEqual([]);
    } finally {
      // A failing boot/close must not strand the disposable regression app.
      await Promise.race([app.close().catch(() => undefined), new Promise(resolve => setTimeout(resolve, 3000))]);
      try { process.kill(appPid); } catch { /* Already closed. */ }
      await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    }
  });
}
