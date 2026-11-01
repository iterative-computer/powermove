import {test,expect} from 'bun:test';
import {readFile,access} from 'node:fs/promises';
import path from 'node:path';
const ref=process.env.PM_TRACE_REF??'/tmp/pm-ref-v1.2.0';
let available=true;try{await access(path.join(ref,'apps/desktop/scripts/trace-recorder/transform.mjs'));}catch{available=false;}
(available ? test : test.skip)('overlay parses every target, preserves parameter lists, and disappears when flag is off',async()=>{
  const {targets,transform,traceRecorderPlugin,traceRecorderMainPlugin}=await import(path.join(ref,'apps/desktop/scripts/trace-recorder/transform.mjs'));
  const ts=(await import(path.join(ref,'apps/desktop/node_modules/typescript/lib/typescript.js'))).default;
  const saved=process.env.POWERMOVE_TRACE_RECORDER;
  try{
    delete process.env.POWERMOVE_TRACE_RECORDER;
    for(const file of targets){
      const id=path.join(ref,'apps/desktop/src/renderer/src',file),source=await readFile(id,'utf8');
      expect(traceRecorderPlugin().transform(source,id)).toBeUndefined();
      const output=transform(source,file);
      const parsed=ts.createSourceFile(file,output,ts.ScriptTarget.Latest,true);
      expect(parsed.parseDiagnostics).toEqual([]);
      const before:any[]=[],after:any[]=[];
      const functions=(node:any,list:any[])=>{if(ts.isFunctionLike(node)&&node.body) list.push(node.parameters.map((p:any)=>p.getText()).join(','));ts.forEachChild(node,(n:any)=>functions(n,list));};
      functions(ts.createSourceFile(file,source,ts.ScriptTarget.Latest,true),before);functions(parsed,after);
      // Added observer thunks have no parameters; original callable signatures survive in order.
      let position=0;for(const params of before){while(position<after.length&&after[position]!==params)position++;expect(position++).toBeLessThan(after.length);}
      expect(output).toContain('__pmTrace');
    }
    const id=path.join(ref,'apps/desktop/src/main/menu.ts'),source=await readFile(id,'utf8');
    expect(traceRecorderMainPlugin().transform(source,id)).toBeUndefined();
    process.env.POWERMOVE_TRACE_RECORDER='1';
    expect(traceRecorderMainPlugin().transform(source,id).code).toContain('menu.routing');
    expect(transform(await readFile(path.join(ref,'apps/desktop/src/renderer/src/kernel/install.ts'),'utf8'),'kernel/install.ts')).toContain('api.workspace.mutate');
  }finally{if(saved===undefined)delete process.env.POWERMOVE_TRACE_RECORDER;else process.env.POWERMOVE_TRACE_RECORDER=saved;}
});
