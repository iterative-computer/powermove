import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { expect, test, launchApp } from './helpers/app';
import { decodeProjectContainer } from '../src/shared/project-container';
const evidence = process.env.POWERMOVE_AUTOSAVE_EVIDENCE || path.join(tmpdir(), 'powermove-autosave-evidence');
const savedName = async (file: string) => { try { return decodeProjectContainer(await readFile(file)).document.proj.name; } catch { return null; } };

test('untitled recovery, named background save, conflicts, retry and safe version copy', async ({ session }) => {
  test.setTimeout(90000); await mkdir(evidence,{recursive:true}); await session.openEditor();
  const cdp = await session.page.context().newCDPSession(session.page);
  const frames: Array<{file:string;at:number}> = [], writes: Promise<void>[] = [];
  cdp.on('Page.screencastFrame', frame => {
    const file = path.join(evidence, `frame-${String(frames.length).padStart(5,'0')}.jpg`);
    frames.push({file,at:frame.metadata.timestamp}); writes.push(writeFile(file,Buffer.from(frame.data,'base64')));
    void cdp.send('Page.screencastFrameAck',{sessionId:frame.sessionId});
  });
  await cdp.send('Page.startScreencast',{format:'jpeg',quality:70,maxWidth:1440,maxHeight:900,everyNthFrame:2});
  const first = await session.page.evaluate(() => {
    const PM=(window as any).PM; PM.hist.do('Name draft',()=>{PM.proj.name='Synthetic draft';}); return PM.proj.id;
  });
  await expect.poll(()=>savedName(path.join(session.userData,'recovery/documents',first+'.pmv'))).toBe('Synthetic draft');
  expect(await session.app.evaluate(()=> (globalThis as any).__saveDialogs || 0)).toBe(0);
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.app.saveStatus)).toBe('SAVED LOCALLY');
  await session.page.screenshot({path:path.join(evidence,'01-draft-recovered.png')});
  const file=path.join(session.userData,'Synthetic.pmv');
  await session.app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>{(globalThis as any).__saveDialogs=((globalThis as any).__saveDialogs||0)+1;return {canceled:false,filePath:file};};},file);
  await session.page.keyboard.press('Meta+S');
  await expect.poll(()=>savedName(file)).toBe('Synthetic draft');
  await session.page.evaluate(()=>{const PM=(window as any).PM;PM.hist.do('Agent edit',()=>{PM.proj.name='Background saved';},'agent');});
  await expect.poll(()=>savedName(file)).toBe('Background saved');
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.app.saveStatus)).toBe('SAVED');
  await session.page.screenshot({path:path.join(evidence,'02-background-saved.png')});
  await session.app.evaluate(({dialog})=>{dialog.showMessageBox=async()=>({response:0,checkboxChecked:false});});
  expect(await session.page.evaluate(async()=>{const PM=(window as any).PM;PM.hist.do('Close flush',()=>{PM.proj.name='Close flush';});const prepared=await PM.prepareToClose({id:'synthetic-close',flush:true});PM.cancelClosePreparation();return prepared;})).toBe(true);
  expect(await savedName(file)).toBe('Close flush');
  await writeFile(file,JSON.stringify({proj:{id:'external',name:'External work',w:100,h:100,layers:[]}}));
  await session.page.evaluate(()=>{const PM=(window as any).PM;PM.hist.do('Newer local edit',()=>{PM.proj.name='Local conflict';});});
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.app.saveStatus)).toBe('SAVE FAILED');
  expect(await savedName(file)).toBe('External work');
  await session.page.screenshot({path:path.join(evidence,'03-conflict-preserved.png')});
  const copy=path.join(session.userData,'Conflict copy.pmv');
  await session.app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});},copy);
  expect(await session.page.evaluate(()=>(window as any).PM.saveProject({saveAs:true}))).toBe(true);
  expect(await savedName(copy)).toBe('Local conflict');
  await session.page.evaluate(()=>(window as any).PM.projectVersionHistory());
  await session.page.getByText('Open Copy',{exact:false}).first().waitFor();
  await session.page.screenshot({path:path.join(evidence,'04-version-history.png')});
  await session.page.evaluate(() => (window as any).PM.theme.apply('dark'));
  expect(await session.page.evaluate(()=>document.documentElement.dataset.theme)).toBe('dark');
  await expect.poll(() => session.page.getByText('Open Copy',{exact:false}).first().evaluate(node => getComputedStyle(node).color === getComputedStyle(node.closest('.modal')!).color)).toBe(true);
  await session.page.screenshot({path:path.join(evidence,'04b-version-history-dark.png')});
  await session.page.getByText('Open Copy',{exact:false}).first().click();
  await expect.poll(()=>session.page.evaluate(()=>(window as any).PM.proj.name)).toContain('recovered copy');
  expect(await session.page.evaluate(()=>(window as any).PM.projectFileState().path)).toBeUndefined();
  expect(await savedName(file)).toBe('External work');expect(await savedName(copy)).toBe('Local conflict');
  await cdp.send('Page.stopScreencast'); await Promise.all(writes); await writeFile(path.join(evidence,'frames.json'),JSON.stringify(frames));
  await writeFile(path.join(evidence,'workflow-results.json'),JSON.stringify({untitledDialogs:0,externalPreserved:true,restoreCopy:true},null,2));
});

