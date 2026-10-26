import * as fs from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { createStore } from './storage';
const folders: string[]=[];
afterEach(async()=>{vi.useRealTimers();for(const folder of folders.splice(0)) await fs.rm(folder,{recursive:true,force:true});});
it('starts durable compact-journal writes within one second during ongoing edits',async()=>{
  const folder=await fs.mkdtemp(path.join(tmpdir(),'powermove-journal-bound-'));folders.push(folder);
  const writeFile=vi.fn(fs.writeFile);const store=createStore(folder,{fs:{...fs,writeFile} as typeof fs});await store.load();
  vi.useFakeTimers();
  for(let revision=1;revision<=11;revision++){store.set('projectJournal.synthetic',[{sequence:revision,patches:[]}]);await vi.advanceTimersByTimeAsync(100);}
  expect(writeFile).toHaveBeenCalled();await store.flushAll({strict:true});
  vi.useRealTimers();const reopened=createStore(folder);await reopened.load();expect(reopened.get!('projectJournal.synthetic')).toEqual([{sequence:11,patches:[]}]);
});
it.each(['ENOSPC','EACCES'])('retains old disk data and failed edits after %s, then retries the retained operation',async code=>{
  const folder=await fs.mkdtemp(path.join(tmpdir(),'powermove-journal-fault-'));folders.push(folder);
  let fail=false;const writeFile:typeof fs.writeFile=async(...args)=>{if(fail)throw Object.assign(new Error(code),{code});return fs.writeFile(...args);};
  const store=createStore(folder,{fs:{...fs,writeFile}});await store.load();const previous=[{sequence:1,patches:[]}],next=[{sequence:2,patches:[]}];
  store.set('projectJournal.synthetic',previous);await store.flushAll({strict:true});
  fail=true;store.set('projectJournal.synthetic',next);await expect(store.flushAll({strict:true})).rejects.toThrow();
  const old=createStore(folder);await old.load();expect(old.get!('projectJournal.synthetic')).toEqual(previous);expect(store.get!('projectJournal.synthetic')).toEqual(next);
  fail=false;await store.flushAll({strict:true});const restored=createStore(folder);await restored.load();expect(restored.get!('projectJournal.synthetic')).toEqual(next);
  expect((await fs.readdir(folder)).some(name=>name.includes('.tmp-'))).toBe(false);
});
