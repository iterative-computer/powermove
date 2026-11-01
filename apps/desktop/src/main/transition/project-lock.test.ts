import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { ProjectFiles } from '../project-files';
import { TransitionLock, projectLockPath } from './lock';
let root: string, files: ProjectFiles[];
beforeEach(async () => {
  root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-project-')); files = [];
  const binary = path.join(root, 'helper'); execFileSync('cc', ['-std=c11', path.resolve('native/transition-lock.c'), '-lm', '-o', binary]);
  TransitionLock.configure(binary);
});
afterEach(async () => { for (const file of files) await file.dispose(); await rm(root, { recursive: true, force: true }); });
function registry(name: string) {
  const file = new ProjectFiles(path.join(root, name, 'registry.json'), path.join(root, name, 'backups'), { transitionLocks: true });
  files.push(file); return file;
}
const document = (name: string) => Buffer.from(JSON.stringify({ proj: { id: 'synthetic', name, layers: [], w: 100, h: 100 } }));
it('locks an external project for its full session; a competing editor opens a detached copy', async () => {
  const a = registry('a'), b = registry('b'), destination = path.join(root, 'project.pmv');
  await a.save('one', document('original'), destination);
  await expect(b.save('one', document('competing'), destination)).rejects.toThrow('locked');
  const opened = await b.open(destination);
  expect(opened.copy).toBe(true); expect(await b.destination(opened.projectId)).toBeUndefined();
  await b.close(opened.token);
  expect((await readFile(destination)).toString()).toBe(document('original').toString());
  await a.releaseProject('one');
  expect(JSON.parse(await readFile(projectLockPath(destination), 'utf8')).pid).toBe(0);
  await b.save('two', document('after-close'), destination);
});
it('does not create a missing project destination directory while acquiring its lock', async () => {
  const files = registry('writer'), destination = path.join(root, 'missing-parent', 'project.pmv');
  await expect(files.save('one', document('unsaved'), destination)).rejects.toThrow();
  await expect(stat(path.dirname(destination))).rejects.toMatchObject({ code: 'ENOENT' });
  expect(await files.destination('one')).toBeUndefined();
  await files.save('one', document('retry'), path.join(root, 'valid.pmv'));
  expect(await readFile(path.join(root, 'valid.pmv'))).toEqual(document('retry'));
});
