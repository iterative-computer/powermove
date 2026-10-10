import { contextBridge, ipcRenderer, webUtils, type IpcRendererEvent } from 'electron';

import {
  IPC,
  type AppUpdateState,
  type WhatsNew,
  type ArtifactFile,
  type AttachmentRevealRequest,
  type CaptureResult,
  type ChatGPTAccountStatus,
  type ClaudeAccountStatus,
  type ClaudeModelOption,
  type CodexProgressEvent,
  type CodexModelOption,
  type CodexRunResult,
  type ConsentResult,
  type ExtensionForkResult,
  type FileSaveResult,
  type MenuCommand,
  type MediaProxyResult,
  type NativeEditAction,
  type PowermoveBridge,
  type StoreErrorEvent,
  type StoreSnapshot,
  type WindowAdoptTab,
  type WindowClaimResult,
  type WindowPlaceTabResult,
  type WindowInitialProject,
  type WindowOpenResult
} from '../shared/ipc';
import {
  EXT_IPC,
  type ExtensionRecord,
  type ExtensionSourceFile,
  type ExtensionsChangedEvent
} from '../shared/extensions';
import { VARS_IPC, type VarsStatus } from '../shared/vars-ipc';
import { MEDIA_PATH_IPC } from '../shared/media-tools';
import { CLOUD_IPC, type CloudChannel, type CloudChannels } from '../shared/cloud-ipc';
import { STORE_IPC, type StoreChannel, type StoreChannels } from '../shared/store-ipc';
import { TRANSCRIPTION_IPC, type TranscribeProgress, type TranscribeResult, type TranscriptionStatus } from '../shared/transcription';

/** One typed invoke for every `cloud:*` channel (store plan §2.7). */
function cloudInvoke<C extends CloudChannel>(channel: C, request?: CloudChannels[C]['req']): Promise<CloudChannels[C]['res']> {
  return ipcRenderer.invoke(channel, request);
}

/** One typed invoke for every `store:*` channel. */
function storeInvoke<C extends StoreChannel>(channel: C, request?: StoreChannels[C]['req']): Promise<StoreChannels[C]['res']> {
  return ipcRenderer.invoke(channel, request);
}

function cloudEvent<T>(channel: string) {
  return (cb: (payload: T) => void): (() => void) => {
    const listener = (_event: IpcRendererEvent, payload: T): void => cb(payload);
    ipcRenderer.on(channel, listener);
    return () => ipcRenderer.removeListener(channel, listener);
  };
}

