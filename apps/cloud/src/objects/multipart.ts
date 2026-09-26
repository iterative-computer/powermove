import { ApiError } from '@powermove/registry/wire';
import { UPLOAD_ENVELOPE } from '@powermove/registry/limits';

const encoder = new TextEncoder();
const bad = () => new ApiError({ error: 'bad_request', detail: 'malformed multipart body' });
const envelope = () => new ApiError({ error: 'too_large', limit: 'envelope' });
function concat(a: Uint8Array, b: Uint8Array): Uint8Array { const out = new Uint8Array(a.length + b.length); out.set(a); out.set(b, a.length); return out; }
function find(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i <= haystack.length - needle.length; i++) { for (let j = 0; j < needle.length; j++) if (haystack[i+j] !== needle[j]) continue outer; return i; }
  return -1;
}
function joined(parts: Uint8Array[]): Uint8Array { const length = parts.reduce((n, p) => n+p.length, 0); const out = new Uint8Array(length); let at=0; for(const p of parts){out.set(p,at);at+=p.length;} return out; }

export interface MultipartPart { name: string; contentType: string; bytes: Uint8Array }

export async function* parseMultipart(stream: ReadableStream<Uint8Array>, contentType: string): AsyncGenerator<MultipartPart> {
  const match = /^multipart\/form-data\s*;\s*boundary=(?:"([^"]+)"|([^;\s]+))(?:\s*;.*)?$/i.exec(contentType);
  const boundary = match?.[1] ?? match?.[2];
  if (!boundary || boundary.length > 70 || /[\r\n]/.test(boundary)) throw bad();
  const first = encoder.encode(`--${boundary}`), delimiter = encoder.encode(`\r\n--${boundary}`), headerEnd = encoder.encode('\r\n\r\n');
  const reader = stream.getReader();
  let buffer: Uint8Array = new Uint8Array(), eof = false, count = 0, parts = 0;
  async function more() {
    if (eof) return;
    const next = await reader.read();
    eof = !!next.done;
    if (next.value) { count += next.value.length; if (count > UPLOAD_ENVELOPE.compressedBytesPerRequest) throw envelope(); buffer = concat(buffer, next.value); }
  }
  try {
    // A preamble is legal. Keep only enough tail to match a split boundary.
    while (true) {
      const at = find(buffer, first);
      if (at >= 0 && (at === 0 || (at >= 2 && buffer[at-2] === 13 && buffer[at-1] === 10))) { buffer = buffer.slice(at + first.length); break; }
      if (eof) throw bad();
      if (buffer.length > first.length + 2) buffer = buffer.slice(-(first.length + 2));
      await more();
    }
    while (true) {
      while (buffer.length < 2 && !eof) await more();
      if (buffer[0] === 45 && buffer[1] === 45) return; // terminal boundary
      if (buffer[0] !== 13 || buffer[1] !== 10) throw bad();
      buffer = buffer.slice(2);
      let at = find(buffer, headerEnd);
      while (at < 0) { if (eof || buffer.length > 16_384) throw bad(); await more(); at = find(buffer, headerEnd); }
      const headers = new TextDecoder('utf-8', {fatal:true, ignoreBOM:false}).decode(buffer.slice(0,at)).split('\r\n');
      buffer = buffer.slice(at + headerEnd.length);
      const disposition = headers.find(h => /^content-disposition:/i.test(h));
      const name = disposition?.match(/(?:^|;)\s*name="([^"]+)"/i)?.[1];
      const contentTypeHeader = headers.find(h => /^content-type:/i.test(h));
      if (!name || !/^content-disposition:\s*form-data\s*;/i.test(disposition ?? '')) throw bad();
      const contentTypeValue = contentTypeHeader?.split(':',2)[1]?.trim() ?? '';
      if (++parts > UPLOAD_ENVELOPE.partsPerRequest) throw envelope();
      const chunks: Uint8Array[] = [];
      while (true) {
        at = find(buffer, delimiter);
        if (at >= 0) {
          chunks.push(buffer.slice(0,at)); buffer = buffer.slice(at + delimiter.length);
          yield { name, contentType: contentTypeValue, bytes: joined(chunks) };
          break;
        }
        if (eof) throw bad();
        const keep = delimiter.length - 1;
        if (buffer.length > keep) { chunks.push(buffer.slice(0, -keep)); buffer = buffer.slice(-keep); }
        await more();
      }
    }
  } catch (error) {
    if (error instanceof ApiError) throw error;
    throw bad();
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
}
