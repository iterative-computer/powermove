// Node (not Bun): reuse the frozen tag's launchApp, disposable profile and hidden test bridge.
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
import { readFile, mkdir, writeFile, realpath } from 'node:fs/promises';
import { createRequire } from 'node:module';
delete process.env.ELECTRON_RUN_AS_NODE;
delete process.env.POWERMOVE_E2E_LIVE;
const ref = process.argv[2];
async function launchConfigured() {
  const profile = process.env.PM_PERF_ELECTRON_PROFILE;
  if (!profile) {
    throw new Error('Configured Electron requires an isolated temporary profile');
  }
  const temporaryRoot = await realpath('/tmp');
  const parent = await realpath(path.dirname(path.resolve(profile)));
  const relative = path.relative(temporaryRoot, parent);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error('Configured Electron profile parent is outside the canonical temporary root');
  }
  const require = createRequire(path.join(ref, 'apps/desktop/package.json'));
  const { _electron } = require('playwright');
  // Refuse existing profiles and linked destinations. Never adopt or overwrite
  // a profile just because it happens to live under the temporary root.
  await mkdir(profile, {mode:0o700});
  await writeFile(path.join(profile, 'onboarding-v1.json'), '{"version":1}\n', {mode:0o600});
  const app = await _electron.launch({executablePath:process.env.PM_PERF_ELECTRON_PATH,
    args:JSON.parse(process.env.PM_PERF_ELECTRON_ARGS ?? '[]'),
    cwd:path.join(ref, 'apps/desktop'), timeout:30_000,
    env:{...process.env, POWERMOVE_USER_DATA:profile, POWERMOVE_DEVTOOLS:'0',
      POWERMOVE_BACKGROUND_TEST:'0', POWERMOVE_DISABLE_RUNTIME_UPDATES:'1',
      CODEX_BINARY:path.join(ref,'apps/desktop/src/main/codex/__fixtures__/fake-codex-app-server.sh'),
      POWERMOVE_FAKE_CHATGPT_STATUS:'connected'}});
  try {
    const page = await app.firstWindow();
    page.setDefaultTimeout(20_000);
    await page.waitForURL(url => url.protocol === 'app:' || url.hostname === 'localhost');
    await page.waitForFunction(() => Boolean(window.PM?.app));
    await page.evaluate(async () => { await window.PM.projectReady; });
    return {app, page, close:()=>app.close()};
  } catch (error) { await app.close().catch(()=>{}); throw error; }
}
const send = value => process.stdout.write(`PM_PERF ${JSON.stringify(value)}\n`);
let session;
try {
  if (process.env.PM_PERF_ELECTRON_PATH) session = await launchConfigured();
  else {
    const { launchApp } = await import(pathToFileURL(path.join(ref, 'apps/desktop/e2e/helpers/app.perf.mjs')));
    session = await launchApp({ normalWindow: true, env: { POWERMOVE_DISABLE_RUNTIME_UPDATES: '1' } });
  }
  await session.page.evaluate(async () => { await window.PM.Kernel.loader.builtinsReady; });
  send({ready:true, pid:session.app.process().pid, endpoint:'harness-ready'});
  for await (const line of createInterface({input:process.stdin})) {
    try {
      const request = JSON.parse(line);
      if (request.op === 'quit') break;
      if (request.op === 'pids') { send({pids:await session.app.evaluate(({app})=>app.getAppMetrics().map(p=>p.pid))}); continue; }
      const doc = request.op === 'open' ? JSON.parse(await readFile(request.path,'utf8')) : null;
      const result = await session.page.evaluate(async ({request, doc}) => {
        const PM = window.PM;
        const started = performance.now();
        if (request.op === 'status') return {opened:!!PM.proj, duration:PM.proj?.dur, playing:PM.playing, error:null};
        if (request.op === 'play') { PM.play(); return {playing:PM.playing}; }
        if (request.op === 'pause') { PM.pause(); return {}; }
        if (!['open','seek','edit'].includes(request.op)) throw new Error('Unknown operation');
        const viewer = PM.Kernel.services.get('viewer');
        if (request.op !== 'open' && !PM.GL?.gl) throw new Error('Missing WebGL context');
        const target = request.op === 'seek' ? request.seconds : 0;
        const wantVersion = Number(PM.animVersion?.() ?? 0) + (request.op === 'edit' ? 1 : 0);
        let off = () => {}, sync, timer, raf, timeout, ownShare;
        const elapsed = await new Promise((resolve,reject) => {
          let claimed = false;
          timeout = setTimeout(()=>reject(new Error('Matching GPU-complete frame timed out')),15000);
          off = PM.bus.on('preview:presented', e => {
            if (claimed || (request.op === 'open' && PM.proj?.id !== doc.id) ||
                Math.abs(Number(e?.time)-target)>1e-5 || (request.op === 'edit' && Number(e?.version ?? -1)<wantVersion)) return;
            claimed = true;
            const gl = PM.GL.gl;
            sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE,0);
            if (!sync) { reject(new Error('WebGL fence unavailable')); return; }
            gl.flush();
            const poll = () => {
              const status = gl.clientWaitSync(sync,0,0);
              if (status === gl.WAIT_FAILED) { reject(new Error('WebGL fence failed')); return; }
              if (status === gl.TIMEOUT_EXPIRED) { timer=setTimeout(poll,1); return; }
              ownShare=performance.now()-started;
              raf=requestAnimationFrame(()=>resolve(performance.now()-started));
            };
            timer=setTimeout(poll,0);
          });
          if (request.op === 'open') {
            window.dispatchEvent(new CustomEvent('pm-open-project',{detail:doc}));
            PM.ProjectsScreen?.hide();
          } else if (request.op === 'seek') {
            PM.pause(); PM.setTime(request.seconds); PM.invalidate?.();
          } else {
            const layer=PM.proj.layers.find(l=>l.type!=='audio' && !l.locked);
            if (!layer) { reject(new Error('No editable layer')); return; }
            PM.Edit.apply({type:'set_property',target:layer.id,path:'opacity',value:request.value});
            PM.invalidate?.();
          }
        }).finally(()=>{off();clearTimeout(timeout);clearTimeout(timer);cancelAnimationFrame(raf);if(sync)PM.GL.gl.deleteSync(sync);});
        return {ms:elapsed, own_share_ms:ownShare, on_screen_ms:null,
          endpoint:'gpu-complete-next-raf',physical_present:false,
          width:viewer?.stage?.clientWidth, height:viewer?.stage?.clientHeight, dpr:devicePixelRatio};
      }, {request,doc});
      send(result);
    } catch(error) { send({error:String(error.message ?? error)}); }
  }
} catch(error) { send({error:String(error.message ?? error)}); process.exitCode=1; }
finally { if(session) await session.close().catch(()=>{session.app.process().kill('SIGKILL');}); }
