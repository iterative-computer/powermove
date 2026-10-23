import { pinNativeDocument } from './pin-document';
import { importFileInPage } from '../page-import';
import { randomUUID } from 'node:crypto';
import { stat } from 'node:fs/promises';
import path from 'node:path';
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
        await page.goto(`${new URL(this.host().url).origin}${project === 'powermove-home' ? '' : `?project=${encodeURIComponent(project)}`}`, { waitUntil: 'domcontentloaded' });
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
          await page.evaluate(pinNativeDocument, snapshot.project);
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
  /** Agent import_media/replace_media: today's editor import (legacy/assistant/
   *  media-management.ts) in the project's tab, current with the session first.
   *  Its commit reaches the session, and so the document worker, before this answers. */
  async importMedia(project: string, args: Record<string, unknown>, replace: boolean): Promise<Record<string, unknown>> {
    const page = await this.page(project);
    // The tab cannot open host paths: the host hands it the file at args.path
    // (a missing or irregular file is left for the import to name, as a desktop read would).
    const source = typeof args['path'] === 'string' && path.isAbsolute(args['path'])
      ? await stat(args['path']).then(info => info.isFile() ? args['path'] as string : null, () => null) : null;
    const input = `powermove-agent-import-${randomUUID()}`;
    await page.evaluate(id => { const element = document.createElement('input'); element.type = 'file'; element.id = id; element.hidden = true; document.body.appendChild(element); }, input);
    try {
      if (source) await page.locator(`#${input}`).setInputFiles(source);
      return await page.evaluate(async ({ project, args, replace, input }) => {
        const PM = (window as any).PM;
        await PM.RemoteSync.refresh();
        if (PM.proj.id !== project) throw new Error('The DOM tab left its pinned project.');
        const file = (document.getElementById(input) as HTMLInputElement).files?.[0];
        const result = await PM.AgentHarness.importMedia(args, replace, file);
        await PM.RemoteSync.flush();
        return result;
      }, { project, args, replace, input });
    } finally { await page.locator(`#${input}`).evaluate(element => element.remove()).catch(() => {}); }
  }
  /** `import_media_to_timeline` and external `import_media`: today's hidden-host import (../page-import.ts). */
  async importFile(project: string, args: Record<string, unknown>): Promise<unknown> {
    const page = await this.page(project);
    await page.evaluate(async project => {
      const PM = (window as any).PM;
      await PM.RemoteSync.refresh();
      if (PM.proj.id !== project) throw new Error('The DOM tab left its pinned project.');
    }, project);
    const result = await importFileInPage(page, args);
    await page.evaluate(async () => { await (window as any).PM.RemoteSync.flush(); });
    return result;
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
        this.log(`[native] ANGLE runner pinned to ${project}`);
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
