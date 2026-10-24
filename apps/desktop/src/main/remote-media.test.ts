import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readdir, readFile, rm, stat } from 'node:fs/promises';
import { createServer, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import { IPC } from '../shared/ipc';
import {
  downloadRemoteMedia, httpsTransport, isPublicAddress, pinAddress, registerRemoteMediaIpc, remoteMediaName, RemoteMediaService,
  REMOTE_MEDIA_CHUNK_BYTES, sniffMedia, type RemoteResponse, type Resolve, type Transport
} from './remote-media';

const PNG = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 1, 2, 3, 4]);
const ascii = (text: string): Uint8Array => Uint8Array.from(text, (char) => char.charCodeAt(0));
const PUBLIC = '93.184.215.14';

const dirs: string[] = [];
async function scratch(): Promise<string> {
  const dir = await mkdtemp(path.join(tmpdir(), 'powermove-remote-media-test-'));
  dirs.push(dir);
  return dir;
}
afterEach(async () => { for (const dir of dirs.splice(0)) await rm(dir, { recursive: true, force: true }); });

/** DNS as a table: each name answers its list, in order. Counts lookups per name. */
function dns(table: Record<string, string[] | string[][]>): Resolve & { calls: string[] } {
  const calls: string[] = [];
  const resolve = (async (hostname: string) => {
    calls.push(hostname);
    const entry = table[hostname];
    if (!entry) throw new Error('ENOTFOUND');
    // A list of lists answers differently on each lookup (a rebinding server).
    const answer = Array.isArray(entry[0]) ? (entry as string[][])[Math.min(calls.filter((name) => name === hostname).length - 1, entry.length - 1)]! : entry as string[];
    return answer.map((address) => ({ address, family: address.includes(':') ? 6 : 4 }));
  }) as unknown as Resolve & { calls: string[] };
  resolve.calls = calls;
  return resolve;
}

type Reply = { status?: number; headers?: Record<string, string>; body?: Uint8Array[] };
/** A server per URL; records every request the guard let through. */
function server(routes: Record<string, Reply | (() => Promise<Reply>)>): Transport & { requests: Array<{ url: string; address: string; family: number }>; cancelled: number } {
  const requests: Array<{ url: string; address: string; family: number }> = [];
  const transport = (async ({ url, address, family }: Parameters<Transport>[0]) => {
    requests.push({ url: url.href, address, family });
    const route = routes[url.href];
    if (!route) return { status: 404, headers: {}, body: (async function* () {})(), cancel: () => {} } satisfies RemoteResponse;
    const reply = typeof route === 'function' ? await route() : route;
    return {
      status: reply.status ?? 200, headers: reply.headers ?? {},
      body: (async function* () { for (const chunk of reply.body ?? []) yield chunk; })(),
      cancel: () => { transport.cancelled += 1; }
    } satisfies RemoteResponse;
  }) as unknown as Transport & { requests: Array<{ url: string; address: string; family: number }>; cancelled: number };
  transport.requests = requests;
  transport.cancelled = 0;
  return transport;
}

describe('public address check', () => {
  it('accepts public unicast only', () => {
    for (const address of ['8.8.8.8', '1.1.1.1', PUBLIC, '100.63.255.255', '100.128.0.0', '172.15.255.255', '172.32.0.0', '2606:4700:4700::1111', '2a00:1450:4001::200e',
      '64:ff9b::808:808']) expect(isPublicAddress(address), address).toBe(true);
  });

  it('refuses loopback, private, link-local, CGNAT, ULA, mapped and every special-purpose range', () => {
    for (const address of [
      '0.0.0.0', '0.1.2.3', '10.0.0.1', '10.255.255.255', '100.64.0.1', '100.127.255.255', '127.0.0.1', '127.255.255.254', '169.254.169.254',
      '172.16.0.1', '172.31.255.255', '192.0.0.8', '192.0.2.1', '192.88.99.1', '192.168.1.1', '198.18.0.1', '198.19.255.255', '198.51.100.7',
      '203.0.113.9', '224.0.0.251', '239.255.255.250', '240.0.0.1', '255.255.255.255',
      '::', '::1', '0:0:0:0:0:0:0:1', 'fe80::1', 'fe80::1%en0', 'fc00::1', 'fd12:3456:789a::1', 'fec0::1', 'ff02::1',
      '::ffff:127.0.0.1', '::ffff:7f00:1', '::ffff:8.8.8.8', '::ffff:0:10.0.0.1', '::127.0.0.1', '64:ff9b::7f00:1', '64:ff9b::a9fe:a9fe', '64:ff9b:1::808:808',
      '100::1', '2001::1', '2001:0:4136:e378:8000:63bf:3fff:fdd2', '2001:2::1', '2001:10::1', '2001:db8::1', '2002:7f00:1::1', '2002:808:808::1', '3fff::1',
      'localhost', 'example.com', '', '1.2.3', '1.2.3.4.5', '256.1.1.1', '01.2.3.4', '::g'
    ]) expect(isPublicAddress(address), address).toBe(false);
  });
});

