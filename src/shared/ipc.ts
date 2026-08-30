/*
 * Frozen Phase 1 IPC contract between the sandboxed renderer and the main
 * process. Every channel here is the ONLY way the renderer reaches native
 * capability. Binary payloads cross as Uint8Array (structured clone) — never
 * base64. Main re-validates every payload at the boundary (see guards.ts);
 * TypeScript types are documentation, not enforcement.
 */

export const IPC = {
  ping: 'app:ping',

  fileSave: 'file:save',
  projectOpen: 'project:open',
  projectConfirmClose: 'project:confirm-close',

  codexRun: 'codex:run',
  codexCancel: 'codex:cancel',
  codexFixPrompt: 'codex:fix-prompt',
  codexRestoreChangeSet: 'codex:restore-change-set',
  codexEvent: 'codex:event', // main → renderer
  chatgptStatus: 'chatgpt:status',
  chatgptConnect: 'chatgpt:connect',
  chatgptDisconnect: 'chatgpt:disconnect',
  chatgptChanged: 'chatgpt:changed', // main → renderer
  claudeStatus: 'claude:status',
  claudeConnect: 'claude:connect',
  claudeDisconnect: 'claude:disconnect',
  claudeChanged: 'claude:changed', // main → renderer
  consentComputer: 'consent:computer',

  artifactRead: 'agent:artifact:read',
  artifactReveal: 'agent:reveal',

  captureWindow: 'capture:window',

  storeSnapshot: 'store:snapshot',
  storeSnapshotSync: 'store:snapshot-sync', // ipcRenderer.sendSync from preload, boot barrier only
  storeSet: 'store:set',
  storeDelete: 'store:delete',
  storeFlush: 'store:flush',
  storeError: 'store:error', // main → renderer

  themeSet: 'theme:set',
  hapticAlignment: 'haptic:alignment',
  log: 'log',
  openExternal: 'shell:open-external',
  menuCommand: 'menu:command' // main → renderer
} as const;

export type IpcChannel = (typeof IPC)[keyof typeof IPC];

/* ── limits enforced in main ─────────────────────────────── */
export const LIMITS = {
  fileSaveBytes: 256 * 1024 * 1024, // larger payloads stream via File System Access
  artifactBytes: 64 * 1024 * 1024,
  codexImages: 6,
  codexImageBytes: 4 * 1024 * 1024,
  codexAttachments: 6,
  codexAttachmentBytes: 100 * 1024,
  codexPromptChars: 200_000,
  codexProjectJsonBytes: 24 * 1024 * 1024,
  codexProgressChars: 320,
  codexTraceChars: 2_000,
  storeValueBytes: 32 * 1024 * 1024,
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
export type ProjectOpenResult = { ok: true; path: string; projectId: string; text: string } | { ok: false; cancelled: boolean; error?: string };
export type CloseDecision = 'save' | 'discard' | 'cancel';

/* ── codex ───────────────────────────────────────────────── */
export type CodexMode = 'editor' | 'autonomous';
export type CodexAccess = 'editor' | 'project' | 'computer';
export type ReasoningEffort = 'low' | 'medium' | 'high';
export type AgentProviderId = 'chatgpt' | 'claude';

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
    }
  | { ok: false; error: string; cancelled: boolean };

export interface CodexCancelRequest {
  id: string;
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

export type CodexTraceEvent =
  | { kind: 'thought'; text: string }
  | { kind: 'answer'; text: string }
  | { kind: 'tool-start'; itemId: string; toolName: string; label: string }
  | { kind: 'tool-end'; itemId: string; isError: boolean };

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
export type MenuCommand = 'newProject' | 'save' | 'saveAs' | 'open' | 'export' | 'undo' | 'redo' | 'copy' | 'paste' | 'settings';

/* ── the preload surface ─────────────────────────────────── */
import type { ExtensionsBridge } from './extensions';

export interface PowermoveBridge {
  ping(): Promise<string>;
  versions: { electron: string; chrome: string; node: string };

  saveFile(req: FileSaveRequest): Promise<FileSaveResult>;
  openProjectFile(): Promise<ProjectOpenResult>;
  confirmProjectClose(name: string): Promise<CloseDecision>;

  codex: {
    run(
      req: CodexRunRequest,
      onProgress?: (text: string) => void,
      onTrace?: (step: CodexTraceEvent) => void
    ): Promise<CodexRunResult>;
    cancel(id: string): Promise<void>;
    fixPrompt(req: CodexFixPromptRequest): Promise<string>;
    restoreChangeSet(req: AgentChangeSetRestoreRequest): Promise<AgentChangeSetRestoreResult>;
    requestComputerConsent(req: ConsentRequest): Promise<ConsentResult>;
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
    snapshot(): Promise<StoreSnapshot>;
    set(key: string, value: unknown): void;
    delete(key: string): void;
    flush(): Promise<void>;
    onError(cb: (e: StoreErrorEvent) => void): () => void;
  };

  setTheme(theme: ThemeSource): void;
  haptic: {
    alignment(): void;
  };
  log(level: LogLevel, text: string): void;
  openExternal(url: string): Promise<void>;
  onMenuCommand(cb: (cmd: MenuCommand) => void): () => void;

  extensions: ExtensionsBridge;
}
