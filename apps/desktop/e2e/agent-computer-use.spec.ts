import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { expect, test, repoRoot } from './helpers/app';

test('agent bridge captures and draws on a real extension canvas with native input', async ({ session }) => {
  await session.openEditor();
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-panel-bridge-'));
  try {
    const bundle = path.join(directory, 'bridge.cjs');
    await build({ entryPoints: [path.join(repoRoot, 'src/main/agent-tools/bridge.ts')], outfile: bundle, bundle: true, platform: 'node', format: 'cjs', external: ['electron'] });
    await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.PANELS['canvas-fixture'] = { id: 'canvas-fixture', title: 'Canvas fixture', size: 260, build(body: HTMLElement) {
        body.innerHTML = '<canvas width="180" height="120" style="width:180px;height:120px;background:#123456"></canvas>';
        const canvas = body.querySelector('canvas')!;
        const events: any[] = []; (window as any).canvasInputEvents = events;
        for (const type of ['pointerdown', 'pointermove', 'pointerup']) canvas.addEventListener(type, (event: any) => {
          events.push({ type, trusted: event.isTrusted, buttons: event.buttons });
          if (type === 'pointerdown') canvas.setPointerCapture(event.pointerId);
          if (event.buttons) { const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#ff0000'; ctx.fillRect(event.offsetX - 3, event.offsetY - 3, 6, 6); }
        });
      } };
      await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'canvas-e2e', callId: 'open', tool: 'open_panel', arguments: { panelId: 'canvas-fixture' }, baseRevision: PM.proj.revision || 0 });
    });
    const connection = await session.app.evaluate(async ({ ipcMain, BrowserWindow }, file) => {
      const { PowermoveAgentToolBridge } = process.getBuiltinModule('module').createRequire(file)(file);
      const owner = BrowserWindow.getAllWindows().find(w => w.webContents.getURL().startsWith('app://powermove'))!.webContents;
      const bridge = new PowermoveAgentToolBridge(ipcMain, { mcpServerPath: '', timeoutMs: 5000 });
      const revision = await owner.executeJavaScript('window.PM.proj.revision || 0');
      const run = await bridge.openSession({ runId: 'native-canvas-e2e', owner, baseRevision: revision });
      (globalThis as any).__panelTestBridge = bridge;
      return { port: Number(run.mcpConfig.env.POWERMOVE_AGENT_TOOL_PORT), token: run.token, runId: run.runId };
    }, bundle);
    const call = (tool: string, args: any = {}) => new Promise<any>((resolve, reject) => {
      const socket = net.createConnection({ host: '127.0.0.1', port: connection.port });
      let data = ''; socket.setEncoding('utf8');
      socket.on('connect', () => socket.write(JSON.stringify({ ...connection, workspace: directory, id: tool, tool, arguments: args }) + '\n'));
      socket.on('data', chunk => data += chunk); socket.on('error', reject);
      socket.on('end', () => { try { resolve(JSON.parse(data)); } catch (e) { reject(e); } });
    });
    const capture = await call('capture_panel', { panelId: 'canvas-fixture' });
    expect(capture.ok, JSON.stringify(capture)).toBe(true);
    expect(capture.content[1].type).toBe('image');
    const metadata = JSON.parse(capture.content[0].text);
    const rect = await session.page.locator('#panel-canvas-fixture canvas').boundingBox();
    const point = (x: number, y: number) => ({ x: rect!.x - metadata.bounds.x + x, y: rect!.y - metadata.bounds.y + y });
    const drawn = await call('computer_use_panel', { panelId: 'canvas-fixture', action: 'drag', points: [point(20,5), point(40,15), point(60,15), point(20,5)] });
    expect(drawn.ok, JSON.stringify(drawn)).toBe(true);
    const events = await session.page.evaluate(() => (window as any).canvasInputEvents);
    expect(events.some((event: any) => event.type === 'pointerdown' && event.trusted)).toBe(true);
    expect(events.some((event: any) => event.type === 'pointermove' && event.trusted && event.buttons === 1)).toBe(true);
    expect(events.some((event: any) => event.type === 'pointerup' && event.trusted)).toBe(true);
    const pixel = await session.page.locator('#panel-canvas-fixture canvas').evaluate((canvas: HTMLCanvasElement) => [...canvas.getContext('2d')!.getImageData(20, 5, 1, 1).data]);
    expect(pixel).toEqual([255, 0, 0, 255]);
    expect((await call('get_workspace_state')).ok).toBe(true);
    const protectedAction = await call('computer_use_panel', { panelId: 'agent', action: 'click', points: [{ x: 1, y: 1 }] });
    expect(protectedAction.ok).toBe(false);
    await session.app.evaluate(async () => { await (globalThis as any).__panelTestBridge.shutdown(); });
  } finally { await rm(directory, { recursive: true, force: true }); }
});
