/*
 * Frozen Phase 1 IPC contract between the sandboxed renderer and the main
 * process. Every channel here is the ONLY way the renderer reaches native
 * capability. Binary payloads cross as Uint8Array (structured clone) — never
 * base64. Main re-validates every payload at the boundary (see guards.ts);
 * TypeScript types are documentation, not enforcement.
 */

export const IPC = {
  ping: 'app:ping',
  fontFamilies: 'fonts:families',

  onboardingAnimationComplete: 'onboarding:animation-complete',
  onboardingAnimationEnding: 'onboarding:animation-ending',
  onboardingAnimationFailed: 'onboarding:animation-failed',
  onboardingLogoTarget: 'onboarding:logo-target',
  onboardingLogoTargetReport: 'onboarding:logo-target-report',
  onboardingBegin: 'onboarding:begin',

  fileSave: 'file:save',
  fileSaveUpload: 'file:save-upload',
  fileSaveChunk: 'file:save-chunk',
  fileSaveAbort: 'file:save-abort',
  projectOpen: 'project:open',
  projectOpenExternal: 'project:open-external', // main → renderer
  projectRead: 'project:read',
  projectReadClose: 'project:read-close',
  projectConfirmClose: 'project:confirm-close',
  dialogConfirm: 'dialog:confirm',

  renderStart: 'render:start',
  renderWrite: 'render:write',
  renderFinish: 'render:finish',
  renderCancel: 'render:cancel',
  mediaProxyCreate: 'media-proxy:create',
  mediaPreviewBegin: 'media-preview:begin',
  mediaPreviewChunk: 'media-preview:chunk',
  mediaPreviewFinish: 'media-preview:finish',
  mediaSequenceCreate: 'media-sequence:create',
  mediaSequenceProgress: 'media-sequence:progress',
  mediaAnimationBegin: 'media-animation:begin',
  mediaAnimationFrame: 'media-animation:frame',
  mediaAnimationFinish: 'media-animation:finish',
  mediaAnimationProgress: 'media-animation:progress',
  mediaImageCreate: 'media-image:create',
  mediaProxyRead: 'media-proxy:read',
  mediaProxyRelease: 'media-proxy:release',
  mediaRevealSource: 'media:reveal-source',
  attachmentReveal: 'attachment:reveal',

  extensionFork: 'ext:fork',

  codexRun: 'codex:run',
  codexSteer: 'codex:steer',
  codexCancel: 'codex:cancel',
  codexFixPrompt: 'codex:fix-prompt',
  codexRebasePrompt: 'codex:rebase-prompt',
  codexRestoreChangeSet: 'codex:restore-change-set',
  codexEvent: 'codex:event', // main → renderer
  agentToolRequest: 'agent-tool:request', // main → renderer
  agentToolResponse: 'agent-tool:response', // renderer → main
  chatgptStatus: 'chatgpt:status',
  chatgptConnect: 'chatgpt:connect',
  chatgptDisconnect: 'chatgpt:disconnect',
  chatgptChanged: 'chatgpt:changed', // main → renderer
  claudeStatus: 'claude:status',
  claudeConnect: 'claude:connect',
  claudeDisconnect: 'claude:disconnect',
  claudeChanged: 'claude:changed', // main → renderer
  compatibleStatus: 'compatible:status',
  compatibleConfigure: 'compatible:configure',
  consentComputer: 'consent:computer',

  artifactRead: 'agent:artifact:read',
  artifactReveal: 'agent:reveal',

  captureWindow: 'capture:window',

  storeSnapshot: 'store:snapshot',
  storeSnapshotSerializedSync: 'store:snapshot-serialized-sync',
  storeSnapshotSync: 'store:snapshot-sync', // ipcRenderer.sendSync from preload, boot barrier only
  storeSet: 'store:set',
  storeSetSerialized: 'store:set-serialized',
  storeDelete: 'store:delete',
  storeFlush: 'store:flush',
  storeError: 'store:error', // main → renderer
  storeChanged: 'store:changed', // main → renderer, keys another window wrote
  storeGetSync: 'store:get-sync', // ipcRenderer.sendSync, refills one invalidated key

  windowInitialProject: 'window:initial-project', // ipcRenderer.sendSync, boot barrier only
  windowClaimProject: 'window:claim-project',
  windowOpenProject: 'window:open-project',
  windowNew: 'window:new',
  windowClose: 'window:close',

  themeSet: 'theme:set',
  hapticAlignment: 'haptic:alignment',
  menuPopup: 'menu:popup',
  log: 'log',
  openExternal: 'shell:open-external',
  nativeEdit: 'edit:native',
  menuCommand: 'menu:command', // main → renderer
  updateStatus: 'update:status',
  updateCheck: 'update:check',
  updateInstall: 'update:install',
  updateChanged: 'update:changed' // main → renderer
} as const;

