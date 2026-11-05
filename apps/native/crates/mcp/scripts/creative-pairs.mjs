// Execute the actual frozen reader against an isolated synthetic filesystem.
// The only path seam maps Adobe's fixed system CEP root into the test root;
// paths outside that root are never read on the machine running this oracle.
import {tag,evaluate} from './tag-oracle.mjs';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const root=path.resolve(input.root??'/synthetic-unused-root');
const checked=file=>{
  if(file==='/Library/Application Support/Adobe/CEP/extensions'||file.startsWith('/Library/Application Support/Adobe/CEP/extensions/'))file=path.join(root,'system-cep',file.slice('/Library/Application Support/Adobe/CEP/extensions'.length));
  if(file!==root&&!file.startsWith(root+'/'))throw Object.assign(Error('Outside synthetic fixture'),{code:'ENOENT'});
  return file;
};
const api=evaluate(tag('apps/desktop/src/main/creative-workspace.ts'),['parseAfterEffectsWorkspace','inspectCreativeWorkspace','inspectCreativeExtension'],{
  lstat:async file=>fsp.lstat(checked(file)),readFile:async(file,...args)=>fsp.readFile(checked(file),...args),readdir:async(file,...args)=>fsp.readdir(checked(file),...args),
  createHash:algorithm=>{
    const hash=createHash(algorithm);
    return {update:file=>{hash.update(checked(file));return {digest:encoding=>hash.digest(encoding)}}};
  },path,os:{homedir:()=>{throw Error('No implicit home in creative oracle')}},process:{platform:'linux',env:{}},Buffer
});
try{
  const result=input.source!==undefined?api.parseAfterEffectsWorkspace(input.source):await api[input.tool==='inspect_creative_workspace'?'inspectCreativeWorkspace':'inspectCreativeExtension'](input.arguments??{}, {platform:input.platform??'darwin',home:path.join(root,'home'),applications:path.join(root,'applications')});
  process.stdout.write(JSON.stringify({result}));
}catch(e){process.stdout.write(JSON.stringify({error:e.message}));}
