import{tag,evaluate}from './main-oracle.mjs';
import fs from'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const types=tag('apps/desktop/src/renderer/src/core/types/project.ts');
const kinds=evaluate(types.match(/export const MEMORY_KINDS = [^;]+;/)[0],['MEMORY_KINDS']);
const api=evaluate(tag('apps/desktop/src/renderer/src/core/memory.ts'),['sanitizeMemory','memoryIndex','listMemory','applyMemoryOperation'],kinds);
try{
 const result=input.operation==='sanitize'?api.sanitizeMemory(input.entries,input.now):input.operation==='index'?api.memoryIndex(input.entries):input.operation==='list'?api.listMemory(input.entries,input.arguments):api.applyMemoryOperation(input.entries,input.arguments,{now:input.now,uid:()=>input.id,author:'agent'});
 process.stdout.write(JSON.stringify({result}));
}catch(e){process.stdout.write(JSON.stringify({error:e.message}));}