/** Auto-update lifecycle as the renderer sees it. `ready` means Squirrel has
 * staged the new version and a normal quit installs it. */
export interface AppUpdateState {
  status: 'idle' | 'checking' | 'downloading' | 'ready' | 'error';
  /** Running app version. */
  current: string;
  /** Version being downloaded or staged, when known. */
  version: string | null;
}

/* ── windows ────────────────────────────────────── */
/** What a window is asked to show when it boots. `projectId` is null for a
 * window that picks its own project the way a single-window launch did; it then
 * has to skip everything in `taken`, because those documents belong to the
 * windows that already have them. */
export interface WindowInitialProject {
  projectId: string | null;
  taken: string[];
}

/** Outcome of asking for a project to be opened in its own window.
 * `focused` means another window already held it and was raised instead, so the
 * asking window must not load a second copy of the same document. */
export type WindowOpenResult =
  | { opened: true; focused: false }
  | { opened: false; focused: true }
  | { opened: false; focused: false; error: string };

/** Outcome of a window asking to take a project over as its own document.
 * A refused claim always means another window has it and was raised instead, so
 * the asking window keeps whatever it already had open. */
export interface WindowClaimResult {
  claimed: boolean;
  focused: boolean;
}

export interface OnboardingLogoTarget {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface OnboardingBridge {
  animationComplete(): void;
  animationEnding(): void;
  animationFailed(message: string): void;
  reportLogoTarget(target: OnboardingLogoTarget): void;
  onLogoTarget(callback: (target: OnboardingLogoTarget) => void): () => void;
  begin(): Promise<void>;
}

export type IpcChannel = (typeof IPC)[keyof typeof IPC];

/* ── limits enforced in main ─────────────────────────────── */
// Default store and agent-context allowance. Project persistence uses fileSaveBytes.
const PROJECT_SNAPSHOT_BYTES = 32 * 1024 * 1024;

export const LIMITS = {
  fileSaveBytes: 256 * 1024 * 1024, // direct IPC / local recovery bound
  codexImages: 6,
  codexImageBytes: 4 * 1024 * 1024,
  codexAttachments: 6,
  codexAttachmentBytes: 100 * 1024,
  codexPromptChars: 200_000,
  codexProjectJsonBytes: PROJECT_SNAPSHOT_BYTES,
  codexProgressChars: 320,
  codexTraceChars: 2_000,
  codexToolDetailChars: 200,
  codexToolOutputChars: 600,
  storeValueBytes: PROJECT_SNAPSHOT_BYTES,
  logChars: 8_000
} as const;

export const REQUEST_ID = /^[a-z0-9-]{8,80}$/;
export const PROJECT_ID = /^[A-Za-z0-9_-]{1,120}$/;

/* ── file ────────────────────────────────────────────────── */
export interface FileSaveRequest {
  name: string;
  data: Uint8Array;
  projectId?: string;
  saveAs?: boolean;
}
export type FileSaveResult = { ok: true; path: string } | { ok: false; cancelled: boolean; error?: string };
export type ProjectOpenResult = {
  ok: true; path: string; projectId: string; token: string; size: number;
  document: any; media: import('./project-container').ProjectMediaRange[];
} | { ok: true; path: string; projectId: string; data: Uint8Array }
  | { ok: false; cancelled: boolean; error?: string };
export type CloseDecision = 'save' | 'discard' | 'cancel';
export interface ConfirmRequest {
  message: string;
  detail?: string;
  /** Primary button label. Defaults to OK. */
  confirmLabel?: string;
  /** Marks the primary action as destructive; the sheet defaults focus to Cancel. */
  destructive?: boolean;
}

/* ── media playback proxies ─────────────────────────────── */
export interface MediaProxyRequest {
  sourcePath: string;
  name: string;
}
export type MediaProxyResult =
  | { ok: true; token: string; type: 'video/webm' | 'image/png'; size: number }
  | { ok: false; error: string };
export interface MediaProxyReadRequest {
  token: string;
  offset: number;
  length: number;
}

/* ── codex ───────────────────────────────────────────────── */
export type CodexMode = 'editor' | 'autonomous';
export type CodexAccess = 'editor' | 'project' | 'computer';
export type ReasoningEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max';
export type AgentProviderId = 'chatgpt' | 'claude' | 'compatible';

export interface CodexAttachment {
  name: string;
  data: Uint8Array;
}

export interface CodexRunRequest {
  id: string; // REQUEST_ID
  provider?: AgentProviderId; // omitted by older renderers; defaults to ChatGPT
  threadId?: string; // optional for older renderer clients; isolates resumed CLI sessions
  mode: CodexMode;
  prompt: string;
  schema: Record<string, unknown> | null;
  images: Uint8Array[]; // PNG/JPEG bytes, ≤ LIMITS.codexImages
  model: string | null;
  reasoningEffort: ReasoningEffort | null;
  access: CodexAccess;
  projectId: string; // PROJECT_ID
  projectName: string;
  projectJSON: string | null; // full project snapshot for autonomous mode
  attachments: CodexAttachment[];
  consentToken: string | null; // required when access === 'computer'
}

export interface AgentExtensionChange {
  id: string;
  action: 'created' | 'updated' | 'removed';
  summary?: string;
}

export interface AgentChangeSetRestoreRequest {
  projectId: string;
  changeSetId: string;
}

export interface AgentChangeSetRestoreResult {
  changeSetId: string;
  extensions: AgentExtensionChange[];
}

export type CodexRunResult =
  | {
      ok: true;
      text: string;
      access: Exclude<CodexAccess, 'editor'> | 'editor';
      extensions?: AgentExtensionChange[];
      /** Durable app-owned rollback record for promoted extension changes. */
      extensionChangeSetId?: string;
      /** The native provider used Powermove's live transactional tool layer. */
      liveEditsApplied?: boolean;
      /** One history entry containing every live edit made during this run. */
      liveEditHistoryId?: string;
    }
  | { ok: false; error: string; cancelled: boolean };

export interface CodexCancelRequest {
  id: string;
  /** Keep edits already committed through the live tool transaction. */
  preserveChanges?: boolean;
}

export interface CodexSteerRequest {
  id: string;
  prompt: string;
  images: Uint8Array[];
}

export interface CodexSteerResult {
  accepted: boolean;
}

export interface CodexFixPromptFile {
  path: string;
  text: string;
}

export interface CodexFixPromptRequest {
  id: string;
  error: string;
  files: CodexFixPromptFile[];
}

export interface CodexRebasePromptRequest {
  id: string;
}

/* Structured activity, streamed from main as it happens. Every `thought` and
   `answer` event is an APPEND-ONLY fragment: the renderer concatenates
   consecutive fragments into the current live row, so a provider that streams
   token deltas and one that only emits whole blocks look the same to the UI.
   Providers must emit each piece of text exactly once (never a delta AND the
   completed block) and must preserve inner whitespace so words do not fuse.
   `tool-start` is an UPSERT keyed by itemId: emit it the instant a call is
   known (label only), then again once the arguments are complete (with
   `detail`); the renderer patches the existing row instead of adding one. */
export type CodexTraceEvent =
  | { kind: 'thought'; text: string }
  | { kind: 'answer'; text: string }
  | {
      kind: 'tool-start';
      itemId: string;
      toolName: string;
      /** Short human label, e.g. "Read", "Edit", "Run", "Search". */
      label: string;
      /** The tool's primary argument in mono: a command, a file path, a query. ≤ LIMITS.codexToolDetailChars */
      detail?: string;
    }
  | {
      kind: 'tool-end';
      itemId: string;
      isError: boolean;
      /** Bounded excerpt of the tool result (first lines of output, diff stats, error text). ≤ LIMITS.codexToolOutputChars */
      output?: string;
    };

export type CodexProgressEvent =
  | {
      id: string;
      kind: 'progress';
      text: string; // already humanised, ≤ LIMITS.codexProgressChars
    }
  | {
      id: string;
      kind: 'trace';
      step: CodexTraceEvent; // main-vetted structured activity
    };

/* Native Codex/Claude harnesses call editor tools through main. The renderer
   owns project state, rendering, and Undo, so main only brokers bounded calls
   to the WebContents that owns the active agent run. */
export type AgentToolContent =
  | { type: 'text'; text: string }
  | { type: 'image'; data: Uint8Array; mimeType: 'image/png' | 'image/jpeg' };

export interface AgentToolRequestEvent {
  runId: string;
  callId: string;
  tool: string;
  arguments: Record<string, unknown>;
  baseRevision: number;
}

export interface AgentToolResponseEvent {
  runId: string;
  callId: string;
  ok: boolean;
  content: AgentToolContent[];
  error?: string;
  changed?: boolean;
  revision?: number;
  historyId?: string;
}

export type ChatGPTConnectionState = 'checking' | 'connected' | 'connecting' | 'disconnected' | 'unavailable';

export interface ChatGPTAccountStatus {
  state: ChatGPTConnectionState;
  email: string | null;
  planType: string | null;
  detail: string | null;
}

export type ClaudeConnectionState = ChatGPTConnectionState;
export interface ClaudeAccountStatus {
  state: ClaudeConnectionState;
  email: string | null;
  planType: string | null;
  detail: string | null;
}

/* Computer authority: main shows a native confirmation and mints a one-use,
   short-lived token. It is never persisted. */
export interface ConsentRequest {
  projectName: string;
  summary: string;
}
export type ConsentResult = { granted: true; token: string; expiresAt: number } | { granted: false };

/* ── agent artifacts ─────────────────────────────────────── */
export interface ArtifactRef {
  projectId: string;
  path: string; // "<runId>/<relative path>" — validated and contained in main
}
export interface ArtifactFile {
  name: string;
  mime: string;
  data: Uint8Array;
}

export interface AttachmentRevealRequest {
  name: string;
  data: Uint8Array;
}

/* ── capture ─────────────────────────────────────────────── */
export type CaptureResult = Uint8Array | null; // PNG bytes

/* ── store ───────────────────────────────────────────────── */
export type StoreSnapshot = Record<string, unknown>;
export interface StoreSetRequest {
  key: string; // validated by store-keys.ts
  value: unknown; // JSON-serialisable; main clones at receipt
}
export interface StoreDeleteRequest {
  key: string;
}
export interface StoreErrorEvent {
  key: string;
  error: string;
}

/* ── misc ────────────────────────────────────────────────── */
export type ThemeSource = 'light' | 'dark' | 'system';
export type LogLevel = 'info' | 'warn' | 'error' | 'uncaught';
export interface LogRequest {
  level: LogLevel;
  text: string;
}
export type MenuCommand =
  | 'newProject'
  | 'save'
  | 'saveAs'
  | 'open'
  | 'import'
  | 'importSequence'
  | 'export'
  | 'contextUndo'
  | 'contextRedo'
  | 'contextCut'
  | 'contextCopy'
  | 'contextPaste'
  | 'contextSelectAll'
  | 'duplicate'
  | 'split'
  | 'toggleVisibility'
  | 'toggleLayerControls'
  | 'bringForward'
  | 'sendBackward'
  | 'bringToFront'
  | 'sendToBack'
  | 'zoomIn'
  | 'zoomOut'
  | 'actualSize'
  | 'fitComposition'
  | 'fitView'
  | 'settings';
export type NativeEditAction = 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll';

export interface RemoteRunRecord {
  id: string;
  projectId: string;
  threadId: string | null;
  provider: string;
  mode: string;
  prompt: string;
  startedAt: number;
  finishedAt: number | null;
  result: CodexRunResult | null;
  eventCount: number;
}

/* ── the preload surface ─────────────────────────────────── */
import type { ExtensionsBridge } from './extensions';

export interface ExtensionForkRequest {
  id: string;
}

export interface ExtensionForkResult {
  id: string;
}

export interface PowermoveExtensionsBridge extends ExtensionsBridge {
  fork(req: ExtensionForkRequest): Promise<ExtensionForkResult>;
}

/** One row of a native context menu. Headers become disabled labels; NSMenu has no section titles. */
export type NativeMenuItem =
  | { type: 'separator' }
  | {
      type: 'normal' | 'checkbox' | 'header';
      id: string;
      label: string;
      enabled?: boolean;
      checked?: boolean;
      /** Electron accelerator string, shown as the key equivalent. */
      accelerator?: string;
      /** PNG data URL, 32×32 (2× of the 16pt slot), black on transparent; shown as a template image. */
      icon?: string;
    };

export type NativeMenuRequest = {
  items: NativeMenuItem[];
  /** Window-relative CSS pixels; omitted means "at the pointer". */
  x?: number;
  y?: number;
};

export interface PowermoveBridge {
  compatible?: {
    status(): Promise<import('./compatible-provider').CompatibleProviderConfig>;
    configure(input: import('./compatible-provider').CompatibleProviderInput): Promise<import('./compatible-provider').CompatibleProviderConfig>;
  };
  agentNotification(options: { sound: string; preview?: boolean }): Promise<void>;
  ping(): Promise<string>;
  fontFamilies?(): Promise<string[] | null>;
  versions: { electron: string; chrome: string; node: string };
  /** True when the page is served by `powermove serve` and the host is another machine. */
  remote?: boolean;
  /** Wraps the renderer's media store so imports reach the host and misses are filled from it. */
  wrapMediaStore?<T extends object>(store: T): T;
  /** Agent runs the host owns: they keep going without this tab. */
  remoteRuns?: {
    list(projectId: string): Promise<RemoteRunRecord[]>;
    /** Replays what happened so far, then streams, and resolves with the result. */
    attach(runId: string, hooks: { onProgress?(text: string): void; onTrace?(step: CodexTraceEvent): void }): Promise<CodexRunResult>;
  };

