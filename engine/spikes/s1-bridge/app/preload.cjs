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

sharedTexture.setSharedTextureReceiver(async ({ importedSharedTexture: imported }, desc) => {
  const recvMs = performance.now();
  received += 1;
  let frame = null;
  try {
    frame = imported.getVideoFrame();
  } catch (err) {
    receiveErrors += 1;
    if (lateErrors.length < 8) lateErrors.push(String(err && err.message));
  } finally {
    imported.release();
  }
  if (!frame) return;
  if (!deliver) {
    // No viewer yet: give the slot straight back.
    frame.close();
    ipcRenderer.send('s1:undelivered', { pool: desc.poolIndex, seq: desc.seq });
    return;
  }
  deliver(frame, desc, recvMs);
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
  receiverStats: () => ({ received, receiveErrors, errors: lateErrors.slice() }),
  send: (channel, value) => {
    if (typeof channel === 'string' && channel.startsWith('s1:')) ipcRenderer.send(channel, value);
  },
  on: (channel, cb) => {
    if (typeof channel === 'string' && channel.startsWith('s1:')) ipcRenderer.on(channel, (_e, value) => cb(value));
  }
});