describe('address pinning', () => {
  it('checks every answer and pins the first, resolving once', async () => {
    const resolve = dns({ 'cdn.example': [PUBLIC, '2606:4700::6810:84e5'] });
    await expect(pinAddress(new URL('https://cdn.example/a.png'), resolve)).resolves.toEqual({ address: PUBLIC, family: 4 });
    expect(resolve.calls).toEqual(['cdn.example']);
  });

  it('refuses a name with any private answer, a failed lookup, and private literals without asking DNS', async () => {
    const resolve = dns({ 'mixed.example': [PUBLIC, '10.0.0.5'], 'none.example': [], 'localhost': ['127.0.0.1'], 'metadata.example': ['169.254.169.254'] });
    for (const url of ['https://mixed.example/', 'https://none.example/', 'https://missing.example/', 'https://localhost/', 'https://metadata.example/',
      'https://127.0.0.1/', 'https://[::1]/', 'https://[::ffff:7f00:1]/', 'https://0x7f.1/', 'https://2130706433/']) {
      await expect(pinAddress(new URL(url), resolve), url).rejects.toThrow();
    }
    expect(resolve.calls).not.toContain('127.0.0.1');
    expect(resolve.calls.filter((name) => /^[\d.[\]:]+$/.test(name))).toEqual([]);
  });

  it('says the same thing for a name that does not resolve and one that resolves privately', async () => {
    const resolve = dns({ 'wiki.corp.example': ['10.1.2.3'], 'empty.corp.example': [] });
    const reasons = await Promise.all(['missing.corp.example', 'wiki.corp.example', 'empty.corp.example'].map((host) =>
      pinAddress(new URL(`https://${host}/a.png`), resolve).then(() => 'resolved', (error: Error) => error.message.replace(host, '<host>'))));
    expect(reasons).toEqual(Array(3).fill('<host> is not reachable on the public internet'));
  });
});

