// Optional independent writer oracle. Run with an Electron executable:
//   Electron tests/generate-electron.cjs /tmp/pm-s6-synthetic-profile
// Only a NEW private /tmp directory is accepted. No installed app/profile opens.
const {app, BrowserWindow, protocol} = require('electron');
const fs = require('node:fs');
const path = require('node:path');
const profile = process.argv[2];
if (!profile || !path.resolve(profile).startsWith('/tmp/pm-s6-synthetic-') || fs.existsSync(profile)) {
  throw new Error('a fresh /tmp/pm-s6-synthetic-* profile is required');
}
app.setPath('userData', profile);
app.disableHardwareAcceleration();
protocol.registerSchemesAsPrivileged([{scheme:'app', privileges:{standard:true,secure:true,supportFetchAPI:true}}]);
app.whenReady().then(async () => {
  protocol.handle('app', () => new Response('<!doctype html><title>Synthetic migration fixture</title>'));
  const w = new BrowserWindow({show:false,webPreferences:{nodeIntegration:true,contextIsolation:false}});
  await w.loadURL('app://powermove');
  const counts = await w.webContents.executeJavaScript(`(async () => {
    const bytes = new TextEncoder().encode('S6 synthetic original bytes\\n');
    const fingerprint = async b => {
      const sample=Math.min(65536,b.length), offsets=[...new Set([0,Math.floor((b.length-sample)/2),b.length-sample])];
      const chunks=[new TextEncoder().encode('powermove-media-v2\\n'+b.length+'\\n'),...offsets.map(n=>b.slice(n,n+sample))];
      const input=new Uint8Array(chunks.reduce((s,c)=>s+c.length,0));let at=0;for(const c of chunks){input.set(c,at);at+=c.length;}
      const h=await crypto.subtle.digest('SHA-256',input);
      return 'v2:'+b.length+':'+Array.from(new Uint8Array(h),b=>b.toString(16).padStart(2,'0')).join('');
    };
    const fp=await fingerprint(bytes);
    const db=await new Promise((resolve,reject)=>{const r=indexedDB.open('powermove-media',2);r.onupgradeneeded=()=>r.result.createObjectStore('assets',{keyPath:'id'});r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
    const records=[
      {id:'media:synthetic',blob:new Blob([bytes],{type:'video/mp4'}),fingerprint:fp},
      {id:'media:synthetic:poster',blob:new Blob(['synthetic poster'],{type:'image/png'}),fingerprint:null},
      {id:'old-bare-id',blob:new Blob(['old original'],{type:'image/png'}),fingerprint:null},
      {id:'media:proxy',blob:new Blob(['synthetic proxy'],{type:'video/webm'}),fingerprint:fp},
      {id:'media:file',blob:new File(['synthetic file'],'synthetic.txt',{type:'text/plain',lastModified:1000}),fingerprint:null},
    ];
    await new Promise((resolve,reject)=>{const t=db.transaction('assets','readwrite');for(const r of records)t.objectStore('assets').put({...r,revision:'synthetic-revision',size:r.blob.size,type:r.blob.type,at:1,unknown:'café ☃ 🚀'});t.oncomplete=resolve;t.onabort=()=>reject(t.error);});
    localStorage.setItem('pm.easing.library.v1',JSON.stringify({version:1,groups:[],active:null,preferences:{}}));
    localStorage.setItem('pm.fx.recent','["blur"]');
    localStorage.setItem('pm.theme','"dark"');
    localStorage.setItem('powermove:native-evaluator','audit');
    localStorage.setItem('synthetic.extension.é','café ☃ 🚀');
    db.close();return {records:records.length,bytes:records.reduce((n,r)=>n+r.blob.size,0),local:localStorage.length};
  })()`);
  // If provided, prove a coherent snapshot while THIS synthetic Electron runs.
  if (process.env.PM_S6_READER) {
    const cp=require('node:child_process');
    const idb=path.join(profile,'IndexedDB');
    const name=fs.readdirSync(idb).find(n=>n.endsWith('.leveldb'));
    const dest=profile+'-live-snapshot';
    const args=['snapshot-media',path.join(idb,name),path.join(idb,name.replace('.leveldb','.blob')),dest];
    const r=cp.spawnSync(process.env.PM_S6_READER,args,{encoding:'utf8'});
    if(r.status!==0)throw new Error('synthetic live snapshot failed: '+r.stderr);
    console.log(r.stdout.trim());
  }
  console.log(JSON.stringify(counts));app.quit();
}).catch(e=>{console.error(e);app.exit(1);});
