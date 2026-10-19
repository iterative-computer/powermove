import { chromium, type Browser, type BrowserContext, type Page } from 'playwright';
import type { AgentToolRequestEvent, AgentToolResponseEvent } from '../../shared/ipc';
import type { ShaderFallbackRequest } from '../../shared/engine-protocol';
import type { ShaderFallbackResult } from '../../main/engine/shader-fallback-relay';
import type { RunningServer } from '../index';
import type { NativeBrowser } from './host';

/** No Chromium process exists until a DOM, raster or ANGLE request needs one. */
export class LazyNativeBrowser implements NativeBrowser {
  private browser: Browser | null = null;
  private context: Promise<BrowserContext> | null = null;
  private pages = new Map<string, Promise<Page>>();
  private runners = new Map<string, Promise<Page>>();
  constructor(private readonly host: () => RunningServer, private readonly log: (line: string) => void) {}
  private start(): Promise<BrowserContext> {
    if (!this.context) this.context = (async () => {
      try {
        this.browser = await chromium.launch({ headless: true, channel: 'chromium',
          ...(process.env['POWERMOVE_CHROMIUM'] ? { executablePath: process.env['POWERMOVE_CHROMIUM'] } : {}),
          args: process.platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu'] : [] });
        this.browser.on('disconnected', () => { this.browser = null; this.context = null; this.pages.clear(); this.runners.clear(); });
        const context = await this.browser.newContext({ viewport: { width: 1440, height: 1000 }, acceptDownloads: false });
        const host = this.host();
        await context.addCookies([{ name: 'pm_session', value: host.token, url: new URL(host.url).origin }]);
        this.log('[native] Chromium started on demand');
        return context;
      } catch (error) { await this.browser?.close().catch(() => {}); this.browser = null; this.context = null; throw new Error(`chromium-unavailable: Install the browser with powermove mcp-install-browser. ${String(error)}`); }
    })();
    return this.context;
  }
  async page(project: string): Promise<Page> {
    let opening = this.pages.get(project);
    if (!opening) {
      opening = (async () => {
        const page = await (await this.start()).newPage();
        page.on('dialog', dialog => { void dialog.dismiss(); });
        page.on('console', event => { if (event.type() === 'error') this.log(`[native DOM] ${event.text()}`); });
        await page.goto(new URL(this.host().url).origin, { waitUntil: 'domcontentloaded' });
        await page.waitForFunction(() => !!(window as any).PM?.AgentHarness?.dispatch && !!(window as any).PM?.openProjectHere && !!(window as any).PM?.RemoteSync && !!(window as any).PM?.Kernel?.loader, { timeout: 60_000 });
        // The dispatcher is installed synchronously; extensions and the initial
        // document boot are asynchronous and can still replace the project.
        await page.evaluate(async () => {
          const PM = (window as any).PM;
          await PM.EngineDoc?.whenReady;
          await PM.Kernel.loader.boot(); await PM.Kernel.loader.whenIdle();
        });
        if (project !== 'powermove-home') {
          const snapshot = await this.host().native!.snapshot(project);
          await page.evaluate(async doc => {
            const PM = (window as any).PM;
            PM.store.set(`project.${doc.id}`, { v: PM.version, proj: doc });
            if (!await PM.openProjectHere(doc.id)) throw new Error('The pinned project could not be opened.');
            await PM.EngineDoc?.whenReady;
          }, snapshot.project);
        }
        await page.evaluate(async () => { await (window as any).PM.RemoteSync.flush(); });
        this.log(`[native] DOM tab pinned to ${project}`);
        return page;
      })().catch(error => { this.pages.delete(project); throw error; });
      this.pages.set(project, opening);
    }
    return opening;
  }
  async call(project: string, request: AgentToolRequestEvent): Promise<AgentToolResponseEvent> {
    const page = await this.page(project);
    return page.evaluate(async ({ project, request }) => {
      const PM = (window as any).PM;
      if (project !== 'powermove-home' && PM.proj.id !== project) throw new Error('The DOM tab left its pinned project.');
      const result = await PM.AgentHarness.dispatch(request);
      await PM.RemoteSync.flush();
      return result;
    }, { project, request });
  }
  async compat(project: string, request: import('../../shared/engine-protocol').CompatFrameRequest): Promise<import('../../shared/engine-ipc').EngineCompatAnswer> {
    const page = await this.page(project);
    const result = await page.evaluate(async request => {
      const PM = (window as any).PM;
      if (PM.proj.id !== request.document || Number(PM.proj.revision) !== request.revision) throw new Error('staleRevision: The compatibility tab does not have the export snapshot.');
      const answer = await PM.EngineRender.compatibility(request);
      return answer.status === 'pixels' ? { ...answer, data: Array.from(answer.data) } : answer;
    }, request);
    return result.status === 'pixels' ? { ...result, data: new Uint8Array(result.data as number[]) } : result;
  }
  async shader(session: number, request: ShaderFallbackRequest, textures: Uint16Array[]): Promise<ShaderFallbackResult> {
    const project = this.host().native!.projectForSession(session);
    let runner = this.runners.get(project);
    if (!runner) {
      runner = (async () => {
        const page = await (await this.start()).newPage();
        await page.goto(`${new URL(this.host().url).origin}/engine/native-runner.html?project=${encodeURIComponent(project)}`);
        return page;
      })().catch(error => { this.runners.delete(project); throw error; });
      this.runners.set(project, runner);
    }
    const page = await runner;
    const result = await page.evaluate(async ({ request, textures }) => {
      const modulePath = '/engine/native-host.js';
      const runner = await import(/* @vite-ignore */ modulePath);
      return runner.runShader(request, textures);
    }, { request, textures: textures.map(p => Buffer.from(p.buffer, p.byteOffset, p.byteLength).toString('base64')) });
    if (!result.ok) return result;
    const bytes = Buffer.from(result.pixels, 'base64');
    return { ok: true, rgba16f: new Uint16Array(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)) };
  }
  async close(): Promise<void> { await this.browser?.close(); this.browser = null; this.context = null; this.pages.clear(); this.runners.clear(); }
}
