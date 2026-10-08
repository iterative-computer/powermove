/*
 * `assets.importUrl`: main downloads a remote image, video or audio file for
 * an extension, so the bytes reach the normal asset import without CORS.
 *
 * The request leaves from this computer, so it can reach what the computer can reach.
 * The guard keeps it on the public internet:
 *
 *  - https only (the shared extension-URL rule), the default port only, no
 *    credentials, no cookies, no Authorization, no proxy, no connection reuse;
 *  - every name is resolved here and every answer checked: loopback, private,
 *    link-local, CGNAT, ULA, multicast, documentation, IPv4-mapped and other
 *    special-purpose addresses are refused, and one bad answer refuses all;
 *  - the connection goes to the address that was checked (the lookup is
 *    pinned), so a second DNS answer cannot move it: no DNS rebinding;
 *  - at most 5 redirects, each one https and checked again the same way;
 *  - at most 512 MiB, counted as it streams to a temporary file, never held
 *    in main's heap;
 *  - the bytes are kept only when their signature is an image, video or audio
 *    format Chromium decodes (the renderer then decodes them before import).
 *
 * The renderer reads the file back in 4 MiB chunks and releases it.
 */
import { randomUUID } from 'node:crypto';
import { lookup as dnsLookup } from 'node:dns/promises';
import { access, mkdtemp, open, rm, type FileHandle } from 'node:fs/promises';
import { request as httpsRequest } from 'node:https';
import { isIP, type LookupFunction } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';

import type { IpcMain, IpcMainInvokeEvent, WebContents } from 'electron';

import { parseExtensionUrl } from '../shared/extension-url';
import { IPC, type RemoteMediaInfo } from '../shared/ipc';

export const REMOTE_MEDIA_MAX_BYTES = 512 * 1024 * 1024;
export const REMOTE_MEDIA_MAX_REDIRECTS = 5;
export const REMOTE_MEDIA_CHUNK_BYTES = 4 * 1024 * 1024;
const TIMEOUT_MS = 5 * 60_000;
const IDLE_MS = 30_000;
/** A downloaded file waits this long for the renderer to read it. */
const HOLD_MS = 5 * 60_000;
const MAX_ACTIVE = 4;
const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308]);
const REQUEST_HEADERS = { accept: 'image/*, video/*, audio/*', 'accept-encoding': 'identity', 'user-agent': 'Powermove' } as const;

export class RemoteMediaError extends Error {
  constructor(message: string) { super(message); this.name = 'RemoteMediaError'; }
}

/* ── addresses ─────────────────────────────────────────── */

function ipv4Value(text: string): number | null {
  const parts = text.split('.');
  if (parts.length !== 4 || !parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)) return null;
  return parts.reduce((value, part) => value * 256 + Number(part), 0);
}

/* Special-purpose IPv4 blocks (RFC 6890 and successors). */
const IPV4_BLOCKED: Array<[string, number]> = [
  ['0.0.0.0', 8], ['10.0.0.0', 8], ['100.64.0.0', 10], ['127.0.0.0', 8], ['169.254.0.0', 16], ['172.16.0.0', 12],
  ['192.0.0.0', 24], ['192.0.2.0', 24], ['192.88.99.0', 24], ['192.168.0.0', 16], ['198.18.0.0', 15], ['198.51.100.0', 24],
  ['203.0.113.0', 24], ['224.0.0.0', 4], ['240.0.0.0', 4]
];
const IPV4_RANGES = IPV4_BLOCKED.map(([base, bits]) => ({ base: ipv4Value(base)!, size: 2 ** (32 - bits) }));
function publicIpv4(value: number): boolean {
  return !IPV4_RANGES.some(({ base, size }) => value >= base && value < base + size);
}

/** Eight 16-bit words, from the URL standard's canonical (compressed, hex-only) form. */
function ipv6Words(text: string): number[] | null {
  if (text.includes('%')) return null; // a zone id names a local interface
  let canonical: string;
  try { canonical = new URL(`http://[${text}]/`).hostname.slice(1, -1); } catch { return null; }
  const [head, tail] = canonical.includes('::') ? canonical.split('::') as [string, string] : [canonical, null];
  const left = head ? head.split(':') : [];
  const right = tail ? tail.split(':') : [];
  const words = [...left, ...Array(8 - left.length - right.length).fill('0'), ...right].map((word) => parseInt(word, 16));
  return words.length === 8 && words.every((word) => Number.isInteger(word) && word >= 0 && word <= 0xffff) ? words : null;
}

