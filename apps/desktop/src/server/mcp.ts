import { mkdir, open, readFile, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { chromium, type Browser, type Page } from 'playwright';
import { serve, type ServeOptions, type RunningServer } from './index';
import { connectMcpHost } from '../main/agent-tools/mcp-client.mjs';
import { startMcpStdio } from '../main/agent-tools/mcp-stdio.mjs';
import { EXTERNAL_MCP_INSTRUCTIONS } from '../main/agent-tools/external-spec';

async function profileLock(userData: string) {
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
      throw new Error(`Could not start the hidden renderer. Run powermove mcp-install-browser once, then retry. ${String(error)}`);
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
        importMedia: async (_owner, args) => {
          const file = path.resolve(String(args.path));
          if (!(await stat(file)).isFile()) throw new Error('Import a regular media file.');
          if (!/\.(?:mp4|mov|m4v|webm|mkv|avi|mp3|wav|m4a|aac|aif|aiff|flac|ogg|png|jpg|jpeg|webp|gif|svg|avif|bmp|tif|tiff|heic|heif|glb|gltf|obj|fbx|blend)$/i.test(file)) throw new Error('Unsupported media file type.');
          const current = requirePage();
          await current.evaluate(() => { const input = document.createElement('input'); input.type = 'file'; input.id = 'powermove-mcp-import'; input.hidden = true; document.body.appendChild(input); });
          try {
            await current.locator('#powermove-mcp-import').setInputFiles(file);
            return await current.evaluate(async ({ at }) => {
              const PM = (window as any).PM;
              const beforeLayers = new Set(PM.proj.layers.map((layer: any) => layer.id));
              const beforeAssets = new Set(Object.keys(PM.proj.assets || {}));
              const input = document.getElementById('powermove-mcp-import') as HTMLInputElement;
              await PM.importFiles(Array.from(input.files!), { placement: at === undefined ? null : { at } });
              const assets = Object.values<any>(PM.proj.assets || {}).filter(asset => !beforeAssets.has(asset.id)).map(asset => ({ id: asset.id, name: asset.name, kind: asset.kind }));
              const layers = PM.proj.layers.filter((layer: any) => !beforeLayers.has(layer.id)).map((layer: any) => ({ id: layer.id, name: layer.name, type: layer.type }));
              if (!assets.length && !layers.length) throw new Error('Powermove did not import the file. Inspect get_workspace_state for errors.');
              return { assets, layers };
            }, { at: args.at as number | undefined });
          } finally { await current.locator('#powermove-mcp-import').evaluate(element => element.remove()).catch(() => {}); }
        }
      }
    });
    await context.addCookies([{ name: 'pm_session', value: host.token, url: new URL(host.url).origin }]);
    await page.goto(new URL(host.url).origin, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => { const PM = (window as any).PM; return PM?.newBlankProject && PM?.Export && PM?.Edit && PM?.GL; }, { timeout: 60000 });
    await Promise.race([ready, new Promise<never>((_resolve, reject) => { const timer = setTimeout(() => reject(new Error('The local MCP bridge did not start.')), 10000); timer.unref(); })]);
    const client = await connectMcpHost(options.userData);
    await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS, close: async () => { await client.close(); await close(); } });
  } finally { process.off('SIGINT', signal); process.off('SIGTERM', signal); await close(); }
}

export async function runConnectedMcp(userData: string): Promise<void> {
  const client = await connectMcpHost(userData);
  await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS });
}
