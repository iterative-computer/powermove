// Synthetic recorded PM/Playwright protocol double. Never launches an app.
const fs = require('node:fs');
let handler;
let version = 1;
const emit = () => {
  // A stale event must not claim this measurement's GPU fence.
  handler?.({time:999, version:0});
  handler?.({time:window.PM.time, version});
};
const gl = {
  SYNC_GPU_COMMANDS_COMPLETE:1, WAIT_FAILED:2, TIMEOUT_EXPIRED:3,
  fenceSync:()=>({}), flush:()=>{}, clientWaitSync:()=>1, deleteSync:()=>{},
};
global.requestAnimationFrame = callback => setTimeout(callback, 0);
global.cancelAnimationFrame = clearTimeout;
global.CustomEvent = class { constructor(type, {detail}) { this.type=type; this.detail=detail; } };
global.window = {
  PM:{app:{}, projectReady:Promise.resolve(), proj:null, time:0, playing:false,
    Kernel:{loader:{builtinsReady:Promise.resolve()},services:{get:()=>({stage:{clientWidth:640,clientHeight:360}})}},
    bus:{on:(_name, callback)=>{ handler=callback; return ()=>{handler=undefined;}; }},
    GL:{gl}, animVersion:()=>version,
    play:()=>{window.PM.playing=true;}, pause:()=>{window.PM.playing=false;},
    setTime:seconds=>{window.PM.time=seconds;emit();}, invalidate:()=>{},
    Edit:{apply:()=>{version++;emit();}}},
  dispatchEvent:event=>{window.PM.proj=event.detail;window.PM.time=0;emit();},
};
global.devicePixelRatio = 2;
exports._electron = {launch:async options => {
  // Record only public test selectors, never the inherited process environment.
  fs.writeFileSync(process.env.PM_PERF_TEST_LAUNCH, JSON.stringify({
    executablePath:options.executablePath, args:options.args,
    profile:options.env.POWERMOVE_USER_DATA,
    runtimeUpdates:options.env.POWERMOVE_DISABLE_RUNTIME_UPDATES,
    runAsNode:options.env.ELECTRON_RUN_AS_NODE ?? null,
    background:options.env.POWERMOVE_BACKGROUND_TEST,
  }));
  return {process:()=>({pid:process.pid}), close:async()=>{},
    firstWindow:async()=>({setDefaultTimeout:()=>{},waitForURL:async()=>{},
      waitForFunction:async fn=>{if(!fn())throw new Error('mock readiness failed');},
      evaluate:async(fn,arg)=>fn(arg)})};
}};
