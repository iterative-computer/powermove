/**
 * Launch the built legacy desktop app the way the e2e harness does
 * (apps/desktop/e2e/helpers/app.ts): Playwright's Electron driver, a hidden
 * background window, an isolated temporary user-data profile, native dialogs
 * substituted, DevTools off.
 *
 * The driver runs in a Node child process (electron-bridge.mts) because Bun
 * cannot host it; see that file. Page functions passed here must be
 * self-contained: they are sent as source text and run in the renderer.
 */
import { existsSync } from 'node:fs';
import { cp, mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { Subprocess } from 'bun';
import { desktopRoot, fixturesDir } from './catalog';

export const mainEntry = path.join(desktopRoot, 'out/main/index.js');
const bridgeEntry = path.join(import.meta.dir, 'electron-bridge.mts');

type PageFunction<A, R> = (arg: A) => R | Promise<R>;

export interface LegacyApp {
  userData: string;
  pageErrors: string[];
  consoleErrors: string[];
  /** Run a self-contained function in the renderer (`page.evaluate`). */
  evaluate<R, A = undefined>(fn: PageFunction<A, R>, arg?: A): Promise<R>;
  /** Evaluate a script's source text in the renderer as one expression (Runtime.evaluate, so the renderer's CSP does not apply). */
  evaluateScript<R = unknown>(source: string): Promise<R>;
  /** `page.waitForFunction` with a self-contained predicate. */
  waitFor<A = undefined>(fn: PageFunction<A, unknown>, arg?: A, timeout?: number): Promise<void>;
  /** Run a function in Electron's main process (`electronApp.evaluate`); it receives the electron module. */
  evaluateMain<R, A = undefined>(fn: (electron: any, arg: A) => R | Promise<R>, arg?: A): Promise<R>;
  setInputFiles(selector: string, files: string[]): Promise<void>;
  close(): Promise<void>;
}

class Bridge {
  private next = 1;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  private ready: Promise<void>;
  readonly process: Subprocess;
  exited = false;

  constructor(onEvent: (event: string, text: string) => void) {
    let markReady!: () => void;
    this.ready = new Promise(resolve => { markReady = resolve; });
    this.process = Bun.spawn(['node', bridgeEntry, desktopRoot], {
      cwd: desktopRoot,
      serialization: 'json',
      stdio: ['ignore', 'inherit', 'inherit'],
      ipc: (message: any) => {
        if (message?.event === 'ready') { markReady(); return; }
        if (message?.event) { onEvent(message.event, String(message.text)); return; }
        const waiter = this.pending.get(message?.id);
        if (!waiter) return;
        this.pending.delete(message.id);
        if (message.ok) waiter.resolve(message.value);
        else waiter.reject(new Error(String(message.error)));
      },
    });
    void this.process.exited.then(code => {
      this.exited = true;
      for (const waiter of this.pending.values()) waiter.reject(new Error(`The Electron bridge exited (${code})`));
      this.pending.clear();
      markReady();
    });
  }

  async request<R>(op: string, args: Record<string, unknown> = {}): Promise<R> {
    await this.ready;
    if (this.exited) throw new Error('The Electron bridge is not running');
    const id = this.next++;
    return new Promise<R>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.process.send({ id, op, args });
    });
  }
}

export interface LaunchOptions {
  /** e2e fixture extensions to install into the profile before launch (as typewriter-raster.spec.ts does). */
  extensions?: string[];
}

export async function launchLegacyApp(options: LaunchOptions = {}): Promise<LegacyApp> {
  if (!existsSync(mainEntry)) throw new Error(`The legacy app is not built (${mainEntry} is missing). Run: bun run --cwd apps/desktop build`);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-corpus-'));
  const extensions = options.extensions ?? [];
  for (const name of extensions) {
    const source = path.join(fixturesDir, name);
    if (!existsSync(path.join(source, 'manifest.json'))) throw new Error(`No fixture extension ${name} in ${fixturesDir}`);
    await cp(source, path.join(userData, 'extensions', name), { recursive: true });
  }
  const pageErrors: string[] = [], consoleErrors: string[] = [];
  const bridge = new Bridge((event, text) => (event === 'pageerror' ? pageErrors : consoleErrors).push(text));
  const env: Record<string, string> = {
    ...process.env as Record<string, string>,
    CODEX_BINARY: path.join(desktopRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
    POWERMOVE_FAKE_CHATGPT_STATUS: 'connected',
    POWERMOVE_USER_DATA: userData,
    POWERMOVE_DEVTOOLS: '0',
    POWERMOVE_BACKGROUND_TEST: '1',
    // Corpus launches are long-lived and killed: never start a runtime download.
    POWERMOVE_DISABLE_RUNTIME_UPDATES: '1',
  };
  delete env.ELECTRON_RUN_AS_NODE;
  let closed = false, electronPid: number | undefined;
  const legacy: LegacyApp = {
    userData, pageErrors, consoleErrors,
    evaluate: (fn, arg) => bridge.request('evaluate', { source: fn.toString(), arg }),
    evaluateScript: source => bridge.request('script', { source }),
    waitFor: async (fn, arg, timeout = 60_000) => { await bridge.request('waitForFunction', { source: fn.toString(), arg, timeout }); },
    evaluateMain: (fn, arg) => bridge.request('appEvaluate', { source: fn.toString(), arg }),
    setInputFiles: async (selector, files) => { await bridge.request('setInputFiles', { selector, files }); },
    async close() {
      if (closed) return;
      closed = true;
      // The bridge force-kills Electron after 20 s; this outer limit covers a wedged bridge.
      let confirmed = false;
      if (!bridge.exited) {
        const result = await Promise.race([bridge.request<{ forced: boolean } | null>('close', { exit: true, timeout: 20_000 }).catch(() => undefined), Bun.sleep(40_000)]);
        confirmed = !!result || result === null;
      }
      await Promise.race([bridge.process.exited, Bun.sleep(5_000)]);
      if (!bridge.exited) bridge.process.kill('SIGKILL');
      // Without the bridge's confirmation Electron may still be running (a wedged or crashed bridge).
      if (!confirmed && electronPid) { try { process.kill(electronPid, 'SIGKILL'); } catch { /* already gone */ } }
      await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    },
  };
  try {
    ({ pid: electronPid } = await bridge.request<{ pid?: number }>('launch', { env, timeout: 120_000 }));
    await legacy.waitFor(() => Boolean((window as any).PM?.ProjectsScreen && (window as any).PM?.mkProject), undefined, 60_000);
    await legacy.evaluate(async () => { await (window as any).PM.Kernel.loader.builtinsReady; });
    // Leave the Projects home for an editor composition, as the e2e helper's openEditor does.
    await legacy.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.ProjectsScreen.isOpen) return;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Corpus capture' }) }));
      PM.ProjectsScreen.hide();
    });
    await legacy.waitFor(() => !(window as any).PM.ProjectsScreen.isOpen);
    await legacy.waitFor(() => Boolean((window as any).PM.GL?.gl && (window as any).PM.Kernel.services.get('viewer')?.stage?.isConnected), undefined, 60_000);
    if (extensions.length) await legacy.waitFor((ids: string[]) => ids.every(id => (window as any).PM.Kernel.loader.activeIds().includes(id)), extensions, 60_000);
  } catch (error) {
    await legacy.close();
    throw error;
  }
  return legacy;
}