  fileUpload?: {
    begin(size: number): Promise<string>;
    chunk(uploadId: string, data: Uint8Array): Promise<void>;
    finish(uploadId: string, metadata: Omit<FileSaveRequest, 'data'>): Promise<FileSaveResult>;
    abort(uploadId: string): Promise<void>;
  };
  saveFile(req: FileSaveRequest): Promise<FileSaveResult>;
  openProjectFile(): Promise<ProjectOpenResult>;
  onProjectOpenExternal(cb: (result: ProjectOpenResult) => void): () => void;
  projectRead?: {
    read(token: string, offset: number, length: number): Promise<Uint8Array>;
    close(token: string): Promise<void>;
  };
  confirmProjectClose(name: string): Promise<CloseDecision>;
  /** Native NSAlert-style confirmation sheet. Resolves true when the primary button is chosen. */
  confirm(request: ConfirmRequest): Promise<boolean>;

  render: {
    start(options:{width:number;height:number;fps:number;format:'prores'|'mp4';alpha:boolean;name:string;bitrateMbps?:number}):Promise<string>;
    write(token:string,data:Uint8Array,audio?:boolean):Promise<void>;
    finish(token:string):Promise<{path?:string;cancelled?:boolean}>;
    cancel(token:string):Promise<void>;
  };
  media: {
    beginPreview(size: number): Promise<string>;
    writePreview(token: string, offset: number, data: Uint8Array): Promise<void>;
    finishPreview(token: string): Promise<{ token: string; size: number }>;
    sourcePath(file: File): string | null;
    revealSource(sourcePath: string): Promise<void>;
    createPlaybackProxy(file: File): Promise<MediaProxyResult>;
    createImageSequence(files: File[], fps: number, onProgress?: (completed: number) => void): Promise<MediaProxyResult>;
    /** Encode frames the renderer decoded from an animated image into a proxy. */
    beginAnimation(fps: number, repeats: number[]): Promise<string>;
    writeAnimationFrame(token: string, index: number, offset: number, data: Uint8Array): Promise<void>;
    finishAnimation(token: string, onProgress?: (completed: number) => void): Promise<MediaProxyResult>;
    /** Convert a still Chromium cannot decode, such as TIFF or HEIC, to PNG. */
    createStillImage(file: File): Promise<MediaProxyResult>;
    readPlaybackProxy(token: string, offset: number, length: number): Promise<Uint8Array>;
    releasePlaybackProxy(token: string): Promise<void>;
  };
  attachments: {
    /** Materialize bytes in app-owned cache storage and reveal that file. */
    reveal(request: AttachmentRevealRequest): Promise<void>;
  };

