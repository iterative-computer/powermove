import { contextBridge, ipcRenderer, sharedTexture } from 'electron';
import type { ImportedLike } from './frame-bridge';

/* Self-contained: sharing the viewer's receiver implementation would make
   Rollup emit a require('./chunks/...') that sandboxed preloads cannot load.
   This receiver owns only the encoder's five-slot BGRA pools. */
const imports = new Map<string, ImportedLike>();
const listeners = new Set<(generation: number, poolIndex: number) => void>();
const retired = new Set<number>();
let launch = 0;
const release = (texture: ImportedLike, notice = true): void => {
  if (texture.subtle && texture.textureId) {
    texture.subtle.release(() => notice
      ? ipcRenderer.invoke('IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN', texture.textureId).catch(() => undefined)
      : undefined);
  } else texture.release();
};
const retire = (generation: number): void => {
  retired.add(generation);
  for (const [key, texture] of imports) if (key.startsWith(`${generation}:`)) {
    imports.delete(key); release(texture);
  }
};
const clear = (): void => {
  for (const texture of imports.values()) release(texture);
  imports.clear(); retired.clear();
};
sharedTexture.setSharedTextureReceiver(async ({ importedSharedTexture }, ...args: unknown[]) => {
  const info = args[0] as { kind?: unknown; launch?: number; generation?: number; poolIndex?: number; poolSize?: number; format?: string } | undefined;
  const texture = importedSharedTexture as ImportedLike;
  if (info?.kind !== 'powermove-frame-surface' || info.format !== 'bgra' || info.poolSize !== 5 ||
      !Number.isSafeInteger(info.launch) || !Number.isSafeInteger(info.generation) || !Number.isSafeInteger(info.poolIndex) ||
      info.launch! < launch || info.generation! < 1 || info.poolIndex! < 0 || info.poolIndex! >= 5) {
    release(texture); return;
  }
  if (info.launch! > launch) { clear(); launch = info.launch!; }
  if (retired.has(info.generation!)) { release(texture); return; }
  const key = `${info.generation}:${info.poolIndex}`;
  const previous = imports.get(key);
  if (previous?.textureId && previous.textureId === texture.textureId) { release(texture, false); return; }
  if (previous) release(previous);
  imports.set(key, texture);
  for (const listener of listeners) listener(info.generation!, info.poolIndex!);
});
contextBridge.exposeInMainWorld('exportEncoderFrames', {
  has: (generation: number, slot: number) => imports.has(`${generation}:${slot}`),
  videoFrame: (generation: number, slot: number) => imports.get(`${generation}:${slot}`)?.getVideoFrame() ?? null,
  onSurface: (listener: (generation: number, slot: number) => void) => { listeners.add(listener); return () => listeners.delete(listener); },
});
ipcRenderer.on('export-encoder:retire', (_event, generation: number) => retire(generation));
window.addEventListener('pagehide', clear);
ipcRenderer.on('export-encoder:port', (event) => window.postMessage('export-encoder:port', '*', event.ports));
