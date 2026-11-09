import { ipcRenderer, sharedTexture } from 'electron';
import { STUDIO_NATIVE_IPC, type StudioNativeBridge, type StudioNativeTexture, type StudioNativeTextureInfo } from '../shared/studio-native';

/** Managed sharedTexture receivers also work in a sandboxed preload. Electron
 * retains GPU resources until the callback's VideoFrame work is complete. */
export function studioNativeBridge(): StudioNativeBridge {
  const listeners = new Set<(info: StudioNativeTextureInfo, texture: StudioNativeTexture) => Promise<void> | void>();
  const supportsTextures = typeof sharedTexture?.setSharedTextureReceiver === 'function';
  if (supportsTextures) sharedTexture.setSharedTextureReceiver(async ({ importedSharedTexture }, info: StudioNativeTextureInfo) => {
    try { for (const listener of listeners) await listener(info, importedSharedTexture); }
    finally { importedSharedTexture.release(); }
  });
  return {
    request: (session, command, binary) => ipcRenderer.invoke(STUDIO_NATIVE_IPC.request, session, command, binary),
    release: session => ipcRenderer.invoke(STUDIO_NATIVE_IPC.release, session),
    status: () => ipcRenderer.invoke(STUDIO_NATIVE_IPC.status),
    ...(supportsTextures ? { onTexture: (callback: (info: StudioNativeTextureInfo, texture: StudioNativeTexture) => Promise<void> | void) => { listeners.add(callback); return () => listeners.delete(callback); } } : {}),
  };
}
