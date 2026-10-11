/**
 * Node-side host for Playwright's Electron driver.
 *
 * Bun 1.3.14 cannot host `_electron.launch`: the driver attaches to Electron's
 * Node inspector and DevTools endpoint and then never resolves (it times out
 * after 180 s, while the same call under Node 24 returns in about 300 ms).
 * The capture tool therefore stays a Bun program and drives the app through
 * this small bridge, which Node runs with built-in type stripping (erasable
 * TypeScript only: no enums, namespaces, or parameter properties).
 *
 * Protocol over the IPC channel (JSON serialization):
 *   request   { id, op, args }
 *   response  { id, ok: true, value } | { id, ok: false, error }
 *   event     { event: 'pageerror' | 'console-error', text }
 *
 * Page functions arrive as source text and are evaluated through CDP
 * (Runtime.evaluate), which is what Playwright does with a function argument;
 * the renderer's CSP does not apply to it.
 */
import { createRequire } from 'node:module';
import path from 'node:path';

interface Request { id: number; op: string; args: Record<string, unknown> }

const desktopRoot = process.argv[2];
if (!desktopRoot) { console.error('usage: node electron-bridge.mts <apps/desktop>'); process.exit(2); }
const desktopRequire = createRequire(path.join(desktopRoot, 'package.json'));
const { _electron } = desktopRequire('playwright');

let app: any = null;
let page: any = null;

const send = (message: unknown): Promise<void> => new Promise((resolve, reject) => {
  process.send!(message, undefined, {}, (error: Error | null) => error ? reject(error) : resolve());
});

const call = (source: string, arg: unknown): string => `(${source})(${arg === undefined ? '' : JSON.stringify(arg)})`;

function requirePage(): any {
  if (!page) throw new Error('The app is not launched');
  return page;
}

async function launch(args: Record<string, unknown>): Promise<unknown> {
  if (app) throw new Error('The app is already launched');
  const env = { ...(args.env as Record<string, string>) };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await _electron.launch({ args: ['.'], cwd: desktopRoot, env, timeout: Number(args.timeout) || 120_000 });
  // Native dialogs are substituted, never shown on the user's desktop (as e2e/helpers/app.ts).
  await app.evaluate(({ dialog }: any) => {
    dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' });
    dialog.showOpenDialog = async () => ({ canceled: true, filePaths: [] });
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const instrumented = new WeakSet<object>();
  const instrument = (target: any) => {
    if (instrumented.has(target)) return;
    instrumented.add(target);
    target.on('pageerror', (error: Error) => { void send({ event: 'pageerror', text: error.stack || error.message }); });
    target.on('console', (message: any) => { if (message.type() === 'error') void send({ event: 'console-error', text: message.text() }); });
  };
  app.on('window', instrument);
  app.windows().forEach(instrument);
  page = await app.firstWindow();
  instrument(page);
  await page.waitForURL((url: URL) => url.protocol === 'app:' || url.hostname === 'localhost');
  await page.waitForLoadState('domcontentloaded');
  return { url: page.url() };
}

async function handle(request: Request): Promise<unknown> {
  const { op, args } = request;
  switch (op) {
    case 'launch': return launch(args);
    case 'evaluate': return requirePage().evaluate(call(String(args.source), args.arg));
    case 'waitForFunction': {
      // page.waitForFunction with a string predicate evaluates it with eval() inside the
      // page, which the renderer's CSP forbids; poll through Runtime.evaluate instead.
      const expression = call(String(args.source), args.arg), timeout = Number(args.timeout) || 60_000;
      const deadline = Date.now() + timeout;
      for (;;) {
        if (await requirePage().evaluate(expression)) return null;
        if (Date.now() > deadline) throw new Error(`waitForFunction timed out after ${timeout} ms: ${String(args.source).slice(0, 160)}`);
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    }
    case 'appEvaluate': {
      if (!app) throw new Error('The app is not launched');
      // eslint-disable-next-line no-new-func
      const fn = new Function(`return (${String(args.source)});`)();
      return app.evaluate(fn, args.arg);
    }
    case 'setInputFiles': {
      await requirePage().locator(String(args.selector)).setInputFiles(args.files as string[]);
      return null;
    }
    case 'close': {
      const current = app;
      app = null; page = null;
      if (current) {
        await current.evaluate(({ dialog }: any) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); }).catch(() => undefined);
        await current.close().catch(() => undefined);
      }
      return null;
    }
    default: throw new Error(`Unknown bridge operation ${op}`);
  }
}

process.on('message', (request: Request) => {
  handle(request).then(
    value => send({ id: request.id, ok: true, value: value === undefined ? null : value }),
    (error: Error) => send({ id: request.id, ok: false, error: (error && (error.stack || error.message)) || String(error) }),
  ).then(() => { if (request.op === 'close' && request.args.exit) process.exit(0); });
});
process.on('disconnect', () => {
  const current = app;
  app = null;
  if (current) current.close().catch(() => undefined).finally(() => process.exit(0));
  else process.exit(0);
});
void send({ event: 'ready' });
