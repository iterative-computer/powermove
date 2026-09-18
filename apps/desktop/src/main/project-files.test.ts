import { mkdir, mkdtemp, readFile, readdir, rm, writeFile, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { atomicWrite, ProjectFiles } from './project-files';

let directory: string;
let registry: string;
let backupDirectory: string;
let files: ProjectFiles;
const bytes = (name: string) => Buffer.from(JSON.stringify({ proj: { id: 'embedded', name, w: 100, h: 100, layers: [] } }));
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'powermove-project-files-'));
  registry = path.join(directory, 'state', 'files.json');
  backupDirectory = path.join(directory, 'backups');
  files = new ProjectFiles(registry, backupDirectory);
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });

describe('real project files', () => {
  it('opens media beyond 1 GiB using a bounded read token', async () => {
    const destination = path.join(directory, 'large.pmv');
    const length = 1024 * 1024 * 1024 + 17;
    const header = Buffer.from(JSON.stringify({ document: { proj: { w: 100, h: 100, layers: [] } },
      media: [{ id: 'video', type: 'video/webm', offset: 0, length }] }));
    const prefix = Buffer.alloc(9); prefix.write('PMV3\n'); prefix.writeUInt32LE(header.length, 5);
    const handle = await open(destination, 'w');
    try {
      await handle.writeFile(Buffer.concat([prefix, header]));
      await handle.truncate(9 + header.length + length);
      await handle.write(Buffer.from([7, 8, 9, 10]), 0, 4, 9 + header.length + length - 4);
    } finally { await handle.close(); }
    const result = await files.open(destination);
    expect('data' in result).toBe(false);
    expect(result.media[0]!.length).toBe(length);
    try {
      expect([...await files.read(result.token, result.size - 4, 4)]).toEqual([7, 8, 9, 10]);
      await expect(files.read(result.token, 0, 1024 * 1024 + 1)).rejects.toThrow('range');
    } finally { await files.close(result.token); }
  }, 30000);

  it('writes, backs up the previous version, and remembers the destination across restarts', async () => {
    const destination = path.join(directory, 'Demo.pmv');
    await files.save('project-one', bytes('first'), destination);
    files = new ProjectFiles(registry, backupDirectory);
    expect(await files.destination('project-one')).toBe(destination);
    await files.save('project-one', bytes('second'));
    expect(await readFile(destination)).toEqual(bytes('second'));
    const backups = await files.backups('project-one');
    expect(backups).toHaveLength(1);
    expect(await readFile(backups[0]!)).toEqual(bytes('first'));
    expect((await readdir(directory)).some(name => name.endsWith('.tmp'))).toBe(false);
  });
  it('Save As changes only that project’s destination', async () => {
    const a = path.join(directory, 'a.pmv'), b = path.join(directory, 'b.pmv');
    await files.save('one', bytes('original'), a);
    await files.save('one', bytes('copy'), b);
    await files.save('one', bytes('latest'));
    expect(await readFile(a)).toEqual(bytes('original'));
    expect(await readFile(b)).toEqual(bytes('latest'));
  });
  it('refuses unknown destinations and externally changed or deleted files', async () => {
    await expect(files.save('unknown', bytes('x'))).rejects.toThrow('Save As');
    const destination = path.join(directory, 'a.pmv');
    await files.save('one', bytes('first'), destination);
    await writeFile(destination, bytes('external'));
    await expect(files.save('one', bytes('overwrite'))).rejects.toThrow('outside Powermove');
    expect(await readFile(destination)).toEqual(bytes('external'));
    await rm(destination);
    await expect(files.save('one', bytes('overwrite'))).rejects.toThrow('outside Powermove');
  });
  it('opens with fresh project identities and blocks stale open copies from overwriting', async () => {
    const destination = path.join(directory, 'a.pmv');
    await writeFile(destination, bytes('first'));
    const a = await files.open(destination), b = await files.open(destination);
    expect(a.projectId).not.toBe(b.projectId);
    expect(a.projectId).not.toBe('embedded');
    await files.close(a.token); await files.close(b.token);
    await files.save(a.projectId, bytes('second'));
    await expect(files.save(b.projectId, bytes('stale'))).rejects.toThrow('outside Powermove');
  });
  it('serializes concurrent writes and leaves a complete previous version', async () => {
    const destination = path.join(directory, 'a.pmv');
    await files.save('one', bytes('first'), destination);
    await Promise.all([files.save('one', bytes('second')), files.save('one', bytes('third'))]);
    expect(await readFile(destination)).toEqual(bytes('third'));
    const backups = await files.backups('one');
    expect(await readFile(backups[0]!)).toEqual(bytes('second'));
  });
  it('rejects malformed files and reports failed writes without losing the original', async () => {
    const destination = path.join(directory, 'bad.pmv');
    await writeFile(destination, '{}');
    await expect(files.open(destination)).rejects.toThrow('not a Powermove project');
    await expect(atomicWrite(path.join(directory, 'missing', 'a.pmv'), bytes('x'))).rejects.toThrow();
    expect(await readFile(destination, 'utf8')).toBe('{}');
  });
  it('preserves the original and removes temporary files when a stream fails mid-save', async () => {
    const destination = path.join(directory, 'stream.pmv');
    await files.save('one', bytes('original'), destination);
    async function* failure() { yield bytes('partial'); throw new Error('disk disconnected'); }
    await expect(files.save('one', failure())).rejects.toThrow('disk disconnected');
    expect(await readFile(destination)).toEqual(bytes('original'));
    expect((await readdir(directory)).some(name => name.endsWith('.tmp'))).toBe(false);
    await files.save('one', bytes('retry'));
    expect(await readFile(destination)).toEqual(bytes('retry'));
  });
  it('rejects changed files at the end of import and releases the read handle', async () => {
    const destination = path.join(directory, 'changed.pmv');
    await writeFile(destination, bytes('before'));
    const opened = await files.open(destination);
    await writeFile(destination, bytes('after and larger'));
    await expect(files.close(opened.token)).rejects.toThrow('changed while opening');
    await expect(files.read(opened.token, 0, 1)).rejects.toThrow('range');
  });
  it('keeps the original intact if the backup cannot be written', async () => {
    const destination = path.join(directory, 'a.pmv');
    await files.save('one', bytes('first'), destination);
    await mkdir(backupDirectory, { recursive: true });
    await writeFile(path.join(backupDirectory, 'one'), 'not a directory');
    await expect(files.save('one', bytes('second'))).rejects.toThrow();
    expect(await readFile(destination)).toEqual(bytes('first'));
  });
  it('keeps only the five newest backups', async () => {
    const destination = path.join(directory, 'pruned.pmv');
    for (let version = 1; version <= 7; version++) await files.save('one', bytes(`version-${version}`), destination);
    const backups = await files.backups('one');
    expect(backups).toHaveLength(5);
    expect(backups).toEqual([...backups].sort((a, b) => path.basename(b).localeCompare(path.basename(a))));
    expect(await readFile(backups[0]!)).toEqual(bytes('version-6'));
  });
});
