import { profileLock } from '../mcp';
import { serve, type RunningServer, type ServeOptions } from '../index';
import { LazyNativeBrowser } from './browser';
import { connectMcpHost } from '../../main/agent-tools/mcp-client.mjs';
import { startMcpStdio } from '../../main/agent-tools/mcp-stdio.mjs';
import { EXTERNAL_MCP_INSTRUCTIONS } from '../../main/agent-tools/external-spec';

export async function runNativeHiddenMcp(options: ServeOptions): Promise<void> {
  const unlock = await profileLock(options.userData);
  let host: RunningServer | undefined;
  const browser = new LazyNativeBrowser(() => { if (!host) throw new Error('native-host-starting'); return host; }, line => process.stderr.write(`${line}\n`));
  let closing: Promise<void> | undefined;
  const close = () => closing ??= (async () => {
    try { await browser.close(); } finally { try { await host?.close(); } finally { await unlock(); } }
  })();
  const signal = () => { void close().finally(() => process.exit(0)); };
  process.once('SIGINT', signal); process.once('SIGTERM', signal);
  let ready!: () => void;
  const external = new Promise<void>(resolve => { ready = resolve; });
  try {
    host = await serve({ ...options, host: '127.0.0.1', port: 0, insecure: true, engineScript: null,
      nativeBrowser: browser, log: line => process.stderr.write(`${line}\n`), externalMcp: { ready,
        capturePanel: async (owner, clip) => (await browser.page(host!.native!.projectForOwner(owner.id))).screenshot({ type: 'jpeg', quality: 85, clip }),
        panelInput: async (owner, args, points) => {
          const page = await browser.page(host!.native!.projectForOwner(owner.id)), first = points[0];
          if (!first) throw new Error('Panel input has no target.');
          await page.mouse.move(first.x, first.y);
          if (args.action === 'scroll') await page.mouse.wheel(0, Number(args.deltaY));
          else if (args.action === 'drag') { await page.mouse.down(); try { for (const point of points.slice(1)) await page.mouse.move(point.x, point.y); } finally { await page.mouse.up(); } }
          else { await page.mouse.click(first.x, first.y); if (args.action === 'type') await page.keyboard.insertText(String(args.text)); if (args.action === 'press') await page.keyboard.press(String(args.key)); }
        },
        // Today's hidden-host import in the project-pinned editor tab: the
        // editor decodes the media and writes its record, poster and placement
        // (import defaults, SVG paths, captions, models) as it always has.
        importMedia: async (_owner, args) => browser.importFile(String(args.projectId), args)
      } });
    await external;
    const client = await connectMcpHost(options.userData);
    await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS, close: async () => { await client.close(); await close(); } });
  } finally { process.off('SIGINT', signal); process.off('SIGTERM', signal); await close(); }
}