/** True only for a public unicast address: the only kind an extension's download may connect to. */
export function isPublicAddress(address: string): boolean {
  const kind = isIP(address);
  if (kind === 4) return publicIpv4(ipv4Value(address)!);
  if (kind !== 6) return false;
  const words = ipv6Words(address);
  if (!words) return false;
  const [a, b] = words as [number, number, ...number[]];
  // NAT64 (64:ff9b::/96) reaches the IPv4 address in its low 32 bits.
  if (a === 0x64 && b === 0xff9b && words.slice(2, 6).every((word) => word === 0)) return publicIpv4(words[6]! * 0x10000 + words[7]!);
  if ((a & 0xe000) !== 0x2000) return false; // only global unicast (2000::/3): no ::1, ::ffff:0:0/96, fc00::/7, fe80::/10, ff00::/8, …
  if (a === 0x2001 && b < 0x200) return false; // 2001::/23 protocol assignments (Teredo, benchmarking, ORCHID)
  if (a === 0x2001 && b === 0xdb8) return false; // documentation
  if (a === 0x2002) return false; // 6to4 carries an IPv4 address
  if (a === 0x3fff && b < 0x1000) return false; // 3fff::/20 documentation
  return true;
}

export type Resolve = (hostname: string) => Promise<ReadonlyArray<{ address: string; family: number }>>;
export const systemResolve: Resolve = (hostname) => dnsLookup(hostname, { all: true, verbatim: true });

/** Resolves once and checks every answer; the caller connects to the returned address and nothing else. */
export async function pinAddress(url: URL, resolve: Resolve): Promise<{ address: string; family: 4 | 6 }> {
  const host = url.hostname.replace(/^\[(.*)\]$/, '$1');
  const literal = isIP(host);
  // One message for a name that does not resolve and one that resolves privately: telling them apart would map the intranet's DNS for the extension.
  const unreachable = (): RemoteMediaError => new RemoteMediaError(`${url.hostname} is not reachable on the public internet`);
  let answers: ReadonlyArray<{ address: string; family: number }>;
  if (literal) answers = [{ address: host, family: literal }];
  else {
    try { answers = await resolve(host); } catch { throw unreachable(); }
  }
  if (!answers.length || answers.some(({ address }) => !isPublicAddress(address))) throw unreachable();
  const { address } = answers[0]!;
  return { address, family: isIP(address) as 4 | 6 };
}

/* ── transport ─────────────────────────────────────────── */

export interface RemoteResponse {
  status: number;
  headers: Record<string, string | string[] | undefined>;
  body: AsyncIterable<Uint8Array>;
  /** Stop reading and close the connection. */
  cancel(): void;
}
/** One GET to `url`'s host over a socket to `address`, which the guard already checked. */
export type Transport = (request: { url: URL; address: string; family: 4 | 6; signal: AbortSignal }) => Promise<RemoteResponse>;

export const httpsTransport: Transport = ({ url, address, family, signal }) => new Promise((resolve, reject) => {
  const host = url.hostname.replace(/^\[(.*)\]$/, '$1');
  // Node asks for every address when it may race families; either way it gets only the pinned one.
  const lookup: LookupFunction = (_hostname, options, callback) => {
    if (options.all) callback(null, [{ address, family }]);
    else callback(null, address, family);
  };
  const request = httpsRequest({
    // downloadRemoteMedia refuses any port but the default; only tests pass another.
    host, port: Number(url.port) || 443, path: `${url.pathname}${url.search}`, method: 'GET', headers: REQUEST_HEADERS,
    ...(isIP(host) ? {} : { servername: host }),
    agent: false, lookup, signal, timeout: IDLE_MS
  }, (response) => resolve({ status: response.statusCode ?? 0, headers: response.headers, body: response, cancel: () => response.destroy() }));
  request.on('timeout', () => request.destroy(new RemoteMediaError('The server stopped responding')));
  request.on('error', reject);
  request.end();
});

