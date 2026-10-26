/** Standalone CLI runner. Its authenticated loopback service is cheap; Chromium
 * starts only when pm-cli meets a user program its native pipeline cannot
 * translate (`POST /`). */
import { createServer } from 'node:http';
import { randomBytes } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium, type Browser, type Page } from 'playwright';
import { encodeFrame, decodeFrame } from '../../shared/wire';
import type { ShaderFallbackRequest } from '../../shared/engine-protocol';

export async function angleService(distDir: string, env: NodeJS.ProcessEnv) {
  let browser: Browser | null = null, page: Promise<Page> | null = null, closed = false;
  const token = randomBytes(32).toString('hex');
  const open = () => page ??= (async () => {
    try {
      browser = await chromium.launch({ headless: true, channel: 'chromium', ...(env['POWERMOVE_CHROMIUM'] ? { executablePath: env['POWERMOVE_CHROMIUM'] } : {}),
        args: process.platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu'] : [] });
    } catch (error) { throw new Error(`chromium-unavailable: Install the browser with powermove mcp-install-browser. ${String(error)}`); }
    if (closed) { await browser.close(); throw new Error('angle-runner-closed'); }
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.setContent('<!doctype html><title>Powermove ANGLE runner</title>');
    await page.addScriptTag({ type: 'module', content: await readFile(path.join(distDir, 'renderer/engine/native-host.js'), 'utf8') });
    await page.waitForFunction(() => typeof (globalThis as any).runNativeShader === 'function');
    return page;
  })().catch(async error => { await browser?.close().catch(() => {}); browser = null; page = null; throw error; });
  const server = createServer(async (request, response) => {
    const answer = (value: unknown) => { const bytes = encodeFrame(value); response.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': bytes.length, Connection: 'close' }); response.end(bytes); };
    if (request.method !== 'POST' || request.url !== '/' || request.headers.authorization !== `Bearer ${token}`) { response.writeHead(403); response.end(); return; }
    try {
      const chunks: Buffer[] = []; let size = 0;
      for await (const chunk of request) { size += chunk.length; if (size > 1024 ** 3) throw new Error('Shader inputs exceed 1 GB'); chunks.push(chunk); }
      const job = decodeFrame(Buffer.concat(chunks)) as { request: ShaderFallbackRequest; textures: Uint8Array[] };
      const current = await open();
      const result = await current.evaluate(async ({ request, textures }) => (globalThis as any).runNativeShader(request, textures),
        { request: job.request, textures: job.textures.map(t => Buffer.from(t).toString('base64')) });
      answer(result.ok ? { ok: true, pixels: Buffer.from(result.pixels, 'base64') } : result);
    } catch (error) { answer({ ok: false, reason: 'unavailable', message: String(error) }); }
  });
  await new Promise<void>((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const address = server.address(); if (!address || typeof address === 'string') throw new Error('ANGLE service has no address');
  return { env: { POWERMOVE_ANGLE_ENDPOINT: `127.0.0.1:${address.port}`, POWERMOVE_ANGLE_TOKEN: token },
    close: async () => { closed = true; try { await browser?.close(); } finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); } } };
}
