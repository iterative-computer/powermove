/*
 * Binary-safe message framing for the remote (web) client. The IPC contract
 * carries JSON plus Uint8Array payloads, so a frame is one JSON header with
 * every byte array pulled out into a trailing blob table:
 *
 *   u32 header bytes | header (utf-8 JSON) | (u32 length | bytes)*
 *
 * Byte arrays in the header become `{ "$bin": <index> }`. Nothing else in
 * the contract needs structured-clone semantics, so plain JSON is enough.
 */

const BIN_KEY = '$bin';

export type WireFrame = Uint8Array;

function isBytes(value: unknown): value is Uint8Array {
  return value instanceof Uint8Array || (typeof ArrayBuffer !== 'undefined' && value instanceof ArrayBuffer);
}

function toBytes(value: Uint8Array | ArrayBuffer): Uint8Array {
  return value instanceof Uint8Array ? value : new Uint8Array(value);
}

function extract(value: unknown, blobs: Uint8Array[]): unknown {
  if (isBytes(value)) {
    blobs.push(toBytes(value));
    return { [BIN_KEY]: blobs.length - 1 };
  }
  if (Array.isArray(value)) return value.map((item) => extract(item, blobs));
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      if (item === undefined) continue;
      out[key] = extract(item, blobs);
    }
    return out;
  }
  return value;
}

function restore(value: unknown, blobs: Uint8Array[]): unknown {
  if (Array.isArray(value)) return value.map((item) => restore(item, blobs));
  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record);
    if (keys.length === 1 && keys[0] === BIN_KEY && typeof record[BIN_KEY] === 'number') {
      const blob = blobs[record[BIN_KEY]];
      if (!blob) throw new Error('wire: missing binary attachment');
      return blob;
    }
    const out: Record<string, unknown> = {};
    for (const key of keys) out[key] = restore(record[key], blobs);
    return out;
  }
  return value;
}

export function encodeFrame(message: unknown): WireFrame {
  const blobs: Uint8Array[] = [];
  const header = new TextEncoder().encode(JSON.stringify(extract(message, blobs) ?? null));
  let total = 4 + header.byteLength;
  for (const blob of blobs) total += 4 + blob.byteLength;
  const frame = new Uint8Array(total);
  const view = new DataView(frame.buffer);
  let offset = 0;
  view.setUint32(offset, header.byteLength);
  offset += 4;
  frame.set(header, offset);
  offset += header.byteLength;
  for (const blob of blobs) {
    view.setUint32(offset, blob.byteLength);
    offset += 4;
    frame.set(blob, offset);
    offset += blob.byteLength;
  }
  return frame;
}

export function decodeFrame(frame: Uint8Array): unknown {
  if (frame.byteLength < 4) throw new Error('wire: truncated frame');
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  let offset = 0;
  const headerLength = view.getUint32(offset);
  offset += 4;
  if (offset + headerLength > frame.byteLength) throw new Error('wire: truncated header');
  const header = JSON.parse(new TextDecoder().decode(frame.subarray(offset, offset + headerLength)));
  offset += headerLength;
  const blobs: Uint8Array[] = [];
  while (offset < frame.byteLength) {
    if (offset + 4 > frame.byteLength) throw new Error('wire: truncated blob table');
    const length = view.getUint32(offset);
    offset += 4;
    if (offset + length > frame.byteLength) throw new Error('wire: truncated blob');
    // Copy so the blob outlives the socket buffer it arrived in.
    blobs.push(frame.slice(offset, offset + length));
    offset += length;
  }
  return restore(header, blobs);
}

/* ── message shapes ─────────────────────────────────────── */

/** Browser → server. */
export type ClientMessage =
  | { t: 'invoke'; id: number; ch: string; args: unknown[] }
  | { t: 'send'; ch: string; args: unknown[] }
  /** ipcRenderer.sendSync equivalent: answered from the channel's `on` listener. */
  | { t: 'sync'; id: number; ch: string; args: unknown[] }
  /** Reply to a server `ask`. */
  | { t: 'answer'; id: number; value: unknown };

/** Server → browser. */
export type ServerMessage =
  | { t: 'result'; id: number; ok: true; value: unknown }
  | { t: 'result'; id: number; ok: false; error: string }
  | { t: 'event'; ch: string; args: unknown[] }
  /** Server needs the browser to decide something (a dialog). */
  | { t: 'ask'; id: number; ch: string; args: unknown[] };

export function isClientMessage(value: unknown): value is ClientMessage {
  if (!value || typeof value !== 'object') return false;
  const message = value as Record<string, unknown>;
  switch (message.t) {
    case 'invoke':
    case 'sync':
      return typeof message.id === 'number' && typeof message.ch === 'string' && Array.isArray(message.args);
    case 'send':
      return typeof message.ch === 'string' && Array.isArray(message.args);
    case 'answer':
      return typeof message.id === 'number';
    default:
      return false;
  }
}

/** Channels the server adds on top of the Electron contract for the web client. */
export const WEB = {
  /** Chunked upload of a browser File to server-side temp storage. */
  uploadBegin: 'web:upload-begin',
  uploadChunk: 'web:upload-chunk',
  uploadFinish: 'web:upload-finish',
  uploadAbort: 'web:upload-abort',
  /** Open an uploaded .pmv as this client's project. */
  projectOpenPath: 'web:project-open-path',
  /** Native dialogs the server cannot show, answered by the browser. */
  askMessageBox: 'web:ask:message-box',
  /** shell.openExternal from the server: the browser opens the URL. */
  openExternal: 'web:open-external',
  /** Window management is the browser's: open a tab for a project, raise or close this one. */
  openWindow: 'web:open-window',
  focusWindow: 'web:focus-window',
  closeWindow: 'web:close-window',
  /** Project sessions: tabs share one document through the host. */
  syncJoin: 'web:sync-join',
  syncLeave: 'web:sync-leave',
  syncPatch: 'web:sync-patch',
  /** Agent runs owned by the host: list them for a project, attach to one's stream. */
  runsList: 'web:runs-list',
  runsAttach: 'web:runs-attach',
  runFinished: 'web:run-finished',
  /** Server-side capabilities the browser bridge adapts to. */
  hello: 'web:hello'
} as const;

export interface WebHello {
  node: string;
  version: string;
  platform: string;
  /** Where dialog-chosen saves land on the host, relative to the home dir. */
  exportsDir: string;
}

export const WEB_UPLOAD_CHUNK_BYTES = 1024 * 1024;
