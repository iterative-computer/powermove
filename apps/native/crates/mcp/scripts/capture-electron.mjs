// Safe transport-only probe. Never launches an editor, opens a project, or
// reads the real connection descriptor. The fixture host has no project tools.
import { spawn } from 'node:child_process';
import net from 'node:net';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { catalog } from './tag-oracle.mjs';
const executable = '/Applications/Powermove.app/Contents/MacOS/Powermove';
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'pm-mcp-capture-'));
const token = randomBytes(32).toString('hex');
const sockets = new Set();
const server = net.createServer(socket => {
  sockets.add(socket);socket.on('close',()=>sockets.delete(socket));
  socket.setEncoding('utf8');let buffer='',authorized=false;
  socket.on('data', data => {
    buffer += data;
    while (buffer.includes('\n')) {
      const end=buffer.indexOf('\n');const m=JSON.parse(buffer.slice(0,end));buffer=buffer.slice(end+1);
      if (!authorized) {if(m.method!=='connect'||m.token!==token){socket.destroy();return;}authorized=true;socket.write(JSON.stringify({id:m.id,result:{connected:true}})+'\n');}
      else if(m.method==='tools/list')socket.write(JSON.stringify({id:m.id,result:catalog.tools})+'\n');
      else socket.write(JSON.stringify({id:m.id,error:'Read-only synthetic host: project calls are disabled.'})+'\n');
    }
  });
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let child;
try {
  await fs.mkdir(path.join(root,'mcp'),{mode:0o700});
  await fs.writeFile(path.join(root,'mcp/connection.json'),JSON.stringify({version:1,instanceId:'synthetic-capture',pid:process.pid,port:server.address().port,token}),{mode:0o600});
  child=spawn(executable,['--powermove-mcp'],{env:{...process.env,POWERMOVE_USER_DATA:root},stdio:['pipe','pipe','pipe']});
  let output='',diagnostics='';const pending=new Map();let next=0;
  child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
  child.stderr.on('data',data=>{diagnostics+=data;});
  child.stdout.on('data',data=>{output+=data;while(output.includes('\n')){const end=output.indexOf('\n');const line=output.slice(0,end);output=output.slice(end+1);try{const m=JSON.parse(line);pending.get(m.id)?.(m);pending.delete(m.id);}catch{}}});
  const pairs=[];
  const rpc=async(method,params)=>{const request={jsonrpc:'2.0',id:++next,method,...(params?{params}:{})};const response=await new Promise((resolve,reject)=>{const timeout=setTimeout(()=>reject(Error('Packaged app did not answer safely within 10 seconds.')),10000);pending.set(request.id,m=>{clearTimeout(timeout);resolve(m);});child.stdin.write(JSON.stringify(request)+'\n');});pairs.push({request,response});return response;};
  await rpc('tools/list');
  await rpc('initialize',{protocolVersion:'2024-11-05'});
  await rpc('initialize');
  await rpc('ping');
  await rpc('tools/call',{name:123,arguments:{}});
  await rpc('unknown');
  await rpc('tools/list');
  const target=process.argv[2];
  // Catalog replies are sourced from the synthetic host, not real app bodies.
  // Do not represent them as live project-tool evidence.
  const evidence={kind:'installed-app-stdio-to-synthetic-read-only-host',projectToolsCaptured:false,pairs};
  if(target)await fs.writeFile(target,JSON.stringify(evidence,null,2)+'\n');
  console.log(`Captured ${pairs.length} transport pairs; no editor or project was opened. Tool catalog came from frozen source.`);
} finally {
  child?.stdin.end();child?.kill();for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));await fs.rm(root,{recursive:true,force:true});
}
