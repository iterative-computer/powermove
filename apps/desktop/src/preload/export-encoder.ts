import { ipcRenderer } from 'electron';

/* The export encoder window's preload (main/engine/export-encoder.ts): it
   only hands the page the MessagePort main talks to it on. Self-contained
   (a sandboxed preload cannot require a shared chunk), so the channel
   repeats EXPORT_ENCODER_PORT (shared/export-encoder.ts). */
const EXPORT_ENCODER_PORT = 'export-encoder:port';

ipcRenderer.on(EXPORT_ENCODER_PORT, (event) => {
  window.postMessage(EXPORT_ENCODER_PORT, '*', event.ports);
});
