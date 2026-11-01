import { readFile, writeFile, mkdir, mkdtemp } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { scenarios, runScenario } from './scenarios.ts';
import { validateTrace } from './validate.ts';
if(process.versions.bun) throw new Error('Run this Playwright driver with Node, not Bun: node tools/trace-recorder/run.ts');
const args=process.argv.slice(2);
const option=(key:string,fallback:string)=>args.includes(key) ? args[args.indexOf(key)+1]! : fallback;
const ref=path.resolve(option('--ref','/tmp/pm-ref-v1.2.0'));
const root=import.meta.dirname;
const golden=path.join(root,'golden');
const output=path.resolve(option('--out',await mkdtemp(path.join(os.tmpdir(),'pm-traces-'))));
const update=args.includes('--update');
if(args.includes('--all-events')&&(update||args.includes('--check'))) throw new Error('All-events captures are diagnostic, not deterministic goldens');
const selected=option('--scenario','').split(',').filter(Boolean);
const names=selected.length ? selected : scenarios;
for(const name of names) if(!scenarios.includes(name)) throw new Error(`Unknown synthetic scenario: ${name}`);
const sources=['core.ts','overlay/runtime.ts','overlay/transform.mjs','scenarios.ts'];
const hash=createHash('sha256'),adapter=createHash('sha256');for(const file of sources){const contents=await readFile(path.join(root,file));hash.update(contents);if(file!=='scenarios.ts')adapter.update(contents);}
const metadata={reference:{revision:execFileSync('git',['-C',ref,'rev-parse','HEAD'],{encoding:'utf8'}).trim(), requestedRef:option('--reference-label','v1.2.0')}, recorder:hash.digest('hex'),adapter:adapter.digest('hex'), driver:'playwright-electron-hidden', input:'synthetic-dom',eventScope:args.includes('--all-events') ? 'all' : 'synchronous-calls'};
delete process.env.ELECTRON_RUN_AS_NODE;
delete process.env.POWERMOVE_E2E_LIVE;
process.env.POWERMOVE_TRACE_RECORDER='1';
const {launchApp,ensureBuilt}=await import(path.join(ref,'apps/desktop/e2e/helpers/app.trace.mjs'));
if(!args.includes('--skip-build')) await ensureBuilt(true);
if(update) await mkdir(golden,{recursive:true});
await mkdir(output,{recursive:true});
const session=await launchApp({env:{POWERMOVE_TRACE_RECORDER:'1'}});
try {
  const installed=await session.page.evaluate(()=>(window as any).__PMTrace?.buildFingerprint);
  if(installed!==metadata.adapter)throw new Error('Stale or disabled instrumentation: apply the overlay and rebuild before capture');
  await session.page.evaluate(async()=>{await (window as any).PM.Kernel.loader.builtinsReady;});
  const capture=async (name:string)=>{
    await session.app.evaluate(()=>{(globalThis as any).__PMTraceMain.length=0;});
    let text=await session.page.evaluate(runScenario,{name,metadata});
    if(name==='menu-equivalents') {
      await session.app.evaluate(({Menu}:any)=>{
        const menu=Menu.getApplicationMenu();const item=menu?.getMenuItemById('contextUndo');
        if(!item) throw new Error('Missing reference Undo menu');
        item.click(item,undefined,{});
      });
      await session.page.waitForFunction(()=>(window as any).PM.proj.name==='Synthetic trace fixture',{},{timeout:10000});
      await session.page.evaluate(()=>{(window as any).__PMTrace.point('driver.menu',{id:'contextRedo',equivalent:'CommandOrControl+Shift+Z'});});
      await session.app.evaluate(({Menu}:any)=>{const item=Menu.getApplicationMenu().getMenuItemById('contextRedo');item.click(item,undefined,{});});
      await session.page.waitForFunction(()=>(window as any).PM.proj.name==='menu edit',{},{timeout:10000});
      await session.page.evaluate(()=>{(window as any).__PMTrace.point('driver.key',{chord:'cmd+z',source:'webContents.sendInputEvent'});});
      await session.app.evaluate(({webContents}:any)=>{
        const target=webContents.getAllWebContents().find((wc:any)=>wc.getURL().startsWith('app:'));
        if(!target)throw new Error('Missing renderer');
        target.sendInputEvent({type:'keyDown',keyCode:'Z',modifiers:['meta']});
        target.sendInputEvent({type:'keyUp',keyCode:'Z',modifiers:['meta']});
      });
      await session.page.waitForFunction(()=>(window as any).PM.proj.name==='Synthetic trace fixture',{},{timeout:10000});
      const main=await session.app.evaluate(()=>(globalThis as any).__PMTraceMain);
      text=await session.page.evaluate((main:any)=>{
        const trace=(window as any).__PMTrace;
        trace.point('main.observations',{observations:main});
        return trace.finish();
      },main);
    }
    if(name==='history-async') text=await session.page.evaluate(()=>{const PM=(window as any).PM;if(PM.proj.name!=='asynchronous part')throw new Error('Missing async continuation');return (window as any).__PMTrace.finish();});
    return text!;
  };
  const captured=new Map<string,string>();
  for(const name of names) {
    const first=await capture(name);
    validateTrace(first);
    const second=await capture(name);
    validateTrace(second);
    await writeFile(path.join(output,`${name}.jsonl`),first);
    if(!args.includes('--all-events')&&first!==second) {
      await writeFile(path.join(output,`${name}.repeat.jsonl`),second);
      throw new Error(`Nondeterministic trace: ${name}; inspect ${output}`);
    }
    if(update) captured.set(name,first);
    else if(args.includes('--check')) {
      const expected=await readFile(path.join(golden,`${name}.jsonl`),'utf8');
      if(expected!==first) throw new Error(`Golden mismatch: ${name}; inspect ${output}`);
    }
    console.log(`${name}: ${first.trim().split('\n').length} rows, ${args.includes('--all-events') ? 'two diagnostic runs' : 'two identical runs'}`);
  }
  for(const [name,text] of captured) await writeFile(path.join(golden,`${name}.jsonl`),text);
  console.log(`Traces: ${output}`);
} finally {await session.close();}
