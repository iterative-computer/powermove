import { constants } from 'node:fs';
import { open } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { Readable } from 'node:stream';
import { AGENT_MEDIA_TYPES, agentMediaExtension } from '../shared/agent-media';
import { readExactly } from './durable-file';

/* Serves the local images, videos and audio an agent shows in its reply
   (app://powermove/_agent-file?path=/abs/file.png). Only media extensions are
   served, as inert bytes: nosniff and a sandbox CSP keep an SVG or a
   mislabeled file from ever running as a document. */

const RANGE_CHUNK = 4 * 1024 * 1024;

export function agentMediaFile(requestUrl: URL): string | null {
  const raw = requestUrl.searchParams.get('path');
  if (!raw) return null;
  const expanded = raw.startsWith('~/') ? path.join(os.homedir(), raw.slice(2)) : raw;
  if (!path.isAbsolute(expanded) || expanded.includes('\0') || expanded.split(/[\\/]/).includes('..')) return null;
  return AGENT_MEDIA_TYPES[agentMediaExtension(expanded)] ? path.normalize(expanded) : null;
}

export async function serveAgentMedia(requestUrl: URL, request: Request): Promise<Response> {
  const file = agentMediaFile(requestUrl);
  if (!file) return new Response('Not found', { status: 404 });
  const headers: Record<string, string> = {
    'Content-Type': AGENT_MEDIA_TYPES[agentMediaExtension(file)]!,
    'Cache-Control': 'no-store',
    'Content-Security-Policy': "default-src 'none'; sandbox",
    'X-Content-Type-Options': 'nosniff',
    'Accept-Ranges': 'bytes'
  };
  let handle;
  try { handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW); }
  catch { return new Response('Not found', { status: 404 }); }
  try {
    const info = await handle.stat();
    if (!info.isFile()) { await handle.close(); return new Response('Not found', { status: 404 }); }
    const size = info.size;
    let start = 0, end = size - 1, status = 200;
    const range = request.headers.get('Range');
    if (range) {
      const match = /^bytes=(\d+)-(\d*)$/.exec(range.trim());
      start = Number(match?.[1]);
      end = match?.[2] ? Math.min(Number(match[2]), size - 1) : size - 1;
      if (!match || !Number.isSafeInteger(start) || start >= size || end < start) {
        await handle.close();
        return new Response(null, { status: 416, headers: { ...headers, 'Content-Range': `bytes */${size}` } });
      }
      end = Math.min(end, start + RANGE_CHUNK - 1);
      status = 206; headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
    }
    headers['Content-Length'] = String(Math.max(0, end - start + 1));
    if (request.method === 'HEAD' || !size) { await handle.close(); return new Response(null, { status, headers }); }
    if (range) {
      // Bounded ranges are read whole before Chromium sees them, avoiding
      // its custom-protocol stream cancellation path while seeking video.
      try { return new Response(new Uint8Array(await readExactly(handle, start, end - start + 1)), { status, headers }); }
      finally { await handle.close(); }
    }
    const stream = handle.createReadStream({ start, end, autoClose: true, signal: request.signal, highWaterMark: 64 * 1024 });
    return new Response(Readable.toWeb(stream) as unknown as ReadableStream, { status, headers });
  } catch {
    await handle.close().catch(() => undefined);
    return new Response('Not found', { status: 404 });
  }
}