  codex: {
    run(
      req: CodexRunRequest,
      onProgress?: (text: string) => void,
      onTrace?: (step: CodexTraceEvent) => void
    ): Promise<CodexRunResult>;
    steer(req: CodexSteerRequest): Promise<CodexSteerResult>;
    cancel(id: string, preserveChanges?: boolean): Promise<void>;
    fixPrompt(req: CodexFixPromptRequest): Promise<string>;
    rebasePrompt(req: CodexRebasePromptRequest): Promise<string>;
    restoreChangeSet(req: AgentChangeSetRestoreRequest): Promise<AgentChangeSetRestoreResult>;
    requestComputerConsent(req: ConsentRequest): Promise<ConsentResult>;
  };

  agentTools: {
    onRequest(cb: (request: AgentToolRequestEvent) => void): () => void;
    respond(response: AgentToolResponseEvent): void;
  };

  chatgpt: {
    status(): Promise<ChatGPTAccountStatus>;
    connect(): Promise<ChatGPTAccountStatus>;
    disconnect(): Promise<ChatGPTAccountStatus>;
    onChanged(cb: (status: ChatGPTAccountStatus) => void): () => void;
  };

  claude: {
    status(): Promise<ClaudeAccountStatus>;
    connect(): Promise<ClaudeAccountStatus>;
    disconnect(): Promise<ClaudeAccountStatus>;
    onChanged(cb: (status: ClaudeAccountStatus) => void): () => void;
  };