describe('remote media download', () => {
  it('downloads media to the pinned address and names it after what the bytes are', async () => {
    const dir = await scratch();
    const resolve = dns({ 'cdn.example': [PUBLIC] });
    const transport = server({ 'https://cdn.example/photos/Sunset%20Beach.jpeg?w=800': { headers: { 'content-type': 'text/html', 'content-length': String(PNG.byteLength) }, body: [PNG.subarray(0, 5), PNG.subarray(5)] } });
    const result = await downloadRemoteMedia('https://cdn.example/photos/Sunset%20Beach.jpeg?w=800', path.join(dir, 'file'), { resolve, transport });
    // The server's type and the URL's extension do not decide: the signature does.
    expect(result).toEqual({ size: PNG.byteLength, name: 'Sunset Beach.png', type: 'image/png', kind: 'image' });
    expect(new Uint8Array(await readFile(path.join(dir, 'file')))).toEqual(PNG);
    expect(transport.requests).toEqual([{ url: 'https://cdn.example/photos/Sunset%20Beach.jpeg?w=800', address: PUBLIC, family: 4 }]);
  });

  it('connects to the checked address even when DNS answers differently next time (rebinding)', async () => {
    const dir = await scratch();
    const resolve = dns({ 'rebind.example': [[PUBLIC], ['127.0.0.1']] });
    const transport = server({ 'https://rebind.example/a': { body: [PNG] } });
    await downloadRemoteMedia('https://rebind.example/a', path.join(dir, 'file'), { resolve, transport });
    expect(resolve.calls).toEqual(['rebind.example']);
    expect(transport.requests.map((request) => request.address)).toEqual([PUBLIC]);

    // A redirect back to the same name is resolved and checked again.
    const hop = dns({ 'rebind.example': [[PUBLIC], ['10.0.0.8']] });
    const redirected = server({ 'https://rebind.example/a': { status: 302, headers: { location: '/b' } }, 'https://rebind.example/b': { body: [PNG] } });
    await expect(downloadRemoteMedia('https://rebind.example/a', path.join(dir, 'hop'), { resolve: hop, transport: redirected })).rejects.toThrow('not reachable on the public internet');
    expect(hop.calls).toEqual(['rebind.example', 'rebind.example']);
    expect(redirected.requests.map((request) => request.url)).toEqual(['https://rebind.example/a']);
  });

  it('re-checks every redirect, allows five, and refuses the sixth, http, and a private hop', async () => {
    const dir = await scratch();
    const hops = Object.fromEntries(Array.from({ length: 6 }, (_, index) => [`https://cdn.example/${index}`, { status: 302, headers: { location: `/${index + 1}` } }]));
    const resolve = dns({ 'cdn.example': [PUBLIC], 'intranet.example': ['10.1.2.3'] });
    const five = server({ ...hops, 'https://cdn.example/5': { body: [PNG] } });
    await expect(downloadRemoteMedia('https://cdn.example/0', path.join(dir, 'five'), { resolve, transport: five })).resolves.toMatchObject({ kind: 'image' });
    expect(five.requests).toHaveLength(6);
    const six = server({ ...hops, 'https://cdn.example/6': { body: [PNG] } });
    await expect(downloadRemoteMedia('https://cdn.example/0', path.join(dir, 'six'), { resolve, transport: six })).rejects.toThrow('More than 5 redirects');
    expect(six.requests).toHaveLength(6);

    for (const [location, message] of [['http://cdn.example/x', 'https URL'], ['file:///etc/passwd', 'https URL'], ['https://intranet.example/x', 'not reachable on the public internet'],
      ['https://127.0.0.1/x', 'not reachable on the public internet'], ['https://[fd00::1]/x', 'not reachable on the public internet'], ['https://cdn.example:8443/x', 'default https port'], ['https://user:pw@cdn.example/x', 'https URL']] as const) {
      const transport = server({ 'https://cdn.example/start': { status: 301, headers: { location } } });
      await expect(downloadRemoteMedia('https://cdn.example/start', path.join(dir, `hop-${transport.requests.length}-${Math.random()}`), { resolve, transport }), location).rejects.toThrow(message);
      expect(transport.requests).toHaveLength(1);
    }
  });

  it('refuses non-media bytes, compressed bodies, errors, other ports and bad URLs', async () => {
    const dir = await scratch();
    const resolve = dns({ 'cdn.example': [PUBLIC] });
    const cases: Array<[string, Reply | null, string]> = [
      ['https://cdn.example/page.png', { body: [ascii('<!doctype html><html>')] }, 'not an image, video or audio'],
      ['https://cdn.example/icon.svg', { body: [ascii('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>')] }, 'not an image, video or audio'],
      ['https://cdn.example/archive.zip', { body: [ascii('PK\u0003\u0004')] }, 'not an image, video or audio'],
      ['https://cdn.example/empty.png', { body: [] }, 'not an image, video or audio'],
      ['https://cdn.example/gz.png', { headers: { 'content-encoding': 'gzip' }, body: [PNG] }, 'compressed'],
      ['https://cdn.example/missing.png', { status: 404 }, 'answered 404'],
      ['https://cdn.example/auth.png', { status: 401 }, 'answered 401'],
      ['https://cdn.example:8443/a.png', null, 'default https port'],
      ['http://cdn.example/a.png', null, 'https URL'],
      ['https://user:pw@cdn.example/a.png', null, 'https URL']
    ];
    for (const [url, reply, message] of cases) {
      const transport = server(reply ? { [url]: reply } : {});
      const target = path.join(dir, `case-${cases.findIndex((entry) => entry[0] === url)}`);
      await expect(downloadRemoteMedia(url, target, { resolve, transport }), url).rejects.toThrow(message);
      if (!reply) expect(transport.requests).toEqual([]);
    }
  });

  it('stops at the size cap, declared or streamed', async () => {
    const dir = await scratch();
    const resolve = dns({ 'cdn.example': [PUBLIC] });
    let streamed = 0;
    const declared = server({ 'https://cdn.example/big': { headers: { 'content-length': String(1024 * 1024 * 1024) }, body: [PNG] } });
    await expect(downloadRemoteMedia('https://cdn.example/big', path.join(dir, 'declared'), { resolve, transport: declared })).rejects.toThrow('larger than 512 MiB');
    expect(declared.cancelled).toBe(1);
    const endless: Transport = async () => ({ status: 200, headers: {}, cancel: () => {}, body: (async function* () {
      yield PNG;
      for (;;) { streamed += 1; yield new Uint8Array(64); }
    })() });
    await expect(downloadRemoteMedia('https://cdn.example/endless', path.join(dir, 'streamed'), { resolve, transport: endless, maxBytes: 4096 })).rejects.toThrow('larger than 512 MiB');
    expect(streamed).toBeLessThan(70);
  });

  it('gives up on a server that never finishes', async () => {
    const dir = await scratch();
    const stalled: Transport = async ({ signal }) => ({ status: 200, headers: {}, cancel: () => {}, body: (async function* () {
      yield PNG;
      await new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(signal.reason), { once: true }));
    })() });
    await expect(downloadRemoteMedia('https://cdn.example/slow', path.join(dir, 'slow'), { resolve: dns({ 'cdn.example': [PUBLIC] }), transport: stalled, timeoutMs: 30 }))
      .rejects.toThrow('took too long');
  });
});

