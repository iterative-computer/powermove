import { contextBridge, ipcRenderer, webUtils, type IpcRendererEvent } from 'electron';

import {
  IPC,
  type ArtifactFile,
  type AttachmentRevealRequest,
  type CaptureResult,
  type ChatGPTAccountStatus,
  type ClaudeAccountStatus,
  type CodexProgressEvent,
  type CodexRunResult,
  type ConsentResult,
  type FileSaveResult,
  type MenuCommand,
  type MediaProxyResult,
  type NativeEditAction,
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
  compatible: {
    status: () => ipcRenderer.invoke(IPC.compatibleStatus),
    configure: input => ipcRenderer.invoke(IPC.compatibleConfigure, input),
  },
  agentNotification: options => ipcRenderer.invoke('agent:notification', options),
  ping: () => ipcRenderer.invoke(IPC.ping) as Promise<string>,

  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  },

  fileUpload: {
    begin: size => ipcRenderer.invoke(IPC.fileSaveUpload, size),
    chunk: (uploadId, data) => ipcRenderer.invoke(IPC.fileSaveChunk, { uploadId, data }),
    finish: (uploadId, metadata) => ipcRenderer.invoke(IPC.fileSave, { ...metadata, uploadId }),
    abort: uploadId => ipcRenderer.invoke(IPC.fileSaveAbort, uploadId),
  },
  saveFile: async (req) => {
    if (req.data.byteLength <= 4 * 1024 * 1024) return ipcRenderer.invoke(IPC.fileSave, req) as Promise<FileSaveResult>;
    const uploadId: string = await ipcRenderer.invoke(IPC.fileSaveUpload, req.data.byteLength);
    try {
      for (let offset = 0; offset < req.data.byteLength; offset += 1024 * 1024) {
        await ipcRenderer.invoke(IPC.fileSaveChunk, { uploadId, data: req.data.slice(offset, offset + 1024 * 1024) });
      }
      const { data, ...metadata } = req;
      return await ipcRenderer.invoke(IPC.fileSave, { ...metadata, uploadId }) as FileSaveResult;
    } finally { await ipcRenderer.invoke(IPC.fileSaveAbort, uploadId).catch(() => undefined); }
  },
  openProjectFile: () => ipcRenderer.invoke(IPC.projectOpen),
  confirmProjectClose: (name) => ipcRenderer.invoke(IPC.projectConfirmClose, name),

  render: {
    start: options => ipcRenderer.invoke(IPC.renderStart,options),
    write: (token,data,audio) => ipcRenderer.invoke(IPC.renderWrite,{token,data,audio}),
    finish: token => ipcRenderer.invoke(IPC.renderFinish,{token}),
    cancel: token => ipcRenderer.invoke(IPC.renderCancel,{token}),
  },
  media: {
    sourcePath: (file) => webUtils.getPathForFile(file) || null,
    revealSource: (sourcePath) => ipcRenderer.invoke(IPC.mediaRevealSource, sourcePath) as Promise<void>,
    createPlaybackProxy: (file) => {
      const sourcePath = webUtils.getPathForFile(file);
      if (!sourcePath) return Promise.resolve({ ok: false, error: 'The original file is no longer available' });
      return ipcRenderer.invoke(IPC.mediaProxyCreate, {
        sourcePath,
        name: file.name
      }) as Promise<MediaProxyResult>;
    },
    readPlaybackProxy: (token, offset, length) =>
      ipcRenderer.invoke(IPC.mediaProxyRead, { token, offset, length }) as Promise<Uint8Array>,
    releasePlaybackProxy: (token) =>
      ipcRenderer.invoke(IPC.mediaProxyRelease, token) as Promise<void>
  },
  attachments: {
    reveal: (request: AttachmentRevealRequest) =>
      ipcRenderer.invoke(IPC.attachmentReveal, request) as Promise<void>
  },

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
    steer: (req) => ipcRenderer.invoke(IPC.codexSteer, req),
    cancel: (id) => ipcRenderer.invoke(IPC.codexCancel, { id }) as Promise<void>,
    fixPrompt: (req) => ipcRenderer.invoke(IPC.codexFixPrompt, req) as Promise<string>,
    restoreChangeSet: (req) => ipcRenderer.invoke(IPC.codexRestoreChangeSet, req),
    requestComputerConsent: (req) =>
      ipcRenderer.invoke(IPC.consentComputer, req) as Promise<ConsentResult>
  },

  agentTools: {
    onRequest: (cb) => {
      const listener = (_event: IpcRendererEvent, request: Parameters<typeof cb>[0]): void => cb(request);
      ipcRenderer.on(IPC.agentToolRequest, listener);
      return () => ipcRenderer.removeListener(IPC.agentToolRequest, listener);
    },
    respond: (response) => {
      ipcRenderer.send(IPC.agentToolResponse, response);
    }
  },

  chatgpt: {
    status: () => ipcRenderer.invoke(IPC.chatgptStatus) as Promise<ChatGPTAccountStatus>,
    connect: () => ipcRenderer.invoke(IPC.chatgptConnect) as Promise<ChatGPTAccountStatus>,
    disconnect: () => ipcRenderer.invoke(IPC.chatgptDisconnect) as Promise<ChatGPTAccountStatus>,
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, status: ChatGPTAccountStatus): void => cb(status);
      ipcRenderer.on(IPC.chatgptChanged, listener);
      return () => ipcRenderer.removeListener(IPC.chatgptChanged, listener);
    }
  },

  claude: {
    status: () => ipcRenderer.invoke(IPC.claudeStatus) as Promise<ClaudeAccountStatus>,
    connect: () => ipcRenderer.invoke(IPC.claudeConnect) as Promise<ClaudeAccountStatus>,
    disconnect: () => ipcRenderer.invoke(IPC.claudeDisconnect) as Promise<ClaudeAccountStatus>,
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, status: ClaudeAccountStatus): void => cb(status);
      ipcRenderer.on(IPC.claudeChanged, listener);
      return () => ipcRenderer.removeListener(IPC.claudeChanged, listener);
    }
  },

  artifacts: {
    read: (ref) => ipcRenderer.invoke(IPC.artifactRead, ref) as Promise<ArtifactFile>,
    reveal: (ref) => ipcRenderer.invoke(IPC.artifactReveal, ref) as Promise<void>
  },

  captureWindow: () => ipcRenderer.invoke(IPC.captureWindow) as Promise<CaptureResult>,

  store: {
    setSerialized: (key, serialized) => ipcRenderer.invoke(IPC.storeSetSerialized, { key, serialized }),
    snapshotSync: () => ipcRenderer.sendSync(IPC.storeSnapshotSync) as StoreSnapshot,
    snapshotSerializedSync: () => ipcRenderer.sendSync(IPC.storeSnapshotSerializedSync) as Record<string, string>,
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
  nativeEdit: (action: NativeEditAction) => ipcRenderer.send(IPC.nativeEdit, action),
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
