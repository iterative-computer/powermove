/*
 * The browser's stand-in for the Electron preload. When the app is served by
 * `powermove serve`, there is no contextBridge; this module speaks the same
 * IPC contract over one WebSocket and installs itself as window.powermove
 * before the engines boot, so the rest of the renderer does not know the
 * difference.
 *
 * What cannot cross the network stays in the browser: confirmations are
 * window.confirm, saves become downloads, dropped files are uploaded first,
 * notifications use the Notification API.
 */
import { IPC, type AgentToolRequestEvent, type CodexRunResult, type FileSaveResult, type MediaProxyResult, type PowermoveBridge, type ProjectOpenResult, type RemoteRunRecord, type StoreErrorEvent } from '../../../shared/ipc';
import { EXT_IPC, type ExtensionRecord } from '../../../shared/extensions';
import { WEB, WEB_UPLOAD_CHUNK_BYTES, type WebHello } from '../../../shared/wire';
import { Connection } from '../../../shared/link';
import { attachRemoteMedia } from './remote-media';

const WS_PATH = '/__powermove/ws';

/** The live connection, for modules that attach after the engines boot (remote-sync). */
let activeLink: Connection | null = null;
export function remoteLink(): Connection | null { return activeLink; }
const CONNECT_TIMEOUT_MS = 4000;

function connect(url: string): Promise<Connection> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const timer = setTimeout(() => { socket.close(); reject(new Error('timeout')); }, CONNECT_TIMEOUT_MS);
    socket.addEventListener('open', () => { clearTimeout(timer); resolve(new Connection(socket)); }, { once: true });
    socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('unreachable')); }, { once: true });
  });
}

/* ── browser-side helpers ─────────────────────────────────── */

function download(pathOnHost: string, name: string): void {
  const anchor = document.createElement('a');
  anchor.href = `/__powermove/download?path=${encodeURIComponent(pathOnHost)}`;
  anchor.download = name;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
}

function pickFile(accept: string, multiple = false): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.multiple = multiple;
    input.style.display = 'none';
    document.body.appendChild(input);
    const finish = (files: File[]) => { input.remove(); resolve(files); };
    input.addEventListener('change', () => finish([...(input.files ?? [])]), { once: true });
    input.addEventListener('cancel', () => finish([]), { once: true });
    input.click();
  });
}

type Toast = (text: string, ms?: number, options?: { error?: boolean }) => void;
const toast: Toast = (text, ms, options) => {
  const pm = (window as unknown as { PM?: { toast?: Toast } }).PM;
  if (pm?.toast) pm.toast(text, ms, options); else console.info(`[powermove] ${text}`);
};

const UPLOAD_PARALLEL = 4;

/** A server-side path for a browser File: uploaded in 1 MiB chunks, a few in flight. */
async function upload(link: Connection, file: File, kind: 'media' | 'project'): Promise<string> {
  const label = file.name || (kind === 'project' ? 'project' : 'media');
  const megabytes = (file.size / 1048576).toFixed(file.size < 10 * 1048576 ? 1 : 0);
  toast(`Uploading ${label} (${megabytes} MB) to the host…`, 60_000, { error: false });
  const id = await link.invoke<string>(WEB.uploadBegin, { name: file.name, size: file.size, kind });
  try {
    let sent = 0, lastShown = -1;
    const offsets: number[] = [];
    for (let offset = 0; offset < file.size; offset += WEB_UPLOAD_CHUNK_BYTES) offsets.push(offset);
    let next = 0;
    const worker = async () => {
      while (next < offsets.length) {
        const offset = offsets[next++]!;
        const data = new Uint8Array(await file.slice(offset, offset + WEB_UPLOAD_CHUNK_BYTES).arrayBuffer());
        await link.invoke(WEB.uploadChunk, { id, offset, data });
        sent += data.byteLength;
        const percent = Math.floor((sent / file.size) * 100);
        if (percent !== lastShown && (percent - lastShown >= 5 || percent === 100)) {
          lastShown = percent;
          toast(`Uploading ${label}… ${percent}%`, 60_000, { error: false });
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(UPLOAD_PARALLEL, offsets.length || 1) }, worker));
    const hostPath = await link.invoke<string>(WEB.uploadFinish, id);
    toast(`Uploaded ${label}. Opening on the host…`, 4000, { error: false });
    return hostPath;
  } catch (error) {
    await link.invoke(WEB.uploadAbort, id).catch(() => undefined);
    toast(`Upload of ${label} failed: ${error instanceof Error ? error.message : String(error)}`, 8000);
    throw error;
  }
}