describe('the https transport', () => {
  it('opens the socket to the pinned address and names the real host for TLS', async () => {
    const hello = new Promise<{ remote: string; bytes: Buffer }>((resolve) => {
      const listener = createServer((socket: Socket) => {
        socket.once('data', (bytes: Buffer) => { resolve({ remote: socket.remoteAddress ?? '', bytes }); socket.destroy(); listener.close(); });
      });
      listener.listen(0, '127.0.0.1', () => {
        const { port } = listener.address() as { port: number };
        // `pinned.invalid` cannot resolve: reaching the listener proves the lookup was pinned.
        void httpsTransport({ url: new URL(`https://pinned.invalid:${port}/a.png`), address: '127.0.0.1', family: 4, signal: new AbortController().signal }).catch(() => undefined);
      });
    });
    const { remote, bytes } = await hello;
    expect(remote).toBe('127.0.0.1');
    expect(bytes[0]).toBe(0x16); // a TLS handshake record
    expect(bytes.includes(Buffer.from('pinned.invalid'))).toBe(true); // SNI carries the host, so the certificate is checked against it
  });
});

describe('media signatures', () => {
  const ftyp = (brand: string) => Uint8Array.from([0, 0, 0, 0x18, ...ascii('ftyp'), ...ascii(brand), 0, 0, 0, 0]);
  it('recognizes the formats Chromium decodes', () => {
    const table: Array<[Uint8Array, string | null]> = [
      [PNG, 'image/png'], [Uint8Array.from([0xff, 0xd8, 0xff, 0xe0]), 'image/jpeg'], [ascii('GIF89a....'), 'image/gif'], [ascii('GIF87a....'), 'image/gif'],
      [Uint8Array.from([...ascii('RIFF'), 0, 0, 0, 0, ...ascii('WEBPVP8 ')]), 'image/webp'], [Uint8Array.from([...ascii('RIFF'), 0, 0, 0, 0, ...ascii('WAVEfmt ')]), 'audio/wav'],
      [Uint8Array.from([...ascii('BM'), 1, 2, 3, 4, 0, 0, 0, 0, 54, 0, 0, 0]), 'image/bmp'],
      [ftyp('avif'), 'image/avif'], [ftyp('isom'), 'video/mp4'], [ftyp('mp42'), 'video/mp4'], [ftyp('qt  '), 'video/quicktime'], [ftyp('M4A '), 'audio/mp4'],
      [Uint8Array.from([0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x82, 0x84, ...ascii('webm')]), 'video/webm'],
      [ascii('OggS\u0000'), 'audio/ogg'], [ascii('fLaC'), 'audio/flac'], [ascii('ID3\u0004'), 'audio/mpeg'], [Uint8Array.from([0xff, 0xfb, 0x90, 0x64]), 'audio/mpeg'],
      [Uint8Array.from([0xff, 0xf1, 0x50, 0x80]), 'audio/aac'],
      // Needs a conversion, or is not media at all.
      [ftyp('heic'), null], [ftyp('3gp4'), null], [Uint8Array.from([0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x82, 0x88, ...ascii('matroska')]), null],
      [Uint8Array.from([...ascii('RIFF'), 0, 0, 0, 0, ...ascii('AVI ')]), null], [ascii('<svg'), null], [ascii('<?xml'), null], [ascii('%PDF-1.7'), null],
      [ascii('BM'), null], [Uint8Array.from([0, 0, 1, 0]), null], [ascii('MZ'), null], [new Uint8Array(), null]
    ];
    for (const [head, type] of table) expect(sniffMedia(head)?.type ?? null, String(type)).toBe(type);
  });

  it('names the file after the URL with the sniffed extension', () => {
    const png = sniffMedia(PNG)!;
    expect(remoteMediaName(new URL('https://a.example/x/Photo%201.JPG?size=large'), png)).toBe('Photo 1.png');
    expect(remoteMediaName(new URL('https://a.example/'), png)).toBe('image.png');
    expect(remoteMediaName(new URL('https://a.example/..%2F..%2Fetc%2Fpasswd'), png)).toBe('passwd.png');
    expect(remoteMediaName(new URL(`https://a.example/${'a'.repeat(300)}.gif`), png)).toBe(`${'a'.repeat(80)}.png`);
  });
});

