import { nativeHiddenHostEnabled } from './hidden-engine-policy';
import { mkdir, open, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { chromium, type Browser, type Page } from 'playwright';
import { serve, type ServeOptions, type RunningServer } from './index';
import { connectMcpHost } from '../main/agent-tools/mcp-client.mjs';
import { startMcpStdio } from '../main/agent-tools/mcp-stdio.mjs';
import { EXTERNAL_MCP_INSTRUCTIONS } from '../main/agent-tools/external-spec';
import { importFileInPage } from './page-import';

export async function profileLock(userData: string) {
  await mkdir(userData, { recursive: true, mode: 0o700 });
  const file = path.join(userData, 'mcp-host.lock');
  const acquire = async () => { const handle = await open(file, 'wx', 0o600); await handle.writeFile(String(process.pid)); await handle.close(); };
  try { await acquire(); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
    const pid = Number(await readFile(file, 'utf8'));
    let live = true;
    try { if (!Number.isInteger(pid) || pid < 1) throw new Error('Invalid lock'); process.kill(pid, 0); }
    catch (error) { live = (error as NodeJS.ErrnoException).code === 'EPERM'; }
    if (live) throw new Error('A hidden Powermove host already uses this profile. Use --connect --user-data <profile> to share it, or --user-data <another-directory> for an independent host.');
    await rm(file, { force: true }); await acquire();
  }
  return async () => { await rm(file, { force: true }); };
}

/** Runtime rendering is intentionally done in Chromium using the production
 * editor, never a parallel implementation of Powermove's document model. */
export async function runHeadlessMcp(options: ServeOptions): Promise<void> {
  if (await nativeHiddenHostEnabled(options.userData)) {
    const { runNativeHiddenMcp } = await import('./native/mcp');
    return runNativeHiddenMcp(options);
  }
  const unlock = await profileLock(options.userData);
  let browser: Browser | undefined, host: RunningServer | undefined, page: Page | undefined;
  let external: { close(): Promise<void> } | undefined;
  let resolveReady!: () => void;
  const ready = new Promise<void>(resolve => { resolveReady = resolve; });
  const requirePage = () => { if (!page || page.isClosed()) throw new Error('The hidden Powermove renderer is unavailable. Restart the MCP server.'); return page; };
  let closing: Promise<void> | undefined;
  const close = () => closing ??= (async () => {
    await external?.close();
    await browser?.close();
    await host?.close();
    await unlock();
  })();
  const signal = () => { void close().finally(() => process.exit(0)); };
  process.once('SIGINT', signal); process.once('SIGTERM', signal);
  try {
    browser = await chromium.launch({ headless: true, channel: 'chromium', args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] }).catch(error => {
      throw new Error(`chromium-unavailable: Could not start the hidden renderer. Run powermove mcp-install-browser once, then retry. ${String(error)}`);
    });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, acceptDownloads: false });
    page = await context.newPage();
    // A headless host cannot ask a person; preserve confirmation semantics by
    // declining. Exports use the host's automatic destination, not dialogs.
    page.on('dialog', dialog => { void dialog.dismiss(); });
    host = await serve({ ...options, host: '127.0.0.1', port: 0, insecure: true, engineScript: null,
      log: line => process.stderr.write(`${line}\n`),
      externalMcp: {
        ready: value => { external = value; resolveReady(); },
        capturePanel: async (_owner, clip) => requirePage().screenshot({ type: 'jpeg', quality: 85, clip }),
        panelInput: async (_owner, args, points) => {
          const current = requirePage(), first = points[0];
          if (!first) throw new Error('Panel input has no target.');
          await current.mouse.move(first.x, first.y);
          if (args.action === 'scroll') await current.mouse.wheel(0, Number(args.deltaY));
          else if (args.action === 'drag') {
            await current.mouse.down();
            try { for (const point of points.slice(1)) await current.mouse.move(point.x, point.y); }
            finally { await current.mouse.up(); }
          } else {
            await current.mouse.click(first.x, first.y);
            if (args.action === 'type') await current.keyboard.insertText(String(args.text));
            if (args.action === 'press') await current.keyboard.press(String(args.key));
          }
        },
        importMedia: async (_owner, args) => importFileInPage(requirePage(), args)
      }
    });
    await context.addCookies([{ name: 'pm_session', value: host.token, url: new URL(host.url).origin }]);
    await page.goto(new URL(host.url).origin, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => { const PM = (window as any).PM; return PM?.newBlankProject && PM?.Export && PM?.Edit && PM?.GL; }, { timeout: 60000 });
    await page.waitForFunction(() => !!(window as any).PM?.Kernel?.loader, { timeout: 60000 });
    await page.evaluate(async () => { const PM = (window as any).PM; await PM.EngineDoc?.whenReady; await PM.Kernel.loader.boot(); await PM.Kernel.loader.whenIdle(); });
    await Promise.race([ready, new Promise<never>((_resolve, reject) => { const timer = setTimeout(() => reject(new Error('The local MCP bridge did not start.')), 10000); timer.unref(); })]);
    const client = await connectMcpHost(options.userData);
    await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS, close: async () => { await client.close(); await close(); } });
  } finally { process.off('SIGINT', signal); process.off('SIGTERM', signal); await close(); }
}

export async function runConnectedMcp(userData: string): Promise<void> {
  const client = await connectMcpHost(userData);
  await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS });
}
