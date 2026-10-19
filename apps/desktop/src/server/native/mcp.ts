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
        importMedia: async (_owner, args, call) => {
          const unpack = (response: import('../../shared/ipc').AgentToolResponseEvent): any => {
            if (!response.ok) throw new Error(response.error ?? 'Media import failed.');
            return JSON.parse((response.content.find(c => c.type === 'text') as { text: string }).text);
          };
          const imported = unpack(await call('import_media', Object.fromEntries(['path', 'folderId', 'expectedFingerprint', 'storage'].filter(k => args[k] !== undefined).map(k => [k, args[k]]))));
          if (args.assetOnly) return imported;
          const snapshot = await host!.native!.snapshot(String(args.projectId));
          const asset = (snapshot.project as any).assets[imported.assetId];
          const at = Number(args.at ?? 0), duration = asset.dur || Math.max(1 / snapshot.project.fps, snapshot.project.dur - at);
          const content = asset.kind === 'audio' ? { asset: asset.id, trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 }
            : { asset: asset.id, w: asset.w || snapshot.project.w, h: asset.h || snapshot.project.h,
              ...(asset.kind === 'video' ? { trim: 0, speed: 1, embeddedAudio: asset.hasAudio === true } : {}) };
          const command = { type: 'add_layer', layerType: asset.kind, name: asset.name, from: Math.round(at * snapshot.project.fps) / snapshot.project.fps, duration, content, select: true };
          unpack(await call('apply_commands', { commands: [JSON.stringify(command)] }));
          const next = await host!.native!.snapshot(String(args.projectId));
          const layer = (next.project as any).layers.find((layer: any) => !(snapshot.project as any).layers.some((old: any) => old.id === layer.id));
          return { assets: [{ id: asset.id, name: asset.name, kind: asset.kind }], layers: layer ? [{ id: layer.id, name: layer.name, type: layer.type }] : [] };
        }
      } });
    await external;
    const client = await connectMcpHost(options.userData);
    await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS, close: async () => { await client.close(); await close(); } });
  } finally { process.off('SIGINT', signal); process.off('SIGTERM', signal); await close(); }
}