/* ── media signatures ──────────────────────────────────── */

export type RemoteMediaKind = RemoteMediaInfo['kind'];
export interface SniffedMedia { kind: RemoteMediaKind; type: string; extension: string }
const HEAD_BYTES = 64;
const ascii = (bytes: Uint8Array, start: number, end: number): string => String.fromCharCode(...bytes.subarray(start, end));

/** The format a file's first bytes declare, among those Chromium decodes itself; null for anything else. */
export function sniffMedia(head: Uint8Array): SniffedMedia | null {
  const at = (index: number): number => head[index] ?? -1;
  if ([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a].every((byte, index) => at(index) === byte)) return { kind: 'image', type: 'image/png', extension: 'png' };
  if (at(0) === 0xff && at(1) === 0xd8 && at(2) === 0xff) return { kind: 'image', type: 'image/jpeg', extension: 'jpg' };
  if (ascii(head, 0, 6) === 'GIF87a' || ascii(head, 0, 6) === 'GIF89a') return { kind: 'image', type: 'image/gif', extension: 'gif' };
  if (ascii(head, 0, 2) === 'BM' && head.length >= 14 && [6, 7, 8, 9].every((index) => at(index) === 0)) return { kind: 'image', type: 'image/bmp', extension: 'bmp' };
  if (ascii(head, 0, 4) === 'RIFF') {
    const form = ascii(head, 8, 12);
    if (form === 'WEBP') return { kind: 'image', type: 'image/webp', extension: 'webp' };
    if (form === 'WAVE') return { kind: 'audio', type: 'audio/wav', extension: 'wav' };
    return null;
  }
  if (ascii(head, 4, 8) === 'ftyp') {
    const brand = ascii(head, 8, 12);
    if (brand === 'avif' || brand === 'avis') return { kind: 'image', type: 'image/avif', extension: 'avif' };
    if (brand === 'M4A ' || brand === 'M4B ') return { kind: 'audio', type: 'audio/mp4', extension: 'm4a' };
    if (brand === 'qt  ') return { kind: 'video', type: 'video/quicktime', extension: 'mov' };
    if (/^(isom|iso[2-9]|mp41|mp42|avc1|dash|M4V |M4VH|M4VP|f4v )$/.test(brand)) return { kind: 'video', type: 'video/mp4', extension: 'mp4' };
    return null; // HEIF, 3GP and the rest need a conversion
  }
  if (at(0) === 0x1a && at(1) === 0x45 && at(2) === 0xdf && at(3) === 0xa3) {
    return ascii(head, 4, head.length).includes('webm') ? { kind: 'video', type: 'video/webm', extension: 'webm' } : null;
  }
  if (ascii(head, 0, 4) === 'OggS') return { kind: 'audio', type: 'audio/ogg', extension: 'ogg' };
  if (ascii(head, 0, 4) === 'fLaC') return { kind: 'audio', type: 'audio/flac', extension: 'flac' };
  if (ascii(head, 0, 3) === 'ID3') return { kind: 'audio', type: 'audio/mpeg', extension: 'mp3' };
  if (at(0) === 0xff && (at(1) & 0xf6) === 0xf0) return { kind: 'audio', type: 'audio/aac', extension: 'aac' };
  if (at(0) === 0xff && (at(1) & 0xe0) === 0xe0 && (at(1) & 0x06) !== 0) return { kind: 'audio', type: 'audio/mpeg', extension: 'mp3' };
  return null;
}

/** The URL's last path segment as a file name, with the extension of what the bytes actually are. */
export function remoteMediaName(url: URL, media: SniffedMedia): string {
  let segment = url.pathname.split('/').filter(Boolean).pop() ?? '';
  try { segment = decodeURIComponent(segment); } catch { /* keep it encoded */ }
  segment = segment.split(/[\\/]/).pop() ?? '';
  const stem = segment.replace(/\.[^.]*$/, '').replace(/[^\p{L}\p{N} _()-]+/gu, ' ').replace(/\s+/g, ' ').trim().slice(0, 80);
  return `${stem || media.kind}.${media.extension}`;
}

