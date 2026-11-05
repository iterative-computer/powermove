import { it, expect, afterEach, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { request as httpRequest } from 'node:http';
import { FigmaLink } from './figma-link';
import { MAX_PAYLOAD } from '../shared/figma/doc';
const links: FigmaLink[] = [],
  dirs: string[] = [];
afterEach(async () => {
  await Promise.all(links.splice(0).map((l) => l.close()));
  await Promise.all(dirs.splice(0).map((d) => rm(d, { recursive: true, force: true })));
});
async function harness(options: ConstructorParameters<typeof FigmaLink>[1] = {}) {
  const received: any[] = [];
  const link = new FigmaLink(async (doc) => {
    received.push(doc);
    return { layers: 1, flattened: 0 };
  }, options);
  links.push(link);
  const { port } = await link.start(0),
    url = `http://localhost:${port}`;
  const post = (route: string, body: any = {}, token?: string, extra: Record<string, string> = {}) =>
    fetch(url + route, {
      method: 'POST',
      headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}), ...extra },
      body: JSON.stringify(body),
    });
  const pair = async () => {
    link.openSetup('sheet');
    const response = post('/pair', { code: '123456' });
    await vi.waitFor(() => expect(link.state.pending?.code).toBe('123456'));
    await link.approve('sheet', link.state.pending!.id, true);
    const r = await response;
    expect(r.status).toBe(200);
    return ((await r.json()) as any).token as string;
  };
  return { link, post, pair, port, received };
}
const doc = {
  format: 'powermove-figma',
  version: 1,
  source: 'plugin',
  nodes: [
    {
      id: '1',
      fileKey: 'synthetic',
      type: 'RECTANGLE',
      name: 'Card',
      w: 100,
      h: 60,
      absoluteTransform: [1, 0, 0, 1, 0, 0],
      fills: [],
      strokes: [],
      effects: [],
      children: [],
    },
  ],
  images: {},
};
it('allows only an open app sheet to approve, and rejects concurrent or closed requests', async () => {
  const h = await harness();
  expect((await h.post('/pair', { code: '123456' })).status).toBe(403);
  h.link.openSetup('sheet');
  const response = h.post('/pair', { code: '123456' });
  await vi.waitFor(() => expect(h.link.state.pending).not.toBeNull());
  expect((await h.post('/pair', { code: '654321' })).status).toBe(409);
  await expect(h.link.approve('other', h.link.state.pending!.id, true)).rejects.toThrow();
  h.link.closeSetup('sheet');
  expect((await response).status).toBe(403);
  expect(h.link.state.pending).toBeNull();
});
it('persists only a hash, validates it after a restart, and revokes it on disconnect', async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-figma-token-test-'));
  dirs.push(dir);
  const storagePath = path.join(dir, 'figma-link.json');
  const h = await harness({ storagePath });
  const token = await h.pair();
  const stored = await readFile(storagePath, 'utf8');
  expect(stored).not.toContain(token);
  expect(JSON.parse(stored)).toEqual({ version: 1, tokenHashes: [createHash('sha256').update(token).digest('hex')] });
  await h.link.close();
  const restarted = await harness({ storagePath });
  expect(restarted.link.paired).toBe(true);
  expect(await (await restarted.post('/status', {}, token)).json()).toMatchObject({
    reachable: true,
    tokenValid: true,
    protocolVersion: 2,
  });
  expect((await restarted.post('/send', doc, token)).status).toBe(200);
  await restarted.link.disconnect();
  await expect(readFile(storagePath)).rejects.toThrow();
  const third = await harness({ storagePath });
  expect(((await (await third.post('/status', {}, token)).json()) as any).tokenValid).toBe(false);
});
it('denies with a cooldown and permits another request after it ends', async () => {
  let now = 100;
  const h = await harness({ now: () => now, cooldownMs: 1000 });
  h.link.openSetup('sheet');
  const response = h.post('/pair', { code: '123456' });
  await vi.waitFor(() => expect(h.link.state.pending).not.toBeNull());
  await h.link.approve('sheet', h.link.state.pending!.id, false);
  expect((await response).status).toBe(403);
  expect((await h.post('/pair', { code: '123456' })).status).toBe(429);
  now += 1001;
  expect(await h.pair()).toHaveLength(64);
});
it('expires a pending approval without keeping a prompt or token', async () => {
  const h = await harness({ approvalMs: 25 });
  h.link.openSetup('sheet');
  expect((await h.post('/pair', { code: '123456' })).status).toBe(408);
  expect(h.link.state.pending).toBeNull();
  expect(h.link.paired).toBe(false);
});
it('reports status, rejects invalid tokens/schemas/origins/hosts and bounds payloads', async () => {
  let open = false;
  const h = await harness({ compositionOpen: async () => open });
  expect(await (await h.post('/status')).json()).toEqual({
    reachable: true,
    tokenValid: false,
    compositionOpen: false,
    protocolVersion: 2,
  });
  for (const value of [1, true, '', null, [], { token: 'unexpected' }])
    expect((await h.post('/status', value)).status).toBe(400);
  const token = await h.pair();
  open = true;
  expect(await (await h.post('/status', {}, token)).json()).toMatchObject({ tokenValid: true, compositionOpen: true });
  expect((await h.post('/send', doc)).status).toBe(401);
  expect((await h.post('/send', { bad: true }, token)).status).toBe(400);
  expect((await h.post('/status', {}, token, { origin: 'https://evil.example' })).status).toBe(403);
  const hostStatus = await new Promise((resolve) => {
    const r = httpRequest(
      {
        hostname: '127.0.0.1',
        port: h.port,
        path: '/status',
        method: 'POST',
        headers: { host: 'evil.example', 'content-type': 'application/json' },
      },
      (response) => {
        response.resume();
        resolve(response.statusCode);
      },
    );
    r.end('{}');
  });
  expect(hostStatus).toBe(403);
  expect((await h.post('/send', { large: 'x'.repeat(MAX_PAYLOAD) }, token)).status).toBe(413);
  expect((await h.post('/send', doc, token)).status).toBe(200);
  expect(h.received).toHaveLength(1);
});
it('surfaces EADDRINUSE as plain state and can close safely', async () => {
  const h = await harness();
  const second = new FigmaLink(async () => ({}));
  links.push(second);
  await expect(second.start(h.port)).rejects.toMatchObject({ code: 'EADDRINUSE' });
  expect(second.state.error).toContain('Another copy');
  expect(second.state.running).toBe(false);
});

it('shares concurrent listener starts without a second bind', async () => {
  const link = new FigmaLink(async () => ({}));
  links.push(link);
  const [first, second] = await Promise.all([link.start(0), link.start(0)]);
  expect(second.port).toBe(first.port);
});

it('keeps disconnect final when it races an approval disk write', async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-figma-revoke-test-'));
  dirs.push(dir);
  const storagePath = path.join(dir, 'figma-link.json');
  const h = await harness({ storagePath });
  h.link.openSetup('sheet');
  const response = h.post('/pair', { code: '123456' });
  await vi.waitFor(() => expect(h.link.state.pending).not.toBeNull());
  await Promise.all([h.link.approve('sheet', h.link.state.pending!.id, true), h.link.disconnect()]);
  await response.catch(() => undefined);
  await expect(readFile(storagePath)).rejects.toThrow();
  const restarted = await harness({ storagePath });
  expect(restarted.link.paired).toBe(false);
});
