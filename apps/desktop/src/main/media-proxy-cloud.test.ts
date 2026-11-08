import { copyFile, mkdtemp, readFile, readdir, rename, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { MediaProxyService } from './media-proxy';

const roots: string[] = [], services: MediaProxyService[] = [], resume: Array<() => void> = [];
afterEach(async () => {
  resume.splice(0).forEach(finish => finish());
  await Promise.all(services.splice(0).map(service => service.dispose()));
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  vi.restoreAllMocks();
});
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), 'pm-proxy-cloud-')); roots.push(root);
  return root;
}

it.each(['video', 'still', 'sequence'])('blocks an offloaded %s before conversion or creating temporary files', async kind => {
  const root = await fixture();
  const convert = vi.fn(), prepare = vi.fn(async () => { throw new Error('This file is stored in iCloud. Download it in Finder, then try again.'); });
  const service = new MediaProxyService(root, convert, convert, undefined, convert, prepare); services.push(service);
  const action = kind === 'video' ? service.create('/synthetic.mov') : kind === 'still' ? service.createStillImage('/synthetic.heic')
    : service.createSequence(['/synthetic-001.png', '/synthetic-002.png'], 30);
  await expect(action).rejects.toThrow('Download it in Finder');
  expect(convert).not.toHaveBeenCalled();
  expect(await readdir(root)).toEqual([]);
});

it('never shares a conversion with a replaced file, even when sync preserves its size and mtime', async () => {
  const root = await fixture(), source = path.join(root, 'synthetic.mov');
  const timestamp = new Date('2026-01-01');
  await writeFile(source, 'old footage'); await utimes(source, timestamp, timestamp);
  let finish!: () => void;
  const wait = new Promise<void>(resolve => { finish = resolve; resume.push(resolve); });
  const convert = vi.fn(async (input: string, output: string) => { await copyFile(input, output); await wait; });
  const service = new MediaProxyService(root, convert, undefined, undefined, undefined, async () => {}); services.push(service);
  const first = service.create(source);
  await vi.waitFor(() => expect(convert).toHaveBeenCalledTimes(1));
  await writeFile(source + '.incoming', 'new footage');
  await utimes(source + '.incoming', timestamp, timestamp);
  await rename(source + '.incoming', source);
  const second = service.create(source);
  await vi.waitFor(() => expect(convert).toHaveBeenCalledTimes(2));
  finish();
  const [a, b] = await Promise.all([first, second]);
  expect(Buffer.from(await service.read(a.token, 0, a.size)).toString()).toBe('old footage');
  expect(Buffer.from(await service.read(b.token, 0, b.size)).toString()).toBe('new footage');
  expect(await readFile(source, 'utf8')).toBe('new footage');
});