let nextMediaRequest = 0;
const bridge: PowermoveBridge = {
  onInputKey: (cb) => {
    const listener = (_event: IpcRendererEvent, input: import('../shared/ipc').SandboxInputKey): void => cb(input);
    ipcRenderer.on(IPC.inputKey, listener);
    return () => ipcRenderer.removeListener(IPC.inputKey, listener);
  },
  sandboxFocus: (focus) => ipcRenderer.send(IPC.storeSandboxFocus, focus),
  sandboxTerminate: (extensionId) => ipcRenderer.invoke(IPC.sandboxTerminate, extensionId) as Promise<number>,
  compatible: {
    status: () => ipcRenderer.invoke(IPC.compatibleStatus),
    configure: input => ipcRenderer.invoke(IPC.compatibleConfigure, input),
  },
  agentNotification: options => ipcRenderer.invoke('agent:notification', options),
  ping: () => ipcRenderer.invoke(IPC.ping) as Promise<string>,

  fontFamilies: () => ipcRenderer.invoke(IPC.fontFamilies),

  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  },

  exportDestination: {
    choose: (name, directory) => ipcRenderer.invoke(IPC.exportChoose, { name, directory }),
    release: token => ipcRenderer.invoke(IPC.exportRelease, token),
  },
  exportSidecar: request => ipcRenderer.invoke(IPC.exportSidecar, request),
  fileUpload: {
    begin: size => ipcRenderer.invoke(IPC.fileSaveUpload, size),
    chunk: (uploadId, data) => ipcRenderer.invoke(IPC.fileSaveChunk, { uploadId, data }),
    finish: (uploadId, metadata) => ipcRenderer.invoke(IPC.fileSave, { ...metadata, uploadId }),
    abort: uploadId => ipcRenderer.invoke(IPC.fileSaveAbort, uploadId),
  },
  projectSave: {
    begin: request => ipcRenderer.invoke(IPC.projectSaveBegin, request),
    chunk: (token, assetId, data) => ipcRenderer.invoke(IPC.projectSaveChunk, { token, assetId, data }),
    finish: token => ipcRenderer.invoke(IPC.projectSaveFinish, token),
    abort: token => ipcRenderer.invoke(IPC.projectSaveAbort, token),
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
  openProjectFromFile: (file) => {
    const filePath = webUtils.getPathForFile(file);
    return filePath ? ipcRenderer.invoke(IPC.projectOpenPath, filePath) : Promise.resolve(null);
  },
  onProjectOpenExternal: (cb) => {
    const listener = (_event: IpcRendererEvent, result: Parameters<typeof cb>[0]): void => cb(result);
    ipcRenderer.on(IPC.projectOpenExternal, listener);
    // Files opened from Finder while this page loaded are held until now.
    ipcRenderer.send(IPC.projectOpenReady);
    return () => ipcRenderer.removeListener(IPC.projectOpenExternal, listener);
  },
  projectRead: {
    read: (token, offset, length) => ipcRenderer.invoke(IPC.projectRead, { token, offset, length }),
    close: token => ipcRenderer.invoke(IPC.projectReadClose, token),
  },
  confirmProjectClose: (name) => ipcRenderer.invoke(IPC.projectConfirmClose, name),
  confirm: (request) => ipcRenderer.invoke(IPC.dialogConfirm, request) as Promise<boolean>,
  clipboardWriteText: (text) => ipcRenderer.invoke(IPC.clipboardWriteText, text) as Promise<void>,

  blender: {
    status: () => ipcRenderer.invoke(IPC.blenderStatus),
    choose: () => ipcRenderer.invoke(IPC.blenderChoose),
    run: job => ipcRenderer.invoke(IPC.blenderRun, job),
    cancel: () => ipcRenderer.invoke(IPC.blenderCancel),
  },
  render: {
    start: options => ipcRenderer.invoke(IPC.renderStart,options),
    write: (token,data,audio) => ipcRenderer.invoke(IPC.renderWrite,{token,data,audio}),
    finish: token => ipcRenderer.invoke(IPC.renderFinish,{token}),
    cancel: token => ipcRenderer.invoke(IPC.renderCancel,{token}),
  },
  media: {
    beginPreview: size => ipcRenderer.invoke(IPC.mediaPreviewBegin, size),
    writePreview: (token, offset, data) => ipcRenderer.invoke(IPC.mediaPreviewChunk, { token, offset, data }),
    finishPreview: token => ipcRenderer.invoke(IPC.mediaPreviewFinish, token),
    cloudStatus: (paths) => ipcRenderer.invoke(IPC.cloudStatus, paths),
    openLocalSource: (path) => ipcRenderer.invoke(IPC.mediaOpenLocalSource, path),
    cloudPrompt: (names) => ipcRenderer.invoke(IPC.cloudPrompt, names),
    downloadCloudSource: (path) => ipcRenderer.invoke(IPC.cloudDownload, path),
    readCloudSource: (token, offset, length) => ipcRenderer.invoke(IPC.cloudRead, { token, offset, length }),
    releaseCloudSource: (token) => ipcRenderer.invoke(IPC.cloudRelease, token),
    sourcePath: (file) => webUtils.getPathForFile(file) || null,
    revealSource: (sourcePath) => ipcRenderer.invoke(IPC.mediaRevealSource, sourcePath) as Promise<void>,
    createPlaybackProxy: (file, originalPath) => {
      const sourcePath = originalPath || webUtils.getPathForFile(file);
      if (!sourcePath) return Promise.resolve({ ok: false, error: 'The original file is no longer available' });
      return ipcRenderer.invoke(IPC.mediaProxyCreate, {
        sourcePath,
        name: file.name
      }) as Promise<MediaProxyResult>;
    },
    createImageSequence: async (files, fps, onProgress, originalPaths) => {
      const sourcePaths = files.map((file, index) => originalPaths?.[index] || webUtils.getPathForFile(file));
      if (sourcePaths.some(source => !source)) return { ok: false, error: 'The original image files are no longer available' };
      const requestId = `sequence-${Date.now()}-${++nextMediaRequest}`;
      const listener = (_event: IpcRendererEvent, progress: { requestId: string; completed: number }) => {
        if (progress.requestId === requestId) onProgress?.(progress.completed);
      };
      ipcRenderer.on(IPC.mediaSequenceProgress, listener);
      try {
        return await ipcRenderer.invoke(IPC.mediaSequenceCreate, { sourcePaths, fps, requestId }) as MediaProxyResult;
      } finally { ipcRenderer.removeListener(IPC.mediaSequenceProgress, listener); }
    },
    beginAnimation: (fps, repeats) => ipcRenderer.invoke(IPC.mediaAnimationBegin, { fps, repeats }) as Promise<string>,
    writeAnimationFrame: (token, index, offset, data) =>
      ipcRenderer.invoke(IPC.mediaAnimationFrame, { token, index, offset, data }) as Promise<void>,
    finishAnimation: async (token, onProgress) => {
      const requestId = `animation-${Date.now()}-${++nextMediaRequest}`;
      const listener = (_event: IpcRendererEvent, progress: { requestId: string; completed: number }) => {
        if (progress.requestId === requestId) onProgress?.(progress.completed);
      };
      ipcRenderer.on(IPC.mediaAnimationProgress, listener);
      try {
        return await ipcRenderer.invoke(IPC.mediaAnimationFinish, { token, requestId }) as MediaProxyResult;
      } finally { ipcRenderer.removeListener(IPC.mediaAnimationProgress, listener); }
    },
    createStillImage: (file, originalPath) => {
      const sourcePath = originalPath || webUtils.getPathForFile(file);
      if (!sourcePath) return Promise.resolve({ ok: false, error: 'The original file is no longer available' });
      return ipcRenderer.invoke(IPC.mediaImageCreate, { sourcePath, name: file.name }) as Promise<MediaProxyResult>;
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

  /* ── agent media tools: asset → readable file (media-tools lane) ── */
  mediaPath: {
    lookup: (request) => ipcRenderer.invoke(MEDIA_PATH_IPC.lookup, request),
    stageBegin: (request) => ipcRenderer.invoke(MEDIA_PATH_IPC.stageBegin, request),
    stageChunk: (token, offset, data) => ipcRenderer.invoke(MEDIA_PATH_IPC.stageChunk, { token, offset, data }) as Promise<void>,
    stageFinish: (token) => ipcRenderer.invoke(MEDIA_PATH_IPC.stageFinish, token),
    stageAbort: (token) => ipcRenderer.invoke(MEDIA_PATH_IPC.stageAbort, token) as Promise<void>,
    release: (projectId) => ipcRenderer.invoke(MEDIA_PATH_IPC.release, projectId) as Promise<void>
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
    answer: (req) => ipcRenderer.invoke(IPC.codexAnswer, req),
    cancel: (id, preserveChanges = false) => ipcRenderer.invoke(IPC.codexCancel, { id, preserveChanges }) as Promise<void>,
    cancelTask: (requestId, taskId) => ipcRenderer.invoke(IPC.codexCancelTask, { requestId, taskId }),
    fixPrompt: (req) => ipcRenderer.invoke(IPC.codexFixPrompt, req) as Promise<string>,
    rebasePrompt: (req) => ipcRenderer.invoke(IPC.codexRebasePrompt, req) as Promise<string>,
    updateFork: (req) => ipcRenderer.invoke(IPC.codexUpdateFork, req),
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
    },
    onInput: (cb) => {
      const listener = (_event: IpcRendererEvent, active: unknown): void => cb(active === true);
      ipcRenderer.on(IPC.agentToolInput, listener);
      return () => ipcRenderer.removeListener(IPC.agentToolInput, listener);
    }
  },

  chatgpt: {
    status: () => ipcRenderer.invoke(IPC.chatgptStatus) as Promise<ChatGPTAccountStatus>,
    models: () => ipcRenderer.invoke(IPC.chatgptModels) as Promise<CodexModelOption[]>,
    connect: () => ipcRenderer.invoke(IPC.chatgptConnect) as Promise<ChatGPTAccountStatus>,
    disconnect: () => ipcRenderer.invoke(IPC.chatgptDisconnect) as Promise<ChatGPTAccountStatus>,
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, status: ChatGPTAccountStatus): void => cb(status);
      ipcRenderer.on(IPC.chatgptChanged, listener);
      return () => ipcRenderer.removeListener(IPC.chatgptChanged, listener);
    }
  },

  agentRuntime: {
    update: (provider) => ipcRenderer.invoke(IPC.agentRuntimeUpdate, provider) as Promise<{ provider: 'claude' | 'codex'; version: string }>
  },

  claude: {
    status: () => ipcRenderer.invoke(IPC.claudeStatus) as Promise<ClaudeAccountStatus>,
    models: () => ipcRenderer.invoke(IPC.claudeModels) as Promise<ClaudeModelOption[]>,
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
    reveal: (ref) => ipcRenderer.invoke(IPC.artifactReveal, ref) as Promise<void>,
    forget: (projectId) => ipcRenderer.invoke(IPC.artifactForget, projectId) as Promise<void>,
    sweep: (liveProjectIds) => ipcRenderer.invoke(IPC.artifactSweep, liveProjectIds) as Promise<string[]>
  },

  captureWindow: () => ipcRenderer.invoke(IPC.captureWindow) as Promise<CaptureResult>,

  store: {
    setSerialized: (key, serialized) => ipcRenderer.invoke(IPC.storeSetSerialized, { key, serialized }),
    snapshotSync: () => ipcRenderer.sendSync(IPC.storeSnapshotSync) as StoreSnapshot,
    snapshotSerializedSync: () => ipcRenderer.sendSync(IPC.storeSnapshotSerializedSync) as Record<string, string>,
    bootstrapSerializedSync: () => ipcRenderer.sendSync(IPC.storeBootstrapSync) as Record<string, string>,
    getEncodedSync: (key) => ipcRenderer.sendSync(IPC.storeGetEncodedSync, { key }) as string | null,
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
    },
    getSync: (key) => ipcRenderer.sendSync(IPC.storeGetSync, { key }) as string | null,
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, payload: { keys?: string[] }): void =>
        cb(Array.isArray(payload?.keys) ? payload.keys : []);
      ipcRenderer.on(IPC.storeChanged, listener);
      return () => ipcRenderer.removeListener(IPC.storeChanged, listener);
    }
  },

  windows: {
    initialProject: () => ipcRenderer.sendSync(IPC.windowInitialProject) as WindowInitialProject,
    claimProject: (projectId) =>
      ipcRenderer.invoke(IPC.windowClaimProject, { projectId }) as Promise<WindowClaimResult>,
    releaseProject: (projectId) =>
      ipcRenderer.invoke(IPC.windowReleaseProject, { projectId }) as Promise<boolean>,
    reorderTabs: (order) =>
      ipcRenderer.invoke(IPC.windowReorderTabs, { order }) as Promise<boolean>,
    tabDropTarget: (point) =>
      ipcRenderer.invoke(IPC.windowTabDropTarget, point) as Promise<boolean>,
    placeTab: (projectId, point) =>
      ipcRenderer.invoke(IPC.windowPlaceTab, { projectId, ...point }) as Promise<WindowPlaceTabResult>,
    onAdoptTab: (cb) => {
      const listener = (_event: IpcRendererEvent, tab: WindowAdoptTab): void => cb(tab);
      ipcRenderer.on(IPC.windowAdoptTab, listener);
      return () => ipcRenderer.removeListener(IPC.windowAdoptTab, listener);
    },
    openProject: (projectId) =>
      ipcRenderer.invoke(IPC.windowOpenProject, { projectId }) as Promise<WindowOpenResult>,
    create: () => ipcRenderer.invoke(IPC.windowNew) as Promise<void>,
    close: () => {
      ipcRenderer.send(IPC.windowClose);
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
  menu: {
    popup: (request) => ipcRenderer.invoke(IPC.menuPopup, request) as Promise<string | null>
  },
  log: (level, text) => {
    ipcRenderer.send(IPC.log, { level, text });
  },
  openExternal: (url) => ipcRenderer.invoke(IPC.openExternal, url) as Promise<void>,
  extensionOpenExternal: (url) => ipcRenderer.invoke(IPC.extensionOpenExternal, url) as Promise<void>,
  remoteMedia: {
    fetch: (url) => ipcRenderer.invoke(IPC.remoteMediaFetch, url),
    read: (token, offset, length) => ipcRenderer.invoke(IPC.remoteMediaRead, { token, offset, length }),
    release: (token) => ipcRenderer.invoke(IPC.remoteMediaRelease, token)
  },
  nativeEdit: (action: NativeEditAction) => ipcRenderer.send(IPC.nativeEdit, action),
  updates: {
    status: () => ipcRenderer.invoke(IPC.updateStatus) as Promise<AppUpdateState>,
    check: () => ipcRenderer.invoke(IPC.updateCheck) as Promise<void>,
    install: () => ipcRenderer.invoke(IPC.updateInstall) as Promise<void>,
    onChanged: (cb) => {
      const listener = (_event: IpcRendererEvent, state: AppUpdateState): void => cb(state);
      ipcRenderer.on(IPC.updateChanged, listener);
      return () => ipcRenderer.removeListener(IPC.updateChanged, listener);
    }
  },
  whatsNew: {
    pending: () => ipcRenderer.invoke(IPC.whatsNewPending) as Promise<WhatsNew | null>,
    seen: () => ipcRenderer.invoke(IPC.whatsNewSeen) as Promise<void>
  },

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
    fork: (req) => ipcRenderer.invoke(IPC.extensionFork, req) as Promise<ExtensionForkResult>,
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
  },

  vars: {
    status: (req) => ipcRenderer.invoke(VARS_IPC.status, { id: req.id }) as Promise<VarsStatus>,
    set: (req) => ipcRenderer.invoke(VARS_IPC.set, { id: req.id, key: req.key, value: req.value }) as Promise<VarsStatus>,
    delete: (req) => ipcRenderer.invoke(VARS_IPC.delete, { id: req.id, key: req.key }) as Promise<VarsStatus>,
    reveal: (req) => ipcRenderer.invoke(VARS_IPC.reveal, { id: req.id, key: req.key }) as Promise<void>,
    values: (req) => ipcRenderer.invoke(VARS_IPC.values, { id: req.id }) as Promise<Record<string, string>>
  },

  cloud: {
    account: () => cloudInvoke(CLOUD_IPC.accountGet),
    signInSocial: (req) => cloudInvoke(CLOUD_IPC.signInSocial, { provider: req.provider }),
    emailSend: (req) => cloudInvoke(CLOUD_IPC.emailSend, { email: req.email }),
    emailVerify: (req) => cloudInvoke(CLOUD_IPC.emailVerify, { email: req.email, otp: req.otp }),
    claimHandle: (req) => cloudInvoke(CLOUD_IPC.claimHandle, { handle: req.handle }),
    setRememberInstalls: (req) => cloudInvoke(CLOUD_IPC.setRememberInstalls, { value: req.value }),
    signOut: () => cloudInvoke(CLOUD_IPC.signOut),
    deleteAccount: () => cloudInvoke(CLOUD_IPC.deleteAccount),
    registryUrl: () => cloudInvoke(CLOUD_IPC.registryUrl),
    setRegistryUrl: (req) => cloudInvoke(CLOUD_IPC.setRegistryUrl, { origin: req.origin }),
    onAccountChanged: cloudEvent(CLOUD_IPC.accountChanged),
    onSignInFailed: cloudEvent(CLOUD_IPC.signInFailed)
  },

  /* Each request is rebuilt field by field so nothing else a page put on the
     object crosses into main. */
  extensionStore: {
    browse: () => storeInvoke(STORE_IPC.browse),
    extensions: (req) => storeInvoke(STORE_IPC.extensions, {
      ...(req.category ? { category: req.category } : {}),
      ...(req.q ? { q: req.q } : {}),
      ...(req.cursor ? { cursor: req.cursor } : {}),
      ...(req.sort ? { sort: req.sort } : {})
    }),
    detail: (req) => storeInvoke(STORE_IPC.detail, { handle: req.handle, slug: req.slug }),
    release: (req) => storeInvoke(STORE_IPC.release, { releaseId: req.releaseId }),
    tree: (req) => storeInvoke(STORE_IPC.tree, { releaseId: req.releaseId }),
    file: (req) => storeInvoke(STORE_IPC.file, { handle: req.handle, slug: req.slug, version: req.version, path: req.path }),
    compare: (req) => storeInvoke(STORE_IPC.compare, { base: req.base, head: req.head }),
    install: (req) => storeInvoke(STORE_IPC.install, { repoId: req.repoId, releaseId: req.releaseId }),
    update: (req) => storeInvoke(STORE_IPC.update, { localId: req.localId }),
    uninstall: (req) => storeInvoke(STORE_IPC.uninstall, { localId: req.localId }),
    library: () => storeInvoke(STORE_IPC.library),
    checkUpdates: () => storeInvoke(STORE_IPC.checkUpdates),
    publishPrepare: (req) => storeInvoke(STORE_IPC.publishPrepare, { localId: req.localId }),
    publish: (req) => storeInvoke(STORE_IPC.publish, {
      localId: req.localId,
      form: {
        version: req.form.version,
        waivers: req.form.waivers.map((waiver) => ({ path: waiver.path, line: waiver.line, reason: waiver.reason })),
        ...(req.form.notes !== undefined ? { notes: req.form.notes } : {}),
        ...(req.form.listing ? {
          listing: { name: req.form.listing.name, tagline: req.form.listing.tagline, category: req.form.listing.category, licence: req.form.listing.licence }
        } : {}),
        ...(req.form.visibility ? { visibility: req.form.visibility } : {}),
        ...(req.form.iconPng ? { iconPng: req.form.iconPng } : {})
      }
    }),
    yank: (req) => storeInvoke(STORE_IPC.yank, { repoId: req.repoId, version: req.version }),
    trust: (req) => storeInvoke(STORE_IPC.trust, { localId: req.localId }),
    untrust: (req) => storeInvoke(STORE_IPC.untrust, { localId: req.localId }),
    onUpdatesChanged: cloudEvent(STORE_IPC.updatesChanged),
    onPublishProgress: cloudEvent(STORE_IPC.publishProgress),
    onLibraryChanged: cloudEvent<void>(STORE_IPC.libraryChanged)
  },

  /* ── transcription lane: on-device speech models and transcribe ── */
  transcription: {
    status: () => ipcRenderer.invoke(TRANSCRIPTION_IPC.status) as Promise<TranscriptionStatus>,
    download: (modelId) => ipcRenderer.invoke(TRANSCRIPTION_IPC.download, String(modelId)) as Promise<TranscriptionStatus>,
    cancelDownload: (modelId) => ipcRenderer.invoke(TRANSCRIPTION_IPC.cancelDownload, String(modelId)) as Promise<TranscriptionStatus>,
    remove: (modelId) => ipcRenderer.invoke(TRANSCRIPTION_IPC.remove, String(modelId)) as Promise<TranscriptionStatus>,
    setActive: (modelId) => ipcRenderer.invoke(TRANSCRIPTION_IPC.setActive, String(modelId)) as Promise<TranscriptionStatus>,
    setLanguage: (language) => ipcRenderer.invoke(TRANSCRIPTION_IPC.setLanguage, String(language)) as Promise<TranscriptionStatus>,
    reveal: () => ipcRenderer.invoke(TRANSCRIPTION_IPC.reveal) as Promise<void>,
    transcribe: (request) => ipcRenderer.invoke(TRANSCRIPTION_IPC.transcribe, {
      path: request.path,
      ...(request.start !== undefined ? { start: request.start } : {}),
      ...(request.end !== undefined ? { end: request.end } : {}),
      ...(request.language !== undefined ? { language: request.language } : {}),
      ...(request.requestId !== undefined ? { requestId: request.requestId } : {})
    }) as Promise<TranscribeResult>,
    cancelTranscribe: (requestId) => ipcRenderer.invoke(TRANSCRIPTION_IPC.cancelTranscribe, String(requestId)) as Promise<void>,
    onStatus: cloudEvent<TranscriptionStatus>(TRANSCRIPTION_IPC.statusChanged),
    onProgress: cloudEvent<TranscribeProgress>(TRANSCRIPTION_IPC.progress),
    onModelRequested: cloudEvent<{ reason: string }>(TRANSCRIPTION_IPC.modelRequested)
  }
};

/*
 * The bridge is handed to the page as a *configurable* global, not through
 * exposeInMainWorld (which pins a read-only, non-configurable property no
 * one can remove). The renderer's kernel reads it once at boot and deletes
 * it (renderer/src/kernel/capture-bridge.ts), so extension code never reaches raw
 * IPC. Arguments cross the context bridge exactly as exposeInMainWorld's do.
 */
contextBridge.executeInMainWorld({
  func: (value: unknown) => {
    Object.defineProperty(globalThis, 'powermove', { value, configurable: true, enumerable: false, writable: false });
  },
  args: [bridge]
});