describe('remote media IPC', () => {
  function fakeIpc() {
    const handlers = new Map<string, (event: IpcMainInvokeEvent, ...args: unknown[]) => unknown>();
    const ipc = { handle: (channel: string, handler: (event: IpcMainInvokeEvent, ...args: unknown[]) => unknown) => handlers.set(channel, handler) } as unknown as IpcMain;
    const senders = new Map<number, EventEmitter>();
    const event = (id: number) => {
      if (!senders.has(id)) senders.set(id, Object.assign(new EventEmitter(), { id, isDestroyed: () => false, isCrashed: () => false }));
      return { sender: senders.get(id) } as unknown as IpcMainInvokeEvent;
    };
    return { ipc, handlers, event, destroy: () => senders.forEach(sender => sender.emit('destroyed')) };
  }

  it('hands the file back in bounded chunks to its own window only, and deletes it on release', async () => {
    const dir = await scratch();
    const service = new RemoteMediaService({ directory: dir, resolve: dns({ 'cdn.example': [PUBLIC] }), transport: server({ 'https://cdn.example/a.png': { body: [PNG] } }) });
    const { ipc, handlers, event } = fakeIpc();
    registerRemoteMediaIpc(ipc, { isTrustedSender: () => true }, service);
    const info = await handlers.get(IPC.remoteMediaFetch)!(event(1), 'https://cdn.example/a.png') as { token: string; size: number };
    expect(info).toMatchObject({ size: PNG.byteLength, name: 'a.png', type: 'image/png', kind: 'image' });
    const read = handlers.get(IPC.remoteMediaRead)!;
    const chunk = await read(event(1), { token: info.token, offset: 8, length: 4 }) as Uint8Array;
    expect(chunk).toEqual(PNG.subarray(8, 12));
    // IPC clones a view's whole ArrayBuffer, so the chunk owns exactly its bytes.
    expect(chunk.buffer.byteLength).toBe(4);
    await expect(read(event(2), { token: info.token, offset: 0, length: 4 })).rejects.toThrow('Unknown download');
    for (const bad of [{ offset: -1, length: 4 }, { offset: 0, length: 0 }, { offset: 0, length: PNG.byteLength + 1 }, { offset: 0, length: REMOTE_MEDIA_CHUNK_BYTES + 1 }, { offset: 0.5, length: 1 }]) {
      await expect(read(event(1), { token: info.token, ...bad })).rejects.toThrow('Invalid download read');
    }
    await handlers.get(IPC.remoteMediaRelease)!(event(2), info.token); // not its window: ignored
    expect((await readdir(dir, { recursive: true })).length).toBe(2);
    await handlers.get(IPC.remoteMediaRelease)!(event(1), info.token);
    await expect(read(event(1), { token: info.token, offset: 0, length: 4 })).rejects.toThrow('Unknown download');
    const [folder] = await readdir(dir);
    expect(await readdir(path.join(dir, folder!))).toEqual([]);
  });

  it('removes a refused download and everything a closed window left behind', async () => {
    const dir = await scratch();
    const service = new RemoteMediaService({ directory: dir, resolve: dns({ 'cdn.example': [PUBLIC] }),
      transport: server({ 'https://cdn.example/a.png': { body: [PNG] }, 'https://cdn.example/page': { body: [ascii('<html>')] } }) });
    const { ipc, handlers, event, destroy } = fakeIpc();
    registerRemoteMediaIpc(ipc, { isTrustedSender: () => true }, service);
    await expect(handlers.get(IPC.remoteMediaFetch)!(event(1), 'https://cdn.example/page')).rejects.toThrow('not an image');
    await handlers.get(IPC.remoteMediaFetch)!(event(1), 'https://cdn.example/a.png');
    const [folder] = await readdir(dir);
    expect(await readdir(path.join(dir, folder!))).toHaveLength(1);
    destroy();
    await vi.waitFor(async () => expect(await readdir(path.join(dir, folder!))).toEqual([]));
    expect((await stat(path.join(dir, folder!))).isDirectory()).toBe(true);
  });

  it('frees its download slot when it cannot even create the file, and tries the folder again next time', async () => {
    const parent = path.join(await scratch(), 'missing', 'deeper');
    const service = new RemoteMediaService({ directory: parent, resolve: dns({ 'cdn.example': [PUBLIC] }), transport: server({ 'https://cdn.example/a.png': { body: [PNG] } }) });
    for (let attempt = 0; attempt < 6; attempt++) await expect(service.fetch(1, 'https://cdn.example/a.png')).rejects.toThrow('ENOENT');
    // A failed mkdtemp is not remembered: once the parent exists, downloads work without a relaunch.
    await mkdir(parent, { recursive: true });
    await expect(service.fetch(1, 'https://cdn.example/a.png')).resolves.toMatchObject({ size: PNG.byteLength });
  });

  it('makes its folder again when the system purged it from the temporary directory', async () => {
    const dir = await scratch();
    const service = new RemoteMediaService({ directory: dir, resolve: dns({ 'cdn.example': [PUBLIC] }), transport: server({ 'https://cdn.example/a.png': { body: [PNG] } }) });
    const first = await service.fetch(1, 'https://cdn.example/a.png');
    await service.release(1, first.token);
    const [folder] = await readdir(dir);
    await rm(path.join(dir, folder!), { recursive: true });
    const second = await service.fetch(1, 'https://cdn.example/a.png');
    expect(await service.read(1, second.token, 0, 8)).toEqual(PNG.subarray(0, 8));
    expect(await readdir(dir)).toHaveLength(1);
  });

  it('refuses untrusted senders before touching the network', async () => {
    const transport = server({});
    const { ipc, handlers, event } = fakeIpc();
    registerRemoteMediaIpc(ipc, { isTrustedSender: () => false }, new RemoteMediaService({ directory: await scratch(), resolve: dns({}), transport }));
    await expect(handlers.get(IPC.remoteMediaFetch)!(event(1), 'https://cdn.example/a.png')).rejects.toThrow('Unauthorized IPC sender');
    expect(transport.requests).toEqual([]);
  });
});
