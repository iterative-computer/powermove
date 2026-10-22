/** Independent Powermove implementation; see docs/VISUAL_REPLIES.md. */
export const VISUAL_REPLY_MAX_BYTES = 384 * 1024;
export const VISUAL_REPLY_MAX_PER_RUN = 4;
export const VISUAL_REPLY_PATH = '/visual-reply.html';
export const VISUAL_REPLY_CSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; media-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
export interface VisualReply {
  version: 1;
  id: string;
  runId: string;
  title: string;
  html: string;
  height: number;
}
export function visualReplyInput(value: unknown): { title: string; html: string; height: number } {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Visual reply expects title, html and optional height.');
  const v = value as Record<string, unknown>;
  if (Object.keys(v).some(k => !['title', 'html', 'height'].includes(k)) || typeof v.title !== 'string' || !v.title.trim() || v.title.length > 120
    || typeof v.html !== 'string' || !v.html.trim() || new TextEncoder().encode(v.html).length > VISUAL_REPLY_MAX_BYTES
    || (v.height !== undefined && (!Number.isInteger(v.height) || (v.height as number) < 160 || (v.height as number) > 900))) {
    throw new Error('Visual reply requires a title (1–120 characters), HTML up to 384 KiB and height from 160 to 900.');
  }
  return { title: v.title.trim(), html: v.html, height: v.height as number ?? 360 };
}
export function readVisualReply(value: unknown): VisualReply | null {
  if (!value || typeof value !== 'object') return null;
  const v = value as VisualReply;
  if (v.version !== 1 || typeof v.id !== 'string' || !/^visual-[A-Za-z0-9_-]{1,120}$/.test(v.id)
    || typeof v.runId !== 'string' || v.runId.length > 160) return null;
  try { visualReplyInput({ title: v.title, html: v.html, height: v.height }); return v; } catch { return null; }
}
/** Every reply has an opaque sandbox origin. No tool, message, link or file bridge. */
export function visualReplyEvent(event: Pick<MessageEvent, 'origin' | 'source' | 'data'>, source: Window | null, token: string): { type: 'ready' | 'size' | 'error'; height?: number; message?: string } | null {
  if (!source || event.source !== source || event.origin !== 'null') return null;
  const d = event.data;
  if (!d || typeof d !== 'object' || Array.isArray(d) || d.channel !== 'powermove-visual' || d.token !== token) return null;
  const allowed = ['channel', 'token', 'type', ...(d.type === 'size' ? ['height'] : d.type === 'error' ? ['message'] : [])];
  if (Object.keys(d).some(key => !allowed.includes(key))) return null;
  if (d.type === 'ready') return { type: 'ready' };
  if (d.type === 'size' && Number.isFinite(d.height) && d.height >= 0 && d.height <= 100000) return { type: 'size', height: Math.max(160, Math.min(900, Math.ceil(d.height))) };
  if (d.type === 'error' && typeof d.message === 'string' && d.message.length <= 300) return { type: 'error', message: d.message };
  return null;
}
/** Identify by the committed frame URL, never window.name (page scripts can change it). */
export function isVisualReplyUrl(value: string): boolean {
  try { const u = new URL(value); return u.pathname === VISUAL_REPLY_PATH && (u.protocol === 'app:' && u.host === 'powermove' || u.protocol === 'http:' && ['localhost','127.0.0.1'].includes(u.hostname)); } catch { return false; }
}