test('two heavy projects retain pending writes through tab switches, undo, redo and crash', async ({session})=>{
  test.setTimeout(180000);await session.openEditor();
  const create=async(name:string,mebibytes:number)=>session.page.evaluate(async({name,mebibytes})=>{
    const PM=(window as any).PM; PM.newBlankProject({name});
    const cv=document.createElement('canvas');cv.width=cv.height=8;cv.getContext('2d')!.fillRect(0,0,8,8);
    const png=await new Promise<Blob>(r=>cv.toBlob(b=>r(b!)));const file=new File([png,...Array(mebibytes).fill(new Uint8Array(1024*1024))],name+'.png',{type:'image/png'});
    await PM.importFiles([file]);await PM.app.importQueue;return {id:PM.proj.id,bytes:file.size,sum:[...new Uint8Array(await png.arrayBuffer())].reduce((sum,byte)=>(sum+byte)>>>0,0)};
  },{name,mebibytes});
  const a=await create('Heavy A',53),b=await create('Heavy B',89);
  await session.page.evaluate(()=> (window as any).PM.flushProject());
  await session.page.evaluate(()=>{let last=performance.now(),max=0;(window as any).__autosaveHeartbeat={gap:()=>max};setInterval(()=>{const now=performance.now();max=Math.max(max,now-last);last=now;},10);});
  // Pause this fixture's durable PMV publication, leaving ordinary journal flushes available.
  await session.app.evaluate(async (_electron, nativePath)=>{
    const native=process.getBuiltinModule('node:module').createRequire(nativePath+'/index.cjs')(nativePath);const fs=process.getBuiltinModule('node:fs');
    const sync=native.fullSync;let release!:()=>void,started=false;const gate=new Promise<void>(r=>release=r);
    native.fullSync=async(fd:number)=>{if(!started&&fs.fstatSync(fd).size>50*1024*1024){started=true;await gate;}return sync(fd);};
    (globalThis as any).__autosaveBarrier={release,started:()=>started,restore:()=>native.fullSync=sync};
  }, path.resolve(__dirname, '../../../packages/macos-haptics'));
  await session.page.evaluate(async a=>{const PM=(window as any).PM;await PM.Tabs.activate(a);PM.hist.do('Agent A',()=>{PM.proj.name='Heavy A agent';},'agent');},a.id);
  await expect.poll(()=>session.app.evaluate(()=>(globalThis as any).__autosaveBarrier.started())).toBe(true);
  await session.page.evaluate(async b=>{const PM=(window as any).PM;await PM.Tabs.activate(b);PM.hist.do('Agent B',()=>{PM.proj.name='Heavy B agent';},'agent');PM.hist.undo();PM.hist.redo();},b.id);
  await session.app.evaluate(()=>(globalThis as any).__autosaveBarrier.release());
  await expect.poll(()=>savedName(path.join(session.userData,'recovery/documents',a.id+'.pmv')),{timeout:60000}).toBe('Heavy A agent');
  await expect.poll(()=>savedName(path.join(session.userData,'recovery/documents',b.id+'.pmv')),{timeout:60000}).toBe('Heavy B agent');
  await session.app.evaluate(()=>(globalThis as any).__autosaveBarrier.restore());
  const heartbeatGapMs=await session.page.evaluate(()=>(window as any).__autosaveHeartbeat.gap());
  await session.page.evaluate(async()=>{const PM=(window as any).PM;PM.hist.do('Crash journal',()=>{PM.proj.name='Recovered from journal';},'agent');await PM.store.flush();});
  // Abruptly kill only this task-owned fixture, without invoking its close flush.
  const child=session.app.process(), exited=new Promise<void>(resolve=>child.once('exit',()=>resolve()));
  child.kill('SIGKILL'); await exited;
  const recovered=await launchApp({userData:session.userData});
  try {
    await recovered.openEditor();
    await recovered.page.evaluate(async id=>{const PM=(window as any).PM;if(PM.proj.id!==id)await PM.openProjectHere(id);},b.id);
    await expect.poll(()=>recovered.page.evaluate(()=>(window as any).PM.proj.name)).toBe('Recovered from journal');
    const restoredBytes=await recovered.page.evaluate(async()=>{const PM=(window as any).PM;const asset=Object.values(PM.proj.assets)[0],saved=await PM.MediaStore.getForSave(asset),source=saved.source;let bytes=0,sum=0;for(let offset=0;offset<source.size;offset+=1024*1024){const part=await source.read(offset,Math.min(1024*1024,source.size-offset));bytes+=part.length;for(const byte of part)sum=(sum+byte)>>>0;}return {bytes,sum,sha256:source.sha256};});
    expect(restoredBytes.bytes).toBe(b.bytes);
    expect(restoredBytes.sum).toBe(b.sum);
    expect(restoredBytes.sha256).toMatch(/^[a-f0-9]{64}$/);
    await mkdir(evidence,{recursive:true});await recovered.page.screenshot({path:path.join(evidence,'05-heavy-crash-recovery.png')});
    await writeFile(path.join(evidence,'heavy-results.json'),JSON.stringify({a,b,restoredBytes,heartbeatGapMs,crashJournalRecovered:true,abruptProcessKill:true},null,2));
  } finally { await recovered.close(); }
});
