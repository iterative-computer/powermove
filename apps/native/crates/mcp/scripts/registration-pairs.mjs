import{tag,evaluate}from './main-oracle.mjs';
import fs from'node:fs';
import path from'node:path';
import{randomUUID}from'node:crypto';
const input=JSON.parse(fs.readFileSync(0,'utf8'));const root=path.resolve(input.root);
const checked=file=>{if(file==='/Applications'||file.startsWith('/Applications/'))file=path.join(root,'applications',file.slice('/Applications'.length));if(file!==root&&!file.startsWith(root+'/'))throw Object.assign(Error('Outside fixture'),{code:'ENOENT'});return file};
const names=['chmodSync','copyFileSync','existsSync','lstatSync','mkdirSync','readFileSync','renameSync','unlinkSync','writeFileSync'];
const bindings=Object.fromEntries(names.map(name=>[name,(file,...args)=>fs[name](checked(file),...(['copyFileSync','renameSync'].includes(name)?[checked(args[0]),...args.slice(1)]:args))]));
const api=evaluate(tag('apps/desktop/src/main/agent-clients.ts'),['listAgentClients','connectAgentClient','disconnectAgentClient'],{...bindings,path,randomUUID,constants:fs.constants});
const calls=[];const env={homeDir:path.join(root,'home'),userDataDir:path.join(root,'profile'),executablePath:input.executable??'/Applications/Powermove.app/Contents/MacOS/Powermove',runClaude:async args=>{calls.push(args);return{code:input.cliCode??0,output:input.cliOutput??''}}};
try{let result=input.action==='inspect'?api.listAgentClients(env):input.action==='connect'?await api.connectAgentClient(input.client,env):await api.disconnectAgentClient(input.client,env);process.stdout.write(JSON.stringify({result:result??null,calls}));}
catch(e){process.stdout.write(JSON.stringify({error:e.message,calls}));}
