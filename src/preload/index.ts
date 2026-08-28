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
import {
  EXT_IPC,
  type ExtensionRecord,
  type ExtensionSourceFile,
  type ExtensionsChangedEvent
} from '../shared/extensions';

const bridge: PowermoveBridge = {
  ping: () => ipcRenderer.invoke(IPC.ping) as Promise<string>,

  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  },

  saveFile: (req) => ipcRenderer.invoke(IPC.fileSave, req) as Promise<FileSaveResult>,
  openProjectFile: () => ipcRenderer.invoke(IPC.projectOpen),
  confirmProjectClose: (name) => ipcRenderer.invoke(IPC.projectConfirmClose, name),

  codex: {
    async run(req, onProgress, onTrace) {
      const listener = (_event: IpcRendererEvent, progress: CodexProgressEvent): void => {
        if (progress.id !== req.id) return;
        if (progress.kind === 'progress') {
          onProgress?.(progress.text);
        } else {
          onTrace?.(progress.step);
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
    fixPrompt: (req) => ipcRenderer.invoke(IPC.codexFixPrompt, req) as Promise<string>,
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
  },

  extensions: {
    list: () => ipcRenderer.invoke(EXT_IPC.list) as Promise<ExtensionRecord[]>,
    setEnabled: (req) =>
      ipcRenderer.invoke(EXT_IPC.setEnabled, req) as Promise<ExtensionRecord[]>,
    remove: (req) => ipcRenderer.invoke(EXT_IPC.remove, req) as Promise<ExtensionRecord[]>,
    reload: (req) => ipcRenderer.invoke(EXT_IPC.reload, req) as Promise<ExtensionRecord[]>,
    create: (req) => ipcRenderer.invoke(EXT_IPC.create, req) as Promise<ExtensionRecord[]>,
    reveal: (req) => ipcRenderer.invoke(EXT_IPC.reveal, req) as Promise<void>,
    readSource: (req) =>
      ipcRenderer.invoke(EXT_IPC.readSource, req) as Promise<ExtensionSourceFile[]>,
    reportHealth: (req) => {
      ipcRenderer.send(EXT_IPC.reportHealth, req);
    },
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, change: ExtensionsChangedEvent): void => cb(change);
      ipcRenderer.on(EXT_IPC.changed, listener);
      return () => ipcRenderer.removeListener(EXT_IPC.changed, listener);
    }
  }
};

contextBridge.exposeInMainWorld('powermove', bridge);
