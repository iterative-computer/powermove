// Blender supervision in host services (contract 11 and 14): the pinned
// worker, its least-privilege launch, snapshot and asset resolution through
// handles, surfaces as events before the answer, cancel stopping Blender.
// A stand-in executable plays Blender's side of the worker protocol (the
// real Blender runs in pm-export's tests/blender_export.rs and the
// engine-blender e2e spec).
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import type { BlenderJobRequest, Event } from '../../shared/engine-protocol';
import { assetExtension, blenderVersion, EngineBlender, pngSize, resolvedDevice, sweepScratch, WORKER_FLAGS, WORKER_SCRIPT, WORKER_SCRIPT_SHA256, workerEnvironment } from './blender';
import { HandleTable } from './handles';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.resolve(here, '../../../../..');

/** A 1×1 RGBA PNG (8-bit, opaque white). */
const PNG = Buffer.from('89504e470d0a1a0a0000000d4948445200000001000000010806000000' + '1f15c4890000000d49444154789c63f8ffffff7f0009fb03fd2a86e38a0000000049454e44ae426082', 'hex');

/** Blender's side of the worker protocol, in Node: records its arguments
 *  and environment, writes a PNG per output layer and a manifest. With
 *  `PM_FAKE_SLOW` in the request directory it never answers. */
const FAKE = (out: string) => `#!${process.execPath}
const fs = require('fs'), path = require('path'), readline = require('readline');
fs.writeFileSync(${JSON.stringify(out)}, JSON.stringify({ argv: process.argv.slice(2), env: process.env, pid: process.pid }));
const png = Buffer.from(${JSON.stringify(PNG.toString('base64'))}, 'base64');
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, path: file } = JSON.parse(line);
  const req = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (req.snapshot.objects.some((o) => o.id === 'slow')) return;
  const files = [];
  for (const layer of req.outputs) { fs.writeFileSync(path.join(req.dir, layer + '.png'), png); files.push({ id: layer, name: layer + '.png' }); }
  fs.writeFileSync(path.join(req.dir, 'manifest.json'), JSON.stringify({ files, resolvedDevice: { kind: 'metal', name: 'Fake GPU' }, assets: req.assets }));
  console.log('PM_BLENDER:' + JSON.stringify({ id, ok: true }));
});
`;

const snapshot = (ids: string[]) => ({
  width: 2, height: 2,
  settings: { enabled: true, engine: 'eevee', samples: 4, previewSamples: 4, previewScale: 0.5, denoise: true, device: 'auto' },
  objects: ids.map((id) => ({ id, castShadow: true, receiveShadow: true, meshes: [{ positions: [0, 0, 0, 1, 0, 0, 0, 1, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 0, 1, 0, 0, 1], materialIndex: 0, matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1], material: { p: { color: { v: '#FFFFFF' } }, maps: ids[0] === 'tex' ? { color: 'img' } : {} }, smooth: true }] })),
  lights: [], blocks: Object.fromEntries(ids.map((id) => [id, ids])), backgroundFor: [], environment: { ambient: 0.5, ambientColor: '#FFFFFF', exposure: 1 }, background: null,
  camera: { matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 5, 1], orthographic: false, orthoHeight: 4, fov: 45, zoom: 1, near: 0.01, far: 1000 }
});

let root = '';
let area = '';
let fake = '';
let record = '';
const handles = new HandleTable();
const session = 3, port = 9;

beforeEach(async () => {
  root = await mkdtemp(path.join(os.tmpdir(), 'pm-engine-blender-'));
  area = path.join(root, 'Blender Surfaces');
  for (const d of ['snapshots', 'assets']) await mkdir(path.join(area, d), { recursive: true });
  record = path.join(root, 'launch.json');
  fake = path.join(root, 'Blender');
  await writeFile(fake, FAKE(record));
  await chmod(fake, 0o755);
});

afterEach(async () => {
  await rm(root, { recursive: true, force: true });
});