/* ── download ──────────────────────────────────────────── */

export interface RemoteMediaNetwork { resolve: Resolve; transport: Transport }
export interface DownloadOptions extends Partial<RemoteMediaNetwork> { maxBytes?: number; timeoutMs?: number }
export type DownloadedMedia = Omit<RemoteMediaInfo, 'token'>;

const single = (value: string | string[] | undefined): string | undefined => (Array.isArray(value) ? value[0] : value);

/** Downloads `value` into `destination` (created, never overwritten). The caller removes the file on failure. */
export async function downloadRemoteMedia(value: unknown, destination: string, options: DownloadOptions = {}): Promise<DownloadedMedia> {
  const resolve = options.resolve ?? systemResolve;
  const transport = options.transport ?? httpsTransport;
  const maxBytes = options.maxBytes ?? REMOTE_MEDIA_MAX_BYTES;
  let url = parseExtensionUrl(value);
  if (!url) throw new RemoteMediaError('assets.importUrl accepts an https URL of at most 2 KB without credentials');
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new RemoteMediaError('The download took too long')), options.timeoutMs ?? TIMEOUT_MS);
  try {
    for (let redirects = 0; ; redirects++) {
      if (url.port) throw new RemoteMediaError('assets.importUrl connects only to the default https port');
      const pinned = await pinAddress(url, resolve);
      controller.signal.throwIfAborted();
      const response = await transport({ url, ...pinned, signal: controller.signal });
      if (REDIRECT_STATUS.has(response.status)) {
        response.cancel();
        if (redirects >= REMOTE_MEDIA_MAX_REDIRECTS) throw new RemoteMediaError(`More than ${REMOTE_MEDIA_MAX_REDIRECTS} redirects`);
        const location = single(response.headers.location);
        let next: URL | null = null;
        try { next = location ? parseExtensionUrl(new URL(location, url).href) : null; } catch { next = null; }
        if (!next) throw new RemoteMediaError('The server redirected somewhere other than an https URL');
        url = next;
        continue;
      }
      if (response.status !== 200) { response.cancel(); throw new RemoteMediaError(`The server answered ${response.status}`); }
      const encoding = single(response.headers['content-encoding']);
      if (encoding && encoding.toLowerCase() !== 'identity') { response.cancel(); throw new RemoteMediaError('The server sent compressed data'); }
      const declared = Number(single(response.headers['content-length']));
      if (Number.isFinite(declared) && declared > maxBytes) { response.cancel(); throw new RemoteMediaError('The file is larger than 512 MiB'); }
      return await save(response, url, destination, maxBytes, controller.signal);
    }
  } catch (error) {
    throw controller.signal.aborted && controller.signal.reason instanceof RemoteMediaError ? controller.signal.reason : error;
  } finally { clearTimeout(timer); }
}

async function save(response: RemoteResponse, url: URL, destination: string, maxBytes: number, signal: AbortSignal): Promise<DownloadedMedia> {
  const file = await open(destination, 'wx', 0o600);
  let size = 0;
  const head = new Uint8Array(HEAD_BYTES);
  let headLength = 0;
  try {
    for await (const chunk of response.body) {
      signal.throwIfAborted();
      size += chunk.byteLength;
      if (size > maxBytes) { response.cancel(); throw new RemoteMediaError('The file is larger than 512 MiB'); }
      if (headLength < HEAD_BYTES) {
        const take = chunk.subarray(0, HEAD_BYTES - headLength);
        head.set(take, headLength); headLength += take.byteLength;
      }
      await file.write(chunk);
    }
  } finally { await file.close(); }
  const media = sniffMedia(head.subarray(0, headLength));
  if (!media) throw new RemoteMediaError('The URL is not an image, video or audio file Powermove can import');
  return { size, name: remoteMediaName(url, media), type: media.type, kind: media.kind };
}

/* ── service and IPC ───────────────────────────────────── */

type Held = { owner: number; path: string; size: number; handle: FileHandle | null; timer: ReturnType<typeof setTimeout> };

/** Downloads per window, held on disk until the window reads and releases them. */
export class RemoteMediaService {
  private held = new Map<string, Held>();
  private active = 0;
  private directory: Promise<string> | null = null;
  constructor(private options: DownloadOptions & { directory?: string } = {}) {}

