import {test,expect} from 'bun:test';
import {readFile,readdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {scenarios} from '../scenarios';
import {validateTrace} from '../validate';
const root=path.resolve(import.meta.dir,'..');
test('synthetic goldens are complete, balanced, current and cover write/key/menu paths',async()=>{
  const files=(await readdir(path.join(root,'golden'))).filter(x=>x.endsWith('.jsonl')).sort();
  expect(files).toEqual(scenarios.map(x=>`${x}.jsonl`).sort());
  const hash=createHash('sha256'),adapter=createHash('sha256');
  for(const file of ['core.ts','overlay/runtime.ts','overlay/transform.mjs','scenarios.ts']){
    const contents=await readFile(path.join(root,file));hash.update(contents);if(file!=='scenarios.ts')adapter.update(contents);
  }
  const expected=hash.digest('hex'),expectedAdapter=adapter.digest('hex'),calls=new Set<string>();
  for(const file of files){
    const text=await readFile(path.join(root,'golden',file),'utf8');
    expect(validateTrace(text).calls).toBeGreaterThan(0);
    const rows=text.trim().split('\n').map(x=>JSON.parse(x)),header=rows[0];
    expect(header.synthetic).toBe(true);expect(header.scenario+'.jsonl').toBe(file);
    expect(header.reference.revision).toBe('07ed5fa17ee640659cb075f9cad1af2095011d60');
    expect(header.recorder).toBe(expected);expect(header.adapter).toBe(expectedAdapter);
    expect(header.eventScope).toBe('synchronous-calls');
    for(const row of rows)if(row.type==='call')calls.add(row.name);
    if(file==='workspace-direct.jsonl'){
      const failure=rows.find(x=>x.type==='throw'&&x.state?.props.workspace);
      expect(failure.state.props.workspace.props.name).toBe('Partial synthetic workspace');
    }
    if(file==='menu-equivalents.jsonl'){
      expect(rows.filter(x=>x.type==='menu.received').length).toBe(2);
      const main=rows.find(x=>x.type==='main.observations').data.props.observations.values;
      expect(main.map((x:any)=>x.props.type)).toEqual(['menu.send','menu.send','menu.routing','menu.routing']);
      expect(main[2].props.ignoreMenuShortcuts).toBe(true);
      expect(rows.some(x=>x.type==='dom.capture'&&x.data.trusted===true)).toBe(true);
    }
  }
  for(const name of ['history.do','history.begin','history.commit','history.cancel','history.external','history.undo','history.redo',
    'edit.mutate','edit.begin','edit.commit','edit.cancel','gesture.once','workspace.mutate','PM.proj.direct','PM.touch','keymap.bubble','keymap.resolve','command.run'])expect(calls.has(name)).toBe(true);
});