function withProgress<T>(link: Connection, channel: string, requestId: string, onProgress: ((completed: number) => void) | undefined, run: () => Promise<T>): Promise<T> {
  const off = link.on(channel, (progress) => {
    const value = progress as { requestId?: string; completed?: number };
    if (value?.requestId === requestId && typeof value.completed === 'number') onProgress?.(value.completed);
  });
  return run().finally(off);
}

/** Generated extension bundles are served by the host at /ext/…, not app://. */
function localizeRecords(records: ExtensionRecord[]): ExtensionRecord[] {
  return records.map((record) => record.bundleUrl?.startsWith('app://powermove/')
    ? { ...record, bundleUrl: `${location.origin}/${record.bundleUrl.slice('app://powermove/'.length)}` }
    : record);
}

const IS_PMV = /\.pmv$/i;
let nextMediaRequest = 0;

/* ── the bridge ───────────────────────────────────────────── */

function createBridge(link: Connection, hello: WebHello, storeSnapshot: Record<string, string>, initialProject: { projectId: string | null; taken: string[] }): PowermoveBridge {
  // Keys another tab wrote arrive as names; the value is re-read before the
  // shim is told, so its synchronous refill sees the fresh string.
  const serialized = new Map(Object.entries(storeSnapshot));
  const storeChangedListeners = new Set<(keys: string[]) => void>();
  link.on(IPC.storeChanged, (payload) => {
    const keys = Array.isArray((payload as { keys?: unknown })?.keys) ? ((payload as { keys: unknown[] }).keys.filter((key): key is string => typeof key === 'string')) : [];
    if (!keys.length) return;
    void Promise.all(keys.map(async (key) => {
      const value = await link.sync<string | null>(IPC.storeGetSync, { key }).catch(() => null);
      if (value === null) serialized.delete(key); else serialized.set(key, value);
    })).then(() => { for (const listener of storeChangedListeners) listener(keys); });
  });

  const subscribe = <T,>(channel: string) => (cb: (value: T) => void) => link.on(channel, (value) => cb(value as T));
  const notify = (title: string, body: string) => {
    if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    try { new Notification(title, { body, silent: true }); } catch { /* not permitted */ }
  };

  const bridge: PowermoveBridge = {
    compatible: {
      status: () => link.invoke(IPC.compatibleStatus),
      configure: (input) => link.invoke(IPC.compatibleConfigure, input)
    },
    agentNotification: async (options) => {
      if (options.preview) return;
      if (typeof Notification !== 'undefined' && Notification.permission === 'default') await Notification.requestPermission().catch(() => undefined);
      notify('Powermove', 'Your agent has finished. Your result is ready.');
    },
    ping: () => link.invoke<string>(IPC.ping),
    remote: true,
    wrapMediaStore: (store, onChange) => attachRemoteMedia(link, store as never, onChange) as never,
    remoteRuns: {
      list: (projectId) => link.invoke<RemoteRunRecord[]>(WEB.runsList, projectId),
      attach: (runId, hooks) => new Promise<CodexRunResult>((resolve, reject) => {
        const offEvents = link.on(IPC.codexEvent, (progress) => {
          const event = progress as { id: string; kind: 'progress' | 'trace'; text?: string; step?: unknown };
          if (event.id !== runId) return;
          if (event.kind === 'progress') hooks.onProgress?.(event.text ?? ''); else hooks.onTrace?.(event.step as never);
        });
        const offFinish = link.on(WEB.runFinished, (payload) => {
          const finished = payload as { id: string; result: CodexRunResult };
          if (finished.id !== runId) return;
          offEvents(); offFinish();
          resolve(finished.result);
        });
        link.invoke(WEB.runsAttach, runId).catch((error) => { offEvents(); offFinish(); reject(error); });
      })
    },
    versions: { electron: '', chrome: /Chrome\/(\S+)/.exec(navigator.userAgent)?.[1] ?? '', node: hello.node },

    fileUpload: {
      begin: (size) => link.invoke(IPC.fileSaveUpload, size),
      chunk: (uploadId, data) => link.invoke(IPC.fileSaveChunk, { uploadId, data }),
      finish: async (uploadId, metadata) => {
        const result = await link.invoke<FileSaveResult>(IPC.fileSave, { ...metadata, uploadId });
        if (result.ok && !IS_PMV.test(result.path)) download(result.path, metadata.name);
        return result;
      },
      abort: (uploadId) => link.invoke(IPC.fileSaveAbort, uploadId)
    },
    saveFile: async (req) => {
      let result: FileSaveResult;
      if (req.data.byteLength <= 4 * 1024 * 1024) {
        result = await link.invoke<FileSaveResult>(IPC.fileSave, req);
      } else {
        const uploadId = await link.invoke<string>(IPC.fileSaveUpload, req.data.byteLength);
        try {
          for (let offset = 0; offset < req.data.byteLength; offset += 1024 * 1024) {
            await link.invoke(IPC.fileSaveChunk, { uploadId, data: req.data.slice(offset, offset + 1024 * 1024) });
          }
          const { data: _data, ...metadata } = req;
          result = await link.invoke<FileSaveResult>(IPC.fileSave, { ...metadata, uploadId });
        } finally { await link.invoke(IPC.fileSaveAbort, uploadId).catch(() => undefined); }
      }
      // Project files stay on the host; exports also come down to this machine.
      if (result.ok && !IS_PMV.test(result.path)) download(result.path, req.name);
      return result;
    },
    openProjectFile: async () => {
      const [file] = await pickFile('.pmv,.json');
      if (!file) { toast('No project file was chosen.', 2500); return { ok: false, cancelled: true }; }
      try {
        const hostPath = await upload(link, file, 'project');
        return await link.invoke<ProjectOpenResult>(WEB.projectOpenPath, hostPath);
      } catch (error) {
        return { ok: false, cancelled: false, error: error instanceof Error ? error.message : 'Could not open project.' };
      }
    },
    onProjectOpenExternal: subscribe(IPC.projectOpenExternal),
    projectRead: {
      read: (token, offset, length) => link.invoke(IPC.projectRead, { token, offset, length }),
      close: (token) => link.invoke(IPC.projectReadClose, token)
    },
    confirmProjectClose: (name) => link.invoke(IPC.projectConfirmClose, name),
    confirm: async (request) => {
      const detail = request.detail ? `\n\n${request.detail}` : '';
      return window.confirm(`${request.message}${detail}`);
    },

    render: {
      start: (options) => link.invoke(IPC.renderStart, options),
      write: (token, data, audio) => link.invoke(IPC.renderWrite, { token, data, audio }),
      finish: async (token) => {
        const result = await link.invoke<{ path?: string; cancelled?: boolean }>(IPC.renderFinish, { token });
        if (result.path) download(result.path, result.path.split('/').pop() ?? 'render');
        return result;
      },
      cancel: (token) => link.invoke(IPC.renderCancel, { token })
    },
    media: {
      // Browser projects restore media through the host's media store. Native
      // source paths and cloud placeholders belong to the desktop that saved them.
      cloudStatus: async () => ({}),
      openLocalSource: async () => null,
      cloudPrompt: async () => ({ download: false, automatic: false }),
      downloadCloudSource: async () => { throw new Error('Download cloud media in the desktop app, then save the project.'); },
      readCloudSource: async () => { throw new Error('Native media sources are unavailable in the browser.'); },
      releaseCloudSource: async () => {},
      beginPreview: (size) => link.invoke(IPC.mediaPreviewBegin, size),
      writePreview: (token, offset, data) => link.invoke(IPC.mediaPreviewChunk, { token, offset, data }),
      finishPreview: (token) => link.invoke(IPC.mediaPreviewFinish, token),
      sourcePath: () => null,
      revealSource: () => Promise.resolve(),
      createPlaybackProxy: async (file) => {
        try {
          const sourcePath = await upload(link, file, 'media');
          return await link.invoke<MediaProxyResult>(IPC.mediaProxyCreate, { sourcePath, name: file.name });
        } catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Upload failed' }; }
      },
      createImageSequence: async (files, fps, onProgress) => {
        try {
          const sourcePaths: string[] = [];
          for (const file of files) sourcePaths.push(await upload(link, file, 'media'));
          const requestId = `sequence-${Date.now()}-${++nextMediaRequest}`;
          return await withProgress(link, IPC.mediaSequenceProgress, requestId, onProgress, () =>
            link.invoke<MediaProxyResult>(IPC.mediaSequenceCreate, { sourcePaths, fps, requestId }));
        } catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Upload failed' }; }
      },
      beginAnimation: (fps, repeats) => link.invoke(IPC.mediaAnimationBegin, { fps, repeats }),
      writeAnimationFrame: (token, index, offset, data) => link.invoke(IPC.mediaAnimationFrame, { token, index, offset, data }),
      finishAnimation: (token, onProgress) => {
        const requestId = `animation-${Date.now()}-${++nextMediaRequest}`;
        return withProgress(link, IPC.mediaAnimationProgress, requestId, onProgress, () =>
          link.invoke<MediaProxyResult>(IPC.mediaAnimationFinish, { token, requestId }));
      },
      createStillImage: async (file) => {
        try {
          const sourcePath = await upload(link, file, 'media');
          return await link.invoke<MediaProxyResult>(IPC.mediaImageCreate, { sourcePath, name: file.name });
        } catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Upload failed' }; }
      },
      readPlaybackProxy: (token, offset, length) => link.invoke(IPC.mediaProxyRead, { token, offset, length }),
      releasePlaybackProxy: (token) => link.invoke(IPC.mediaProxyRelease, token)
    },
    attachments: {
      // Nothing to reveal on this machine: hand the bytes to the browser instead.
      reveal: async (request) => {
        const url = URL.createObjectURL(new Blob([request.data as BlobPart]));
        const anchor = document.createElement('a');
        anchor.href = url; anchor.download = request.name; anchor.rel = 'noopener';
        document.body.appendChild(anchor); anchor.click(); anchor.remove();
        setTimeout(() => URL.revokeObjectURL(url), 10_000);
      }
    },

    codex: {
      async run(req, onProgress, onTrace) {
        const off = link.on(IPC.codexEvent, (progress) => {
          const event = progress as { id: string; kind: 'progress' | 'trace'; text?: string; step?: unknown };
          if (event.id !== req.id) return;
          if (event.kind === 'progress') onProgress?.(event.text ?? ''); else onTrace?.(event.step as never);
        });
        try { return await link.invoke(IPC.codexRun, req); } finally { off(); }
      },
      steer: (req) => link.invoke(IPC.codexSteer, req),
      cancel: (id, preserveChanges = false) => link.invoke(IPC.codexCancel, { id, preserveChanges }),
      fixPrompt: (req) => link.invoke(IPC.codexFixPrompt, req),
      rebasePrompt: (req) => link.invoke(IPC.codexRebasePrompt, req),
      restoreChangeSet: (req) => link.invoke(IPC.codexRestoreChangeSet, req),
      requestComputerConsent: (req) => link.invoke(IPC.consentComputer, req)
    },

    agentTools: {
      onRequest: (cb) => link.on(IPC.agentToolRequest, (request) => cb(request as AgentToolRequestEvent)),
      respond: (response) => link.send(IPC.agentToolResponse, response)
    },

    chatgpt: {
      status: () => link.invoke(IPC.chatgptStatus),
      models: () => link.invoke(IPC.chatgptModels),
      connect: () => link.invoke(IPC.chatgptConnect),
      disconnect: () => link.invoke(IPC.chatgptDisconnect),
      onChanged: subscribe(IPC.chatgptChanged)
    },
    claude: {
      status: () => link.invoke(IPC.claudeStatus),
      connect: () => link.invoke(IPC.claudeConnect),
      disconnect: () => link.invoke(IPC.claudeDisconnect),
      onChanged: subscribe(IPC.claudeChanged)
    },
    artifacts: {
      read: (ref) => link.invoke(IPC.artifactRead, ref),
      reveal: (ref) => link.invoke(IPC.artifactReveal, ref)
    },

    captureWindow: () => Promise.resolve(null),

    store: {
      snapshotSync: () => Object.fromEntries([...serialized].map(([key, value]) => [key, JSON.parse(value)])),
      snapshotSerializedSync: () => Object.fromEntries(serialized),
      snapshot: () => link.invoke(IPC.storeSnapshot),
      set: (key, value) => { serialized.set(key, JSON.stringify(value)); link.send(IPC.storeSet, { key, value }); },
      setSerialized: async (key, value) => { serialized.set(key, value); await link.invoke(IPC.storeSetSerialized, { key, serialized: value }); },
      delete: (key) => { serialized.delete(key); link.send(IPC.storeDelete, { key }); },
      flush: () => link.invoke(IPC.storeFlush),
      onError: (cb) => link.on(IPC.storeError, (error) => cb(error as StoreErrorEvent)),
      getSync: (key) => serialized.get(key) ?? null,
      onChanged: (cb) => { storeChangedListeners.add(cb); return () => { storeChangedListeners.delete(cb); }; }
    },

    windows: {
      initialProject: () => initialProject,
      claimProject: (projectId) => link.invoke(IPC.windowClaimProject, { projectId }),
      openProject: (projectId) => link.invoke(IPC.windowOpenProject, { projectId }),
      create: () => link.invoke(IPC.windowNew),
      close: () => link.send(IPC.windowClose)
    },

    setTheme: () => {},
    haptic: { alignment: () => {} },
    // No `menu`: the overlay Menu falls back to its own listbox.
    log: (level, text) => link.send(IPC.log, { level, text }),
    openExternal: async (url) => { window.open(url, '_blank', 'noopener'); },
    nativeEdit: (action) => { try { document.execCommand(action === 'selectAll' ? 'selectAll' : action); } catch { /* unsupported */ } },
    onMenuCommand: () => () => {},
    updates: {
      status: () => link.invoke(WEB.updateStatus),
      check: () => link.invoke(WEB.updateCheck),
      install: async () => {
        const result = await link.invoke<{ restarting: true } | { restarting: false; command: string }>(WEB.updateInstall);
        if (result.restarting) { toast('Updating the host… this tab reconnects when it is back.', 8000, { error: false }); return; }
        // The host cannot replace itself the way it was started: hand over the command.
        try { await navigator.clipboard.writeText(result.command); } catch { /* clipboard may be blocked */ }
        toast(`To update, run this on the host (copied): ${result.command}`, 12_000, { error: false });
      },
      onChanged: subscribe(WEB.updateChanged)
    },

    extensions: {
      list: async () => localizeRecords(await link.invoke<ExtensionRecord[]>(EXT_IPC.list)),
      setEnabled: async (req) => localizeRecords(await link.invoke<ExtensionRecord[]>(EXT_IPC.setEnabled, req)),
      remove: async (req) => localizeRecords(await link.invoke<ExtensionRecord[]>(EXT_IPC.remove, req)),
      reload: async (req) => localizeRecords(await link.invoke<ExtensionRecord[]>(EXT_IPC.reload, req)),
      create: async (req) => localizeRecords(await link.invoke<ExtensionRecord[]>(EXT_IPC.create, req)),
      fork: (req) => link.invoke(IPC.extensionFork, req),
      reveal: (req) => link.invoke(EXT_IPC.reveal, req),
      readSource: (req) => link.invoke(EXT_IPC.readSource, req),
      reportHealth: (req) => link.send(EXT_IPC.reportHealth, req),
      onChanged: subscribe(EXT_IPC.changed)
    }
  };

  /* server-initiated */
  link.answer(WEB.askMessageBox, (box) => {
    const { message, detail, buttons, cancelId } = box as { message: string; detail: string | null; buttons: string[]; cancelId: number };
    // Two-button boxes map onto confirm(); with three, OK means the primary and
    // Cancel means cancel — the middle option is not reachable from a browser.
    const primary = buttons.findIndex((_label, index) => index !== cancelId);
    const accepted = window.confirm(`${message}${detail ? `\n\n${detail}` : ''}\n\nOK = ${buttons[primary] ?? 'OK'}`);
    return accepted ? Math.max(primary, 0) : cancelId;
  });
  link.on(WEB.openExternal, (url) => { if (typeof url === 'string') window.open(url, '_blank', 'noopener'); });
  link.on(WEB.openWindow, (projectId) => {
    const target = new URL(location.href);
    target.search = typeof projectId === 'string' && projectId ? `?project=${encodeURIComponent(projectId)}` : '';
    window.open(target.toString(), '_blank');
  });
  link.on(WEB.focusWindow, () => window.focus());
  link.on(WEB.closeWindow, () => window.close());
  link.on('__closed', () => {
    document.documentElement.classList.add('host-disconnected');
    (window as unknown as { PM?: { toast?: (text: string, ms?: number) => void } }).PM?.toast?.('Lost the connection to the Powermove host. Reload to reconnect.', 8000);
  });

  return bridge;
}

/**
 * Install window.powermove when the page is served by `powermove serve`.
 * Resolves immediately in Electron (the preload already installed it) and
 * after a short failed probe in a plain browser without a host.
 */
export async function installWebBridge(): Promise<boolean> {
  const scope = window as unknown as { powermove?: PowermoveBridge };
  if (scope.powermove) return false;
  if (location.protocol !== 'http:' && location.protocol !== 'https:') return false;
  const params = new URLSearchParams(location.search);
  const project = params.get('project');
  const url = new URL(WS_PATH, location.href);
  url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  if (project) url.searchParams.set('project', project);
  let link: Connection;
  try { link = await connect(url.toString()); } catch { return false; }
  const [hello, snapshot, initialProject] = await Promise.all([
    link.invoke<WebHello>(WEB.hello),
    link.sync<Record<string, string>>(IPC.storeSnapshotSerializedSync),
    link.sync<{ projectId: string | null; taken: string[] }>(IPC.windowInitialProject)
  ]);
  activeLink = link;
  scope.powermove = createBridge(link, hello, snapshot, initialProject);
  document.documentElement.classList.add('remote-app');
  if (project) history.replaceState(null, '', location.pathname);
  return true;
}
