// File-only reference; every path is supplied by an isolated synthetic fixture.
import {tag,evaluate} from './tag-oracle.mjs';
import fs from 'node:fs/promises';
import {readFileSync} from 'node:fs';
import path from 'node:path';
import {randomBytes} from 'node:crypto';
const input=JSON.parse(readFileSync(0,'utf8'));
const source=tag('packages/registry/src/manifest.ts');
const {parseManifest,EXTENSION_ID}=evaluate(source,['parseManifest','EXTENSION_ID'],{URL});
if(input.operation==='manifest') process.stdout.write(JSON.stringify({valid:parseManifest(input.manifest).ok}));
else {
  const {forkBuiltinExtension}=evaluate(tag('apps/desktop/src/main/extensions/fork.ts'),['forkBuiltinExtension'],{fs,path,randomBytes,parseManifest,EXTENSION_ID});
  try {process.stdout.write(JSON.stringify({result:await forkBuiltinExtension(input.options)}));}
  catch(e){process.stdout.write(JSON.stringify({error:e.message}));}
}
