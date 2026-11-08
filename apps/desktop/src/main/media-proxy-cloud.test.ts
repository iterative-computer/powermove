import { copyFile, mkdtemp, readFile, readdir, rename, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { MediaProxyService } from './media-proxy';
import { FileContentCache } from './file-content-version';

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

it.each(['metadata', 'content'])('shares conversion only when the bytes match after a %s change', async change => {
  const root = await fixture(), source = path.join(root, 'synthetic.mov');
  const timestamp = new Date('2026-01-01');
  await writeFile(source, 'old footage'); await utimes(source, timestamp, timestamp);
  let finish!: () => void;
  const wait = new Promise<void>(resolve => { finish = resolve; resume.push(resolve); });
  const convert = vi.fn(async (input: string, output: string) => {
    await copyFile(input, output);
    await wait;
  });
  const service = new MediaProxyService(root, convert, undefined, undefined, undefined, async () => {}); services.push(service);
  const hash = vi.spyOn(FileContentCache.prototype, 'hash');
  const first = service.create(source);
  await vi.waitFor(() => expect(convert).toHaveBeenCalledTimes(1));
  if (change === 'content') {
    await writeFile(source + '.incoming', 'new footage');
    await utimes(source + '.incoming', timestamp, timestamp);
    await rename(source + '.incoming', source);
  } else await utimes(source, new Date('2026-02-01'), new Date('2026-02-01'));
  const second = service.create(source);
  if (change === 'content') await vi.waitFor(() => expect(convert).toHaveBeenCalledTimes(2));
  // Let the second request finish fingerprinting before unblocking conversion.
  await vi.waitFor(() => expect(hash).toHaveBeenCalledTimes(2));
  await hash.mock.results[1]!.value;
  await Promise.resolve();
  finish();
  const [a, b] = await Promise.all([first, second]);
  expect(convert).toHaveBeenCalledTimes(change === 'content' ? 2 : 1);
  expect(Buffer.from(await service.read(a.token, 0, a.size)).toString()).toBe('old footage');
  expect(Buffer.from(await service.read(b.token, 0, b.size)).toString()).toBe(change === 'content' ? 'new footage' : 'old footage');
  expect(await readFile(source, 'utf8')).toBe(change === 'content' ? 'new footage' : 'old footage');
});
