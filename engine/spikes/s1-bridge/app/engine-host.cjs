'use strict';
// The interactive engine process (Electron utilityProcess). Loads the Rust addon,
// which creates the GPU device and IOSurface pool, checks in with main through the
// per-launch bootstrap name in PM_S1_BRIDGE_NAME, and sends every surface once.
//
// Two channels reach this process:
//   - process.parentPort, from main: start, stop, stats, and releases for frames
//     whose transfer failed in main.
//   - a MessagePort from the viewer's window (MessageChannelMain made by main): input,
//     per-frame releases, display timing, mode, and clock pings, and (persistent
//     transport only) per-frame descriptors to the viewer. Main's JavaScript thread is
//     never on this path.

const path = require('node:path');
const addon = require(path.join(__dirname, 'native', 's1_engine.node'));

let engine = null;
let viewer = null;
const pending = [];

function handleViewer(m) {
  if (m.t === 'ping') {
    viewer.postMessage({ t: 'pong', id: m.id, now: addon.nowUs() });
    return;
  }
  if (!engine) {
    pending.push(m);
    return;
  }
  switch (m.t) {
    case 'input':
      engine.input(m.rev, m.x, m.y, m.ts, m.kind);
      break;
    case 'release':
      engine.release(m.pool, m.seq);
      break;
    case 'timing':
      engine.setDisplayTiming(m.anchor, m.period);
      break;
    case 'mode':
      engine.setMode(m.mode);
      break;
    case 'lead':
      engine.setLeadFrames(m.lead);
      break;
    default:
      process.parentPort.postMessage({ type: 'error', message: `unknown viewer message ${m.t}` });
  }
}

process.parentPort.on('message', e => {
  const m = e.data;
  switch (m.type) {
    case 'port':
      viewer = e.ports[0];
      viewer.on('message', ev => handleViewer(ev.data));
      viewer.start();
      break;
    case 'start':
      try {
        const { transport, ...config } = m.config;
        engine = new addon.Engine(config);
        if (transport === 'persistent') {
          // Persistent-import transport: the viewer holds one import per pool slot, so
          // each descriptor goes straight to it and main's JavaScript is never on the
          // per-frame path. Main still receives every descriptor over Mach.
          engine.onFrame(f => {
            if (!viewer) return;
            viewer.postMessage({
              t: 'frame',
              d: {
                poolIndex: f.poolIndex,
                seq: f.seq,
                ptsUs: f.ptsUs,
                durationUs: f.durationUs,
                revision: f.revision,
                inputTsUs: f.inputTsUs,
                renderStartUs: f.renderStartUs,
                gpuDoneUs: f.gpuDoneUs,
                bridgeRecvUs: f.hookUs,
                mainJsUs: addon.nowUs(),
                flags: f.flags,
                colorSpace: f.colorSpace,
                overlay: f.overlay
              }
            });
          });
        }
        process.parentPort.postMessage({ type: 'ready', info: JSON.parse(engine.info()) });
        for (const p of pending.splice(0)) handleViewer(p);
      } catch (err) {
        process.parentPort.postMessage({ type: 'error', message: String(err && err.message ? err.message : err) });
      }
      break;
    case 'release':
      engine?.release(m.pool, m.seq);
      break;
    case 'stats':
      process.parentPort.postMessage({
        type: 'stats',
        id: m.id,
        stats: engine ? JSON.parse(engine.stats()) : null,
        log: engine && m.log ? JSON.parse(engine.takeLog()) : []
      });
      break;
    case 'stop':
      engine?.stop();
      engine = null;
      process.parentPort.postMessage({ type: 'stopped' });
      setTimeout(() => process.exit(0), 50);
      break;
    default:
      process.parentPort.postMessage({ type: 'error', message: `unknown message ${m.type}` });
  }
});