async function job(ids: string[], name: string, extra: Partial<BlenderJobRequest> = {}): Promise<BlenderJobRequest> {
  const bytes = Buffer.from(JSON.stringify(snapshot(ids)));
  const hash = createHash('sha256').update(bytes).digest();
  await writeFile(path.join(area, 'snapshots', `${hash.toString('hex')}.json`), bytes);
  const cache = handles.mint({ kind: { kind: 'cache', cache: 'blenderSurfaces' }, session, port, path: area });
  return {
    job: name,
    priority: 'userJob',
    snapshot: { hash, encoding: 'renderSnapshotJsonV1', handle: cache, bytes: bytes.length },
    workerScriptHash: WORKER_SCRIPT_SHA256,
    assets: [],
    settings: { engine: 'eevee', samples: 4, preview: false, previewScale: 0.5, denoise: true, device: 'auto' },
    surfaces: ids.map((layer, i) => ({ layer, compositionPath: ['root'], time: 0, shutter: null, size: { width: 2, height: 2 }, key: new Uint8Array(32).fill(i + 1) })),
    batchLayers: true,
    output: cache,
    ...extra
  };
}

const blender = (available = true) =>
  new EngineBlender({
    locate: async () => (available ? { available: true, path: fake, version: 'Blender 4.5.14 LTS' } : { available: false, error: 'Install Blender' }),
    handles,
    tempRoot: root,
    log: () => {}
  });

describe('the pinned worker', () => {
  it('is the file pm-blender hashes, and pins section 17 output', async () => {
    const file = await readFile(path.join(repo, 'engine/crates/pm-blender/worker/worker.py'), 'utf8');
    expect(WORKER_SCRIPT).toBe(file);
    expect(Buffer.from(WORKER_SCRIPT_SHA256)).toEqual(createHash('sha256').update(file).digest());
    for (const line of ["image.color_depth='16'", "scene.view_settings.view_transform='AgX';scene.view_settings.look='None';scene.view_settings.gamma=1", "scene.render.dither_intensity=0", "'use_stamp_date'"]) expect(WORKER_SCRIPT).toContain(line);
    expect(WORKER_FLAGS).toEqual(['--background', '--factory-startup', '--disable-autoexec', '--offline-mode', '--python-exit-code', '1', '--python']);
    expect(blenderVersion('Blender 4.5.14 LTS')).toBe('4.5.14');
    expect(blenderVersion('Blender 5.0')).toBe('5.0.0');
    expect(resolvedDevice({ resolvedDevice: { kind: 'oneapi', name: 'Arc' } })).toEqual({ kind: 'oneApi', name: 'Arc' });
    expect(resolvedDevice({})).toBeNull();
  });
});