  /** e2e only (installed on globalThis by main when background testing): swap DNS and the socket for fakes. */
  useNetworkForTests(network: Partial<RemoteMediaNetwork>): void { this.options = { ...this.options, ...network }; }

  async fetch(owner: number, url: unknown): Promise<RemoteMediaInfo> {
    if (this.active >= MAX_ACTIVE) throw new RemoteMediaError('Other imports are still downloading. Try again when they finish.');
    this.active += 1;
    const token = randomUUID();
    let target: string | null = null;
    try {
      target = path.join(await this.dir(), token);
      const media = await downloadRemoteMedia(url, target, this.options);
      const timer = setTimeout(() => void this.release(owner, token), HOLD_MS);
      timer.unref?.();
      this.held.set(token, { owner, path: target, size: media.size, handle: null, timer });
      return { token, ...media };
    } catch (error) {
      if (target) await rm(target, { force: true });
      throw error;
    } finally { this.active -= 1; }
  }

  async read(owner: number, token: unknown, offset: unknown, length: unknown): Promise<Uint8Array> {
    const entry = typeof token === 'string' ? this.held.get(token) : undefined;
    if (!entry || entry.owner !== owner) throw new RemoteMediaError('Unknown download');
    if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || (offset as number) < 0 || (length as number) < 1 ||
      (length as number) > REMOTE_MEDIA_CHUNK_BYTES || (offset as number) + (length as number) > entry.size) throw new RemoteMediaError('Invalid download read');
    entry.timer.refresh();
    entry.handle ??= await open(entry.path, 'r');
    // Its own ArrayBuffer: IPC clones a view's whole backing store, so never hand back a slice of a pooled one.
    const bytes = new Uint8Array(length as number);
    const { bytesRead } = await entry.handle.read(bytes, 0, bytes.byteLength, offset as number);
    return bytesRead === bytes.byteLength ? bytes : bytes.slice(0, bytesRead);
  }

  async release(owner: number, token: unknown): Promise<void> {
    const entry = typeof token === 'string' ? this.held.get(token) : undefined;
    if (!entry || entry.owner !== owner) return;
    this.held.delete(token as string);
    clearTimeout(entry.timer);
    await entry.handle?.close().catch(() => undefined);
    await rm(entry.path, { force: true });
  }

  async releaseOwner(owner: number): Promise<void> {
    await Promise.all([...this.held].filter(([, entry]) => entry.owner === owner).map(([token]) => this.release(owner, token)));
  }

  /* Made once, but never trusted forever: a failed mkdtemp is not cached, and
     a folder macOS purged from $TMPDIR since is made again. */
  private async dir(): Promise<string> {
    for (let attempt = 0; ; attempt++) {
      const pending = this.directory ??= mkdtemp(path.join(this.options.directory ?? tmpdir(), 'powermove-remote-media-'));
      try {
        const directory = await pending;
        await access(directory);
        return directory;
      } catch (error) {
        if (this.directory === pending) this.directory = null;
        if (attempt > 0 || (error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      }
    }
  }
}

export function registerRemoteMediaIpc(ipc: Pick<IpcMain, 'handle'>, ctx: { isTrustedSender(event: IpcMainInvokeEvent): boolean }, service = new RemoteMediaService()): RemoteMediaService {
  const watched = new WeakSet<WebContents>();
  const owner = (event: IpcMainInvokeEvent): number => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const sender = event.sender;
    if (!watched.has(sender)) {
      watched.add(sender);
      sender.once('destroyed', () => void service.releaseOwner(sender.id));
    }
    return sender.id;
  };
  ipc.handle(IPC.remoteMediaFetch, async (event, url: unknown) => service.fetch(owner(event), url));
  ipc.handle(IPC.remoteMediaRead, async (event, input: unknown) => {
    const request = (input ?? {}) as { token?: unknown; offset?: unknown; length?: unknown };
    return service.read(owner(event), request.token, request.offset, request.length);
  });
  ipc.handle(IPC.remoteMediaRelease, async (event, token: unknown) => service.release(owner(event), token));
  return service;
}