  artifacts: {
    read(ref: ArtifactRef): Promise<ArtifactFile>;
    reveal(ref: ArtifactRef): Promise<void>;
  };

  captureWindow(): Promise<CaptureResult>;

  store: {
    /** Synchronous boot barrier: legacy classic scripts read PM.store during load. */
    snapshotSync(): StoreSnapshot;
    /** Keep large recovery trees out of contextBridge's recursive object copying. */
    snapshotSerializedSync?(): Record<string, string>;
    snapshot(): Promise<StoreSnapshot>;
    set(key: string, value: unknown): void;
    setSerialized?(key: string, serialized: string): Promise<void>;
    delete(key: string): void;
    flush(): Promise<void>;
    onError(cb: (e: StoreErrorEvent) => void): () => void;
    /** Re-reads one key another window wrote. Null when the key is unset. */
    getSync?(key: string): string | null;
    /** Keys another window wrote, so this one can drop them from its cache. */
    onChanged?(cb: (keys: string[]) => void): () => void;
  };

  /** This renderer's own window. One project per window; opening a document
   *  another window already has raises that window instead. */
  windows?: {
    /** The project this window was created for, read during boot. */
    initialProject(): WindowInitialProject;
    claimProject(projectId: string | null): Promise<WindowClaimResult>;
    openProject(projectId: string): Promise<WindowOpenResult>;
    create(): Promise<void>;
    close(): void;
  };

  setTheme(theme: ThemeSource): void;
  haptic: {
    alignment(): void;
  };
  /** Native NSMenu context menus. Resolves with the chosen item id, or null when dismissed. */
  menu?: {
    popup(request: NativeMenuRequest): Promise<string | null>;
  };
  log(level: LogLevel, text: string): void;
  openExternal(url: string): Promise<void>;
  nativeEdit(action: NativeEditAction): void;
  onMenuCommand(cb: (cmd: MenuCommand) => void): () => void;

  updates: {
    status(): Promise<AppUpdateState>;
    check(): Promise<void>;
    /** Quit normally (save barrier included), install the staged update, relaunch. */
    install(): Promise<void>;
    onChanged(cb: (state: AppUpdateState) => void): () => void;
  };

  extensions: PowermoveExtensionsBridge;
}