describe('asset extensions', () => {
  it('come from the file, or from its first bytes when the media store keeps none', async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-blender-ext-'));
    try {
      const write = async (name: string, bytes: number[] | string) => {
        const file = path.join(dir, name);
        await writeFile(file, typeof bytes === 'string' ? Buffer.from(bytes, 'utf8') : Buffer.from(bytes));
        return file;
      };
      expect(await assetExtension(await write('a.PNG', [0]))).toBe('.png');
      expect(await assetExtension(await write('png', [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]))).toBe('.png');
      expect(await assetExtension(await write('jpeg', [0xff, 0xd8, 0xff, 0xe0]))).toBe('.jpg');
      expect(await assetExtension(await write('webp', 'RIFF\0\0\0\0WEBPVP8 '))).toBe('.webp');
      expect(await assetExtension(await write('glb', 'glTF\x02\0\0\0'))).toBe('.glb');
      expect(await assetExtension(await write('blend', 'BLENDER-v405'))).toBe('.blend');
      // Images other than PNG, JPEG and WebP reach the worker converted to PNG by the engine
      // (today's canvas); an EXR or HDR file without its extension is not passed through.
      expect(await assetExtension(await write('exr', [0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0]))).toBeNull();
      expect(await assetExtension(await write('hdr', '#?RADIANCE\n'))).toBeNull();
      expect(await assetExtension(await write('linked.exr', [0]))).toBe('.exr');
      // Text models, named by the asset's name today (`chair.obj`, `scene.gltf`).
      expect(await assetExtension(await write('obj', '# Blender export\n\nmtllib chair.mtl\no Chair\nv 0 0 0\n'))).toBe('.obj');
      expect(await assetExtension(await write('obj-crlf', '\ufeff\r\n#c\r\nv 1 2 3\r\nf 1 1 1\r\n'))).toBe('.obj');
      expect(await assetExtension(await write('obj-group', 'g part\nv 0 0 0\n'))).toBe('.obj');
      expect(await assetExtension(await write('gltf', '{\n  "asset": { "version": "2.0" },\n  "scenes": []\n}'))).toBe('.gltf');
      expect(await assetExtension(await write('gltf-bom', '\ufeff  {"scene":0,"asset":{"version":"2.0"}}'))).toBe('.gltf');
      expect(await assetExtension(await write('json', '{"name":"not a model"}'))).toBeNull();
      expect(await assetExtension(await write('text', 'hello world\n'))).toBeNull();
      expect(await assetExtension(await write('binary', [0x00, 0x01, 0x76, 0x20]))).toBeNull();
      expect(await assetExtension(await write('tiff', [0x4d, 0x4d, 0, 0x2a]))).toBeNull();
      expect(await assetExtension(path.join(dir, 'absent'))).toBeNull();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe('scratch', () => {
  it('of processes that no longer run is swept', async () => {
    const parent = path.join(root, 'powermove-engine-blender');
    for (const name of ['999999', String(process.pid), 'not-a-pid']) await mkdir(path.join(parent, name), { recursive: true });
    await sweepScratch(parent);
    const left = (await import('node:fs/promises')).readdir(parent);
    expect((await left).sort()).toEqual(['not-a-pid', String(process.pid)].sort());
  });
});

describe('Blender jobs from the engines', () => {
  it('launch Blender least-privileged and deliver every surface before the answer', async () => {
    const b = blender();
    const events: Event[] = [];
    const req = await job(['a', 'b'], 'j1');
    await b.render(req, session, port, (e) => events.push(e));
    const launch = JSON.parse(await readFile(record, 'utf8'));
    expect(launch.argv.slice(0, 7)).toEqual([...WORKER_FLAGS]);
    expect(launch.argv[7]).toMatch(/worker\.py$/);
    // Nothing of main's environment reaches Blender (macOS's CoreFoundation
    // sets __CF_USER_TEXT_ENCODING in every process it starts).
    const seen = Object.keys(launch.env).filter((k) => k !== '__CF_USER_TEXT_ENCODING').sort();
    expect(seen).toEqual(Object.keys(workerEnvironment('/x')).sort());
    expect(launch.env.HOME.startsWith(root)).toBe(true);
    const surfaces = events.filter((e) => e.type === 'blenderSurface');
    expect(surfaces).toHaveLength(2);
    expect(events.filter((e) => e.type === 'blenderProgress').at(-1)).toMatchObject({ completed: 2, total: 2 });
    for (const [i, s] of surfaces.entries()) {
      if (s.type !== 'blenderSurface') throw new Error('surface');
      expect(s).toMatchObject({ job: 'j1', requestIndex: i, size: { width: 1, height: 1 }, crop: { x: 0, y: 0, width: 1, height: 1 }, device: { kind: 'metal', name: 'Fake GPU' } });
      const file = path.join(area, `${Buffer.from(s.key).toString('hex')}.png`);
      expect((await stat(file)).size).toBe(PNG.length);
      expect(await pngSize(file)).toEqual({ width: 1, height: 1 });
    }
    // One invocation per layer when batching is off.
    const single = await job(['a', 'b'], 'j2', { batchLayers: false });
    const more: Event[] = [];
    await b.render(single, session, port, (e) => more.push(e));
    expect(more.filter((e) => e.type === 'blenderSurface')).toHaveLength(2);
    expect((await b.status())).toEqual({ type: 'blenderStatus', available: true, version: '4.5.14', error: null });
    await b.dispose();
  });

  it('refuse another worker script, foreign handles, and a missing Blender', async () => {
    const b = blender();
    const wrong = await job(['a'], 'j3', { workerScriptHash: new Uint8Array(32) });
    await expect(b.render(wrong, session, port, () => {})).rejects.toMatchObject({ code: 'invalidMessage' });
    const foreign = await job(['a'], 'j4');
    await expect(b.render(foreign, session, port + 1, () => {})).rejects.toThrow();
    const missing = blender(false);
    await expect(missing.render(await job(['a'], 'j5'), session, port, () => {})).rejects.toMatchObject({ code: 'blenderMissing' });
    expect(await missing.status()).toMatchObject({ available: false, error: 'Install Blender' });
    await b.dispose();
  });

  it('stop Blender when cancelled', async () => {
    const b = blender();
    const req = await job(['slow'], 'j6');
    const running = b.render(req, session, port, () => {});
    // Wait for the stand-in to start.
    for (let i = 0; i < 100; i++) {
      if (await stat(record).catch(() => null)) break;
      await new Promise((r) => setTimeout(r, 20));
    }
    const { pid } = JSON.parse(await readFile(record, 'utf8'));
    // An engine cancels only its own launch's jobs (the interactive
    // engine's paused viewer and the job engine share one Blender).
    expect(b.owns('j6', port)).toBe(true);
    expect(b.owns('j6', port + 1)).toBe(false);
    expect(b.cancel('j6')).toBe(true);
    await expect(running).rejects.toMatchObject({ code: 'cancelled' });
    await new Promise((r) => setTimeout(r, 100));
    expect(() => process.kill(pid, 0)).toThrow();
    // A queued job of a launch that ended is cancelled too.
    expect(b.cancel('nothing')).toBe(false);
    await b.dispose();
  });

  it('pause Blender while anything plays and resume it after', async () => {
    const { execFileSync } = await import('node:child_process');
    const state = (pid: number) => execFileSync('/bin/ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' }).trim();
    const b = blender();
    b.setPaused(true);
    // A job started during playback waits: its Blender starts stopped.
    let done = false;
    const running = b.render(await job(['a'], 'j7'), session, port, () => {}).then(() => { done = true; });
    for (let i = 0; i < 100; i++) {
      if (await stat(record).catch(() => null)) break;
      await new Promise((r) => setTimeout(r, 20));
    }
    await new Promise((r) => setTimeout(r, 300));
    expect(done).toBe(false);
    b.setPaused(false);
    await running;
    expect(done).toBe(true);
    // A running job stops and continues with playback.
    const slow = b.render(await job(['slow'], 'j8'), session, port, () => {});
    await new Promise((r) => setTimeout(r, 300));
    const { pid } = JSON.parse(await readFile(record, 'utf8'));
    b.setPaused(true);
    await new Promise((r) => setTimeout(r, 100));
    expect(state(pid)).toMatch(/^T/);
    b.setPaused(false);
    await new Promise((r) => setTimeout(r, 100));
    expect(state(pid)).not.toMatch(/^T/);
    b.cancel('j8');
    await expect(slow).rejects.toMatchObject({ code: 'cancelled' });
    await b.dispose();
  });

  it('lets only an explicit interactive cache capture finish during playback, then restores the pause', async () => {
    const { execFileSync } = await import('node:child_process');
    const state = (pid: number) => execFileSync('/bin/ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' }).trim();
    const b = blender();
    await b.render(await job(['warm'], 'warm'), session, port, () => {});
    const { pid } = JSON.parse(await readFile(record, 'utf8'));
    b.setPaused(true);
    expect(state(pid)).toMatch(/^T/);
    const automatic = b.render(await job(['a'], 'automatic', { priority: 'interactive' }), session, port, () => {});
    const capture = b.render(await job(['b'], 'foreground', { priority: 'interactive', foregroundCapture: true }), session, port, () => {});
    await Promise.all([automatic, capture]);
    expect(state(pid)).toMatch(/^T/);
    let completed = false;
    const ordinary = b.render(await job(['c'], 'ordinary', { priority: 'interactive' }), session, port, () => {}).then(() => { completed = true; });
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(completed).toBe(false);
    b.setPaused(false);
    await ordinary;
    expect(completed).toBe(true);
    await b.dispose();
  });

  it('restores playback pause when the queued foreground capture is cancelled', async () => {
    const { execFileSync } = await import('node:child_process');
    const state = (pid: number) => execFileSync('/bin/ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' }).trim();
    const b = blender();
    await b.render(await job(['warm'], 'warm'), session, port, () => {});
    const { pid } = JSON.parse(await readFile(record, 'utf8'));
    b.setPaused(true);
    const automatic = b.render(await job(['slow'], 'slow-preview', { priority: 'interactive' }), session, port, () => {})
      .then(() => null, (error: Error) => error);
    const capture = b.render(await job(['b'], 'foreground', { priority: 'interactive', foregroundCapture: true }), session, port, () => {})
      .then(() => null, (error: Error) => error);
    expect(b.cancel('foreground')).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(state(pid)).toMatch(/^T/);
    expect(b.cancel('slow-preview')).toBe(true);
    expect(await automatic).toMatchObject({ code: 'cancelled' });
    expect(await capture).toMatchObject({ code: 'cancelled' });
    await b.dispose();
  });

  it('rejects a foreground capture from a non-interactive class', async () => {
    const b = blender();
    await expect(b.render(await job(['a'], 'invalid', { foregroundCapture: true }), session, port, () => {}))
      .rejects.toMatchObject({ code: 'invalidMessage' });
    await b.dispose();
  });
});
