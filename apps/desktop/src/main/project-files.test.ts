import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { atomicWrite, ProjectFiles } from './project-files';

let directory: string;
let registry: string;
let files: ProjectFiles;
const bytes = (name: string) => Buffer.from(JSON.stringify({ proj: { id: 'embedded', name, w: 100, h: 100, layers: [] } }));
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'powermove-project-files-'));
  registry = path.join(directory, 'state', 'files.json');
  files = new ProjectFiles(registry);
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });

describe('real project files', () => {
  it('writes, backs up the previous version, and remembers the destination across restarts', async () => {
    const destination = path.join(directory, 'Demo.pmv');
    await files.save('project-one', bytes('first'), destination);
    files = new ProjectFiles(registry);
    expect(await files.destination('project-one')).toBe(destination);
    await files.save('project-one', bytes('second'));
    expect(await readFile(destination)).toEqual(bytes('second'));
    expect(await readFile(destination + '1')).toEqual(bytes('first'));
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
    await files.save(a.projectId, bytes('second'));
    await expect(files.save(b.projectId, bytes('stale'))).rejects.toThrow('outside Powermove');
  });
  it('serializes concurrent writes and leaves a complete previous version', async () => {
    const destination = path.join(directory, 'a.pmv');
    await files.save('one', bytes('first'), destination);
    await Promise.all([files.save('one', bytes('second')), files.save('one', bytes('third'))]);
    expect(await readFile(destination)).toEqual(bytes('third'));
    expect(await readFile(destination + '1')).toEqual(bytes('second'));
  });
  it('rejects malformed files and reports failed writes without losing the original', async () => {
    const destination = path.join(directory, 'bad.pmv');
    await writeFile(destination, '{}');
    await expect(files.open(destination)).rejects.toThrow('not a Powermove project');
    await expect(atomicWrite(path.join(directory, 'missing', 'a.pmv'), bytes('x'))).rejects.toThrow();
    expect(await readFile(destination, 'utf8')).toBe('{}');
  });
  it('keeps the original intact if the backup cannot be written', async () => {
    const destination = path.join(directory, 'a.pmv');
    await files.save('one', bytes('first'), destination);
    const { mkdir } = await import('node:fs/promises');
    await mkdir(destination + '1');
    await expect(files.save('one', bytes('second'))).rejects.toThrow();
    expect(await readFile(destination)).toEqual(bytes('first'));
  });
});
