import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProjectFiles } from './project-files';
import { publishFile, RecoveryFileError } from './durable-file';

vi.mock('./durable-file', async importOriginal => ({
  ...await importOriginal<typeof import('./durable-file')>(), publishFile: vi.fn(),
}));
let directory: string, destination: string, files: ProjectFiles;
const document = (name: string) => Buffer.from(JSON.stringify({ proj: { name, w: 100, h: 100, layers: [] } }));
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'pm-save-recovery-'));
  destination = path.join(directory, 'Project.pmv');
  files = new ProjectFiles(path.join(directory, 'registry.json'), path.join(directory, 'backups'));
  const actual = await vi.importActual<typeof import('./durable-file')>('./durable-file');
  vi.mocked(publishFile).mockReset().mockImplementation(actual.publishFile);
  await files.save('one', document('first'), destination);
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });

describe('legacy save publication cleanup', () => {
  it('retains displaced foreign bytes when recovery finalization leaves them at the staging path', async () => {
    let recoveryPath = '';
    vi.mocked(publishFile).mockImplementationOnce(async temporary => {
      // Model a verified exchange conflict whose recovery rename failed while
      // the directory still permits unlink. The caller must retain these bytes.
      recoveryPath = temporary;
      await writeFile(temporary, 'foreign work retained after exchange');
      throw new RecoveryFileError('Recovery rename failed.', temporary);
    });
    await expect(files.save('one', document('second'))).rejects.toMatchObject({ recoveryPath: expect.any(String) });
    expect(await readFile(recoveryPath, 'utf8')).toBe('foreign work retained after exchange');
    expect(await readFile(destination)).toEqual(document('first'));
  });
  it('cleans its own staging file after an ordinary prepublication failure and permits retry', async () => {
    vi.mocked(publishFile).mockRejectedValueOnce(new Error('publication failed'));
    await expect(files.save('one', document('second'))).rejects.toThrow('publication failed');
    expect((await readdir(directory)).filter(name => name.endsWith('.tmp'))).toEqual([]);
    await files.save('one', document('retry'));
    expect(await readFile(destination)).toEqual(document('retry'));
  });
});
