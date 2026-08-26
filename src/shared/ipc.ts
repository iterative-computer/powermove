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

  codexRun: 'codex:run',
  codexCancel: 'codex:cancel',
  codexFixPrompt: 'codex:fix-prompt',
  codexEvent: 'codex:event', // main → renderer
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
}
export type FileSaveResult = { ok: true; path: string } | { ok: false; cancelled: boolean; error?: string };

/* ── codex ───────────────────────────────────────────────── */
export type CodexMode = 'editor' | 'autonomous';
export type CodexAccess = 'editor' | 'project' | 'computer';
export type ReasoningEffort = 'low' | 'medium' | 'high';

export interface CodexAttachment {
  name: string;
  data: Uint8Array;
}

export interface CodexRunRequest {
  id: string; // REQUEST_ID
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

export type CodexRunResult =
  | {
      ok: true;
      text: string;
      access: Exclude<CodexAccess, 'editor'> | 'editor';
      extensions?: AgentExtensionChange[];
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
export type MenuCommand = 'newProject' | 'save' | 'open' | 'export' | 'undo' | 'redo' | 'settings';

/* ── the preload surface ─────────────────────────────────── */
import type { ExtensionsBridge } from './extensions';

export interface PowermoveBridge {
  ping(): Promise<string>;
  versions: { electron: string; chrome: string; node: string };

  saveFile(req: FileSaveRequest): Promise<FileSaveResult>;

  codex: {
    run(
      req: CodexRunRequest,
      onProgress?: (text: string) => void,
      onTrace?: (step: CodexTraceEvent) => void
    ): Promise<CodexRunResult>;
    cancel(id: string): Promise<void>;
    fixPrompt(req: CodexFixPromptRequest): Promise<string>;
    requestComputerConsent(req: ConsentRequest): Promise<ConsentResult>;
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
