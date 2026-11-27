/* Request bodies read under a byte cap before anything parses them: a declared Content-Length over the cap is
   refused outright, and a body streamed without one is cut off the moment it passes the cap. */
import type { Context } from 'hono';
import type { Env } from './env';
import { V2Error } from './v2-error';

export const AERO_BODY_BYTES = 8 * 1024 * 1024;
export const WEBHOOK_BODY_BYTES = 512 * 1024;

/** The body as text, at most `max` bytes; 413 past that. */
export async function readLimited(c: Context<Env>, max: number): Promise<string> {
  const declared = c.req.header('content-length');
  if (declared !== undefined && (!/^\d+$/.test(declared) || Number(declared) > max)) throw new V2Error(413, 'too_large');
  const body = c.req.raw.body;
  if (!body) return '';
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) { await reader.cancel().catch(() => {}); throw new V2Error(413, 'too_large'); }
    chunks.push(value);
  }
  const all = new Uint8Array(size);
  let at = 0;
  for (const chunk of chunks) { all.set(chunk, at); at += chunk.byteLength; }
  return new TextDecoder().decode(all);
}

/** The body as JSON under the cap, or null when it isn't JSON. */
export async function readJsonLimited(c: Context<Env>, max: number): Promise<unknown> {
  const text = await readLimited(c, max);
  try { return JSON.parse(text); } catch { return null; }
}
