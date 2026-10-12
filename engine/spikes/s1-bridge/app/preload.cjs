'use strict';
// Viewer preload (sandboxed, context-isolated world), plan section 18 step 5.
//
// setSharedTextureReceiver runs here, in the isolated world. Each received texture
// becomes a VideoFrame created in this world, the per-transfer import is released
// at once (the VideoFrame keeps the shared image alive until it is closed and the
// GPU work that read it has finished), and the VideoFrame crosses to the main world
// through contextBridge, which passes VideoFrame objects natively.
//
// The engine's MessagePort arrives from main and is forwarded to the main world
// with window.postMessage, so input and releases flow renderer -> engine directly.

const { contextBridge, ipcRenderer, sharedTexture } = require('electron');

let deliver = null;
let received = 0;
let receiveErrors = 0;
const lateErrors = [];
// How a frame's pool slot goes back to the engine (plan section 18 step 4):
//   'gpu-done'  the per-transfer import is released at once; the main world closes the
//               VideoFrame after submitting the draw and releases the slot when
//               queue.onSubmittedWorkDone() resolves.
//   'callback'  the per-transfer import is held until the main world has closed the
//               VideoFrame, then released with the per-use release callback
//               (subtle.release(cb)); the slot goes back when that callback runs.
let releaseMode = 'gpu-done';
let onReleased = null;
// The channel Electron's own SharedTextureImported.release() wrapper uses to tell main
// that this renderer dropped its reference (lib/renderer/api/shared-texture.ts). The
// 'callback' mode calls subtle.release(cb) directly, so it must send the same notice
// or main's allReferencesReleased never fires.
const RELEASE_TO_MAIN = 'IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN';

function releaseWithCallback(imported, desc) {
  const id = imported.textureId;
  imported.subtle.release(async () => {
    const t = performance.now();
    if (onReleased) onReleased(desc.poolIndex, desc.seq, t);
    try {
      await ipcRenderer.invoke(RELEASE_TO_MAIN, id);
    } catch (err) {
      receiveErrors += 1;
      if (lateErrors.length < 8) lateErrors.push(`release notice: ${err && err.message}`);
    }
  });
}

// Persistent-import transport: main sends each pool slot once and this world keeps
// the import for the window's lifetime; per-frame descriptors come from the engine
// over its MessagePort and each one gets a fresh VideoFrame from the held import.
const persistent = new Map();

sharedTexture.setSharedTextureReceiver(async ({ importedSharedTexture: imported }, desc) => {
  if (desc && desc.persistent) {
    const old = persistent.get(desc.poolIndex);
    if (old) old.release();
    persistent.set(desc.poolIndex, imported);
    return;
  }
  const recvMs = performance.now();
  received += 1;
  let frame = null;
  try {
    frame = imported.getVideoFrame();
  } catch (err) {
    receiveErrors += 1;
    if (lateErrors.length < 8) lateErrors.push(String(err && err.message));
  }
  const byCallback = releaseMode === 'callback' && frame && deliver;
  if (!byCallback) imported.release();
  if (!frame) {
    ipcRenderer.send('s1:undelivered', { pool: desc.poolIndex, seq: desc.seq });
    return;
  }
  if (!deliver) {
    // No viewer yet: give the slot straight back.
    frame.close();
    ipcRenderer.send('s1:undelivered', { pool: desc.poolIndex, seq: desc.seq });
    return;
  }
  // In 'callback' mode the main world calls done() once it has closed the frame.
  deliver(frame, desc, recvMs, byCallback ? () => releaseWithCallback(imported, desc) : null);
});

ipcRenderer.on('s1:port', event => {
  window.postMessage({ s1: 'engine-port' }, '*', event.ports);
});

// Agent-like tool traffic from main (see load.cjs): build and return a result the
// size of a layer listing.
ipcRenderer.on('s1:agent', (_e, req) => {
  const layers = [];
  for (let i = 0; i < 600; i++) layers.push({ id: `layer-${i}`, name: `Layer ${i}`, in: i * 0.1, out: i * 0.1 + 5, props: { x: i, y: i * 2, opacity: 1, effects: ['blur', 'glow'] } });
  ipcRenderer.send('s1:agent-reply', { id: req.id, layers });
});

contextBridge.exposeInMainWorld('s1', {
  onFrame: cb => {
    deliver = cb;
  },
  frameFor: poolIndex => {
    const imported = persistent.get(poolIndex);
    return imported ? imported.getVideoFrame() : null;
  },
  persistentSlots: () => persistent.size,
  releasePersistent: () => {
    const n = persistent.size;
    for (const imported of persistent.values()) imported.release();
    persistent.clear();
    return n;
  },
  setReleaseMode: (mode, cb) => {
    releaseMode = mode === 'callback' ? 'callback' : 'gpu-done';
    onReleased = typeof cb === 'function' ? cb : null;
  },
  receiverStats: () => ({ received, receiveErrors, errors: lateErrors.slice(), releaseMode }),
  send: (channel, value) => {
    if (typeof channel === 'string' && channel.startsWith('s1:')) ipcRenderer.send(channel, value);
  },
  on: (channel, cb) => {
    if (typeof channel === 'string' && channel.startsWith('s1:')) ipcRenderer.on(channel, (_e, value) => cb(value));
  }
});
