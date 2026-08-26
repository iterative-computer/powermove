import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron';

import {
  IPC,
  type ArtifactFile,
  type CaptureResult,
  type CodexProgressEvent,
  type CodexRunResult,
  type ConsentResult,
  type FileSaveResult,
  type MenuCommand,
  type PowermoveBridge,
  type StoreErrorEvent,
  type StoreSnapshot
} from '../shared/ipc';

const bridge: PowermoveBridge = {
  ping: () => ipcRenderer.invoke(IPC.ping) as Promise<string>,

  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  },

  saveFile: (req) => ipcRenderer.invoke(IPC.fileSave, req) as Promise<FileSaveResult>,

  codex: {
    async run(req, onProgress) {
      const listener = (_event: IpcRendererEvent, progress: CodexProgressEvent): void => {
        if (progress.id === req.id && progress.kind === 'progress') {
          onProgress?.(progress.text);
        }
      };

      ipcRenderer.on(IPC.codexEvent, listener);
      try {
        return (await ipcRenderer.invoke(IPC.codexRun, req)) as CodexRunResult;
      } finally {
        ipcRenderer.removeListener(IPC.codexEvent, listener);
      }
    },
    cancel: (id) => ipcRenderer.invoke(IPC.codexCancel, { id }) as Promise<void>,
    requestComputerConsent: (req) =>
      ipcRenderer.invoke(IPC.consentComputer, req) as Promise<ConsentResult>
  },

  artifacts: {
    read: (ref) => ipcRenderer.invoke(IPC.artifactRead, ref) as Promise<ArtifactFile>,
    reveal: (ref) => ipcRenderer.invoke(IPC.artifactReveal, ref) as Promise<void>
  },

  captureWindow: () => ipcRenderer.invoke(IPC.captureWindow) as Promise<CaptureResult>,

  store: {
    snapshotSync: () => ipcRenderer.sendSync(IPC.storeSnapshotSync) as StoreSnapshot,
    snapshot: () => ipcRenderer.invoke(IPC.storeSnapshot) as Promise<StoreSnapshot>,
    set: (key, value) => {
      ipcRenderer.send(IPC.storeSet, { key, value });
    },
    delete: (key) => {
      ipcRenderer.send(IPC.storeDelete, { key });
    },
    flush: () => ipcRenderer.invoke(IPC.storeFlush) as Promise<void>,
    onError: (cb) => {
      const listener = (_event: IpcRendererEvent, error: StoreErrorEvent): void => cb(error);
      ipcRenderer.on(IPC.storeError, listener);
      return () => ipcRenderer.removeListener(IPC.storeError, listener);
    }
  },

  setTheme: (theme) => {
    ipcRenderer.send(IPC.themeSet, theme);
  },
  haptic: {
    alignment: () => {
      ipcRenderer.send(IPC.hapticAlignment);
    }
  },
  log: (level, text) => {
    ipcRenderer.send(IPC.log, { level, text });
  },
  openExternal: (url) => ipcRenderer.invoke(IPC.openExternal, url) as Promise<void>,
  onMenuCommand: (cb) => {
    const listener = (_event: IpcRendererEvent, command: MenuCommand): void => cb(command);
    ipcRenderer.on(IPC.menuCommand, listener);
    return () => ipcRenderer.removeListener(IPC.menuCommand, listener);
  }
};

contextBridge.exposeInMainWorld('powermove', bridge);
