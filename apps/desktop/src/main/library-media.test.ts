import { mkdir, mkdtemp, readFile, readdir, rename, rm, symlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { LibraryMedia } from './library-media';

let root: string, media: LibraryMedia;
beforeEach(async () => { root = await mkdtemp(path.join(tmpdir(), 'powermove-file-media-')); media = new LibraryMedia(root); await media.initialize(); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
async function put(key: string, bytes = 'synthetic media', revision?: string) {
  const token = await media.begin(1, { key, size: Buffer.byteLength(bytes), type: 'video/mp4', revision });
  await media.chunk(1, token, 0, Buffer.from(bytes));
  return media.finish(1, token);
}

it('links an imported original without copying it and reads the link after a restart', async () => {
  const source = path.join(root, 'original.mp4'); await writeFile(source, 'original video');
  const record = await media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4' });
  expect(record.sourcePath).toBe(source);
  expect((await readdir(root)).filter(name => /^[a-f0-9]{64}\./.test(name))).toEqual([]);
  const reopened = new LibraryMedia(root); await reopened.initialize();
  const saved = (await reopened.get(2, 'clip'))!;
  expect(await (await reopened.serve(saved.url.split('/').at(-1)!, new Request(saved.url))).text()).toBe('original video');
  await reopened.remove('clip');
  expect(await readFile(source, 'utf8')).toBe('original video');
});

it('preserves the old media unless a migration source matches its full checksum', async () => {
  await put('clip', 'original video', 'old-revision');
  const source = path.join(root, 'source.mp4'); await writeFile(source, 'different data');
  await expect(media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4', matchExisting: true, revision: 'old-revision' })).rejects.toThrow('match');
  expect(await readFile((await media.fileFor('clip'))!, 'utf8')).toBe('original video');
  await writeFile(source, 'original video');
  const linked = await media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4', matchExisting: true, revision: 'old-revision' });
  expect(linked.sourcePath).toBe(source);
  expect(linked.revision).toBe('old-revision');
  await media.remove('clip');
  const date = new Date('2026-01-01'); await utimes(source, date, date);
  await media.collectUnused(Date.parse('2026-10-14'));
  expect(await readFile(source, 'utf8')).toBe('original video');
});

it('reports moved sources for relinking without serving a different or symlinked file', async () => {
  const source = path.join(root, 'source.mp4'); await writeFile(source, 'original video');
  const record = await media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4' });
  await rename(source, source + '.moved');
  await expect(media.get(1, 'clip')).rejects.toThrow('Locate File');
  expect((await media.serve(record.url.split('/').at(-1)!, new Request(record.url))).status).toBe(404);
  await symlink(source + '.moved', source);
  await expect(media.get(1, 'clip')).rejects.toThrow('Locate File');
});

it('does not read or serve an offloaded source before the cloud download is authorized', async () => {
  const source = path.join(root, 'cloud.mp4'); await writeFile(source, 'original video');
  let available = true;
  media = new LibraryMedia(root, undefined, async () => available);
  const linked = await media.link(1, { key: 'cloud', sourcePath: source, size: 14, type: 'video/mp4' });
  available = false;
  await expect(media.get(1, 'cloud')).rejects.toThrow('Locate File');
  expect((await media.serve(linked.url.split('/').at(-1)!, new Request(linked.url))).status).toBe(404);
  expect((await media.get(1, 'cloud', true))?.sourcePath).toBe(source);
  await expect(media.link(1, { key: 'other', sourcePath: source, size: 14, type: 'video/mp4' })).rejects.toThrow('Download');
});

it('durably stores shared payloads once, preserves revisions and serves them without browser caching', async () => {
  const first = await put('one', 'synthetic media', 'revision-one');
  await put('two');
  expect((await readdir(root)).filter(name => name.endsWith('.mp4'))).toHaveLength(1);
  const reopened = new LibraryMedia(root); await reopened.initialize();
  const stored = await reopened.get(2, 'one');
  expect(stored?.revision).toBe('revision-one');
  const token = stored!.url.split('/').at(-1)!;
  const response = await reopened.serve(token, new Request(stored!.url));
  expect(response.headers.get('cache-control')).toBe('no-store');
  expect(await response.text()).toBe('synthetic media');
  expect(first.size).toBe('synthetic media'.length);
});

it('serves exact bounded ranges without reading unrelated bytes', async () => {
  const record = await put('range', '0123456789');
  const token = record.url.split('/').at(-1)!;
  const response = await media.serve(token, new Request(record.url, { headers: { Range: 'bytes=2-5' } }));
  expect(response.status).toBe(206);
  expect(response.headers.get('Content-Range')).toBe('bytes 2-5/10');
  expect(response.headers.get('Content-Length')).toBe('4');
  expect(await response.text()).toBe('2345');
  for (const range of ['bytes=0-10', 'bytes=6-5', 'bytes=-1-4', 'bytes=0-1,4-5', 'bytes=9007199254740992-9007199254740993']) {
    expect((await media.serve(token, new Request(record.url, { headers: { Range: range } }))).status).toBe(416);
  }
});

it('honors request cancellation and allows another read after the consumer cancels', async () => {
  const record = await put('cancel', 'some synthetic media');
  const token = record.url.split('/').at(-1)!;
  const controller = new AbortController(); controller.abort();
  await expect(media.serve(token, new Request(record.url, { signal: controller.signal }))).rejects.toMatchObject({ name: 'AbortError' });
  const response = await media.serve(token, new Request(record.url));
  await response.body!.cancel();
  expect(await (await media.serve(token, new Request(record.url))).text()).toBe('some synthetic media');
});

it('rejects incomplete uploads and cross-window access, preserving an existing copy on failed replacement', async () => {
  await put('one', 'original');
  const token = await media.begin(1, { key: 'one', size: 99, type: 'video/mp4' });
  await expect(media.chunk(2, token, 0, Buffer.from('other window'))).rejects.toThrow('Invalid');
  await expect(media.finish(2, token)).rejects.toThrow('Unknown');
  await media.chunk(1, token, 0, Buffer.from('partial'));
  await expect(media.finish(1, token)).rejects.toThrow('incomplete');
  expect(await readFile((await media.fileFor('one'))!, 'utf8')).toBe('original');
  expect(await readdir(path.join(root, '.incoming'))).toEqual([]);
});

it('does not let an old migration overwrite a newer import of the same key', async () => {
  await put('one', 'new import', 'new-revision');
  const token = await media.begin(1, { key: 'one', size: 3, type: 'video/mp4', revision: 'old-revision', onlyIfMissing: true });
  await media.chunk(1, token, 0, Buffer.from('old'));
  expect((await media.finish(1, token)).revision).toBe('new-revision');
  expect(await readFile((await media.fileFor('one'))!, 'utf8')).toBe('new import');
});

it('refuses traversal keys and symlinked payloads and detects externally modified bytes', async () => {
  for (const key of ['../outside', 'a/b', '', 'a\0b']) await expect(media.get(1, key)).rejects.toThrow('key');
  await put('one');
  const file = (await media.fileFor('one'))!;
  await writeFile(file, 'changed file of different size');
  await expect(media.get(1, 'one')).rejects.toThrow('changed');
  await rm(file); await symlink(path.join(root, 'Index'), file);
  await expect(media.get(1, 'one')).rejects.toThrow('changed');
});

it('releases abandoned uploads and grants, and removes only old unreferenced payloads', async () => {
  const record = await put('one', 'old');
  const old = (await media.fileFor('one'))!;
  await put('one', 'new');
  const referenced = (await media.fileFor('one'))!;
  const date = new Date('2026-01-01'); await utimes(old, date, date); await utimes(referenced, date, date);
  const token = await media.begin(1, { key: 'pending', size: 5, type: 'video/mp4' });
  await media.chunk(1, token, 0, Buffer.from('123'));
  await media.release(1);
  expect(await readdir(path.join(root, '.incoming'))).toEqual([]);
  expect((await media.serve(record.url.split('/').at(-1)!, new Request(record.url))).status).toBe(404);
  expect(await media.collectUnused(Date.parse('2026-10-14'))).toBe(3);
  expect(await readFile(referenced, 'utf8')).toBe('new');
});

it('removes a verified imported artifact duplicate while keeping its conversation download pinned', async () => {
  const source = path.join(root, 'working.mp4'); await writeFile(source, 'generated media');
  media = new LibraryMedia(root, async () => source);
  const reference = { projectId: 'one', path: 'run/working.mp4' };
  const token = await media.begin(1, { key: 'one', size: 15, type: 'video/mp4', artifact: reference });
  await media.chunk(1, token, 0, Buffer.from('generated media')); await media.finish(1, token);
  await expect(readFile(source)).rejects.toMatchObject({ code: 'ENOENT' });
  await media.remove('one');
  const pinned = await media.archivedArtifact(reference);
  expect(await readFile(pinned!.path, 'utf8')).toBe('generated media');
  const old = new Date('2026-01-01'); await utimes(pinned!.path, old, old);
  expect(await media.collectUnused(Date.parse('2026-10-14'))).toBe(0);
  await media.forgetArtifacts('one');
  expect(await media.collectUnused(Date.parse('2026-10-14'))).toBe(15);
});

it('keeps an artifact whose bytes differ from its imported conversion', async () => {
  const source = path.join(root, 'source.mov'); await writeFile(source, 'original format');
  media = new LibraryMedia(root, async () => source);
  const token = await media.begin(1, { key: 'converted', size: 15, type: 'video/mp4', artifact: { projectId: 'one', path: 'run/source.mov' } });
  await media.chunk(1, token, 0, Buffer.from('converted media')); await media.finish(1, token);
  expect(await readFile(source, 'utf8')).toBe('original format');
});

it('does not reuse an old download when a restored revision label has different media bytes', async () => {
  const first = await put('one', 'first', 'same-revision');
  const second = await put('one', 'second', 'same-revision');
  expect(second.url).not.toBe(first.url);
  expect(await (await media.serve(second.url.split('/').at(-1)!, new Request(second.url))).text()).toBe('second');
});

it('upgrade removes a verified linked duplicate, updates existing download grants, and keeps unreferenced unique files', async () => {
  const saved = await put('clip', 'original video', 'original-revision');
  const owned = (await media.fileFor('clip'))!;
  const source = path.join(root, 'original.mp4'); await writeFile(source, 'original video');
  await media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4', revision: 'original-revision', matchExisting: true });
  const unrelated = path.join(root, 'a'.repeat(64) + '.mp4'); await writeFile(unrelated, 'unique');
  expect(await media.collectLinkedDuplicates()).toBe(14);
  await expect(readFile(owned)).rejects.toMatchObject({ code: 'ENOENT' });
  expect(await (await media.serve(saved.url.split('/').at(-1)!, new Request(saved.url))).text()).toBe('original video');
  expect(await readFile(unrelated, 'utf8')).toBe('unique'); expect(await readFile(source, 'utf8')).toBe('original video');
});

it('upgrade keeps owned media referenced by another project or artifact and skips a changed original', async () => {
  await put('clip', 'original video', 'revision'); await put('other-project', 'original video');
  const owned = (await media.fileFor('clip'))!;
  const source = path.join(root, 'source.mp4'); await writeFile(source, 'original video');
  await media.link(1, { key: 'clip', sourcePath: source, size: 14, type: 'video/mp4', revision: 'revision', matchExisting: true });
  expect(await media.collectLinkedDuplicates()).toBe(0);
  await media.remove('other-project'); await media.release(1);
  await writeFile(source, 'different data');
  expect(await media.collectLinkedDuplicates()).toBe(0); expect(await readFile(owned, 'utf8')).toBe('original video');
});

it('upgrade does not delete a link that names an owned payload itself', async () => {
  await put('clip', 'original video', 'revision'); const owned = (await media.fileFor('clip'))!;
  await media.link(1, { key: 'clip', sourcePath: owned, size: 14, type: 'video/mp4', revision: 'revision', matchExisting: true });
  expect(await media.collectLinkedDuplicates()).toBe(0); expect(await readFile(owned, 'utf8')).toBe('original video');
});
