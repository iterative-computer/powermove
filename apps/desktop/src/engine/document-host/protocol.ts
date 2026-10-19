/*
 * The document worker interface (contract 15, plan section 22;
 * docs/engine/document-host.md). Frozen at version 1 for Phase 3c: the
 * serve host's document engine (engine/main.ts) and the hidden MCP host
 * (agents lane) both drive per-project workers through it.
 *
 * Three parties:
 *
 * - The **embedder** (the serve supervisor, the hidden MCP host) owns a
 *   `DocumentHostPool` and implements `DocumentHostServices`: the profile
 *   store, the project session (the document every member shares), media
 *   paths, and transcription. Nothing else crosses into a worker.
 * - The **pool** (pool.ts) starts one worker process per open project,
 *   routes tool calls by project, restarts a crashed worker with backoff,
 *   and answers every failure with a named `DocumentHostFailure`.
 * - The **worker** (worker.ts) is one project: its own pm-wasm mirror,
 *   kernel, history, agent harness, captions service, and extension
 *   registry, in a process of its own (a crash in one project never takes
 *   another one down; CoreText's font registry needs a main thread, which a
 *   worker thread does not have).
 *
 * Tool arguments and results are the tools' existing JSON
 * (`AgentToolRequestEvent` / `AgentToolResponseEvent`, shared/ipc.ts): the
 * contract types `ToolCall` / `ToolResult` (shared/engine-protocol) name the
 * same fields (`run` is `runId`, `isError` is `!ok`, `revision` is the
 * document revision after the call).
 *
 * Versioning: adding an optional field or a service method the worker does
 * not require is compatible (the `extensions` service and its
 * `extensionsChanged` message were added this way). Changing a message, a failure code, or which
 * side runs a tool bumps `DOCUMENT_HOST_PROTOCOL`.
 */
import type { AgentToolRequestEvent, AgentToolResponseEvent } from '../../shared/ipc';
import type { MediaPathLookupRequest, MediaPathResult } from '../../shared/media-tools';
import type { TranscribeProgress, TranscribeRequest, TranscribeResult, TranscriptionStatus } from '../../shared/transcription';
import type { Patch } from '../../shared/patch';
import type { ExtensionRecord, ExtensionsChangedEvent } from '../../shared/extensions';
import type { ExtensionRegistered, PersistReason, ProjectOpened } from '../../shared/engine-protocol';

export const DOCUMENT_HOST_PROTOCOL = 1;

/** The profile-store key a worker persists its instance state under. */
export const INSTANCE_STATE_KEY = (projectId: string): string => `documentHost.${projectId}`;

/* ── failures ────────────────────────────────────────────────────────── */

/**
 * Every failure a pool or worker reports. `retry` says whether calling
 * again can succeed (the pool restarts or re-checks first); the message is
 * the one an agent or a person reads.
 */
export type DocumentHostFailureCode =
  /** pm-wasm is missing or failed to load with the new engine on. */
  | 'pm-wasm-unavailable'
  /** The native font service (pm-bridge's registry) is missing or failed. */
  | 'font-service-unavailable'
  /** The host has no document for the project. */
  | 'project-missing'
  /** The project is stamped `requiresEngine: "native"` and the new engine is off. */
  | 'requires-new-engine'
  /** The worker process exited or could not start. */
  | 'worker-crashed'
  /** The worker did not answer in time. */
  | 'worker-timeout'
  /** The pool is shutting down or the instance was closed. */
  | 'closed'
  /** The tool belongs to another route (contract 15's `route_tool`). */
  | 'wrong-route'
  /**
   * A contained runtime could not start (sandbox.ts: the runtime bundle is
   * not built). Store extensions and scripts never run uncontained.
   */
  | 'sandbox-unavailable'
  /** A host service the call needs is not provided by the embedder. */
  | 'service-unavailable'
  /** The worker answered with an error of its own. */
  | 'worker-error';

export interface DocumentHostFailure {
  code: DocumentHostFailureCode;
  message: string;
  retry: boolean;
}

export class DocumentHostError extends Error implements DocumentHostFailure {
  constructor(readonly code: DocumentHostFailureCode, message: string, readonly retry = false) {
    super(message);
    this.name = 'DocumentHostError';
  }
  toJSON(): DocumentHostFailure { return { code: this.code, message: this.message, retry: this.retry }; }
  static from(failure: DocumentHostFailure): DocumentHostError { return new DocumentHostError(failure.code, failure.message, failure.retry); }
}

export const isFailure = (value: unknown): value is DocumentHostFailure =>
  !!value && typeof value === 'object' && typeof (value as DocumentHostFailure).code === 'string' && typeof (value as DocumentHostFailure).message === 'string';

/* ── what the embedder provides ──────────────────────────────────────── */

export interface SessionJoin { seq: number; doc: unknown | null }

/** Remote edits another session member made (another tab, a device). */
export interface SessionPatch { projectId: string; seq: number; patches: Patch[] }

/**
 * The embedder's side. Every method may reject; the worker turns a
 * rejection into a named failure of the call that needed it.
 */
export interface DocumentHostServices {
  /** The profile store (main's StorageEngine on the desktop, the CLI host's store). */
  store: {
    read(key: string): Promise<unknown | null>;
    write(key: string, value: unknown): Promise<void>;
    remove(key: string): Promise<void>;
  };
  /**
   * The project session: the document every member shares. `join` answers
   * the host's copy (`doc` null only when the member brought the same
   * bytes, which a worker never does). `patch` sends this worker's edits
   * and answers the new sequence number.
   */
  session: {
    join(projectId: string): Promise<SessionJoin>;
    patch(projectId: string, patches: Patch[]): Promise<number>;
    leave(projectId: string): void;
    /** Remote patches for any project; the pool fans them out by project. */
    onPatch(listener: (patch: SessionPatch) => void): () => void;
  };
  /** Agent media and captions find a project's media through the host's store. */
  mediaPath?: { lookup(request: MediaPathLookupRequest): Promise<MediaPathResult | null> };
  /** On-device transcription (captions). Absent: `generate_captions` says no model is available. */
  transcription?: {
    status(): Promise<TranscriptionStatus>;
    transcribe(request: TranscribeRequest): Promise<TranscribeResult>;
    cancelTranscribe(requestId: string): Promise<void>;
    /** Progress events for every request; the worker filters by `requestId`. */
    onProgress(listener: (progress: TranscribeProgress) => void): () => void;
  };
  /**
   * The user's extensions, as the editor's Mods panel lists them: user and
   * project extensions, overrides and forks of built-ins (`replaces`),
   * enabled state, health, and Store extensions (run in the contained
   * sandbox, `sandbox.ts`). Absent: built-ins only. A worker loads them
   * through the kernel's own loader (load order, `replaces`, `dependsOn`,
   * blocked states), as the editor does.
   */
  extensions?: {
    /** `EXT_IPC.list`: every record, with `bundleUrl` as the host serves it. */
    list(): Promise<ExtensionRecord[]>;
    /** The ES module source a record's `bundleUrl` names. */
    bundle(url: string): Promise<string>;
    /** `EXT_IPC.changed`: records changed (installed, enabled, rebuilt). */
    onChanged(listener: (event: ExtensionsChangedEvent) => void): () => void;
    /** Values for an extension's declared vars; absent or rejecting: none. */
    values?(id: string): Promise<Record<string, string>>;
  };
  log?(line: string): void;
}

/* ── the pool's API (what the embedder calls) ─────────────────────────── */

export interface OpenOptions {
  /** The agent session pinning this project (contract 15 `OpenProject.pinnedBy`). */
  pinnedBy?: string | null;
}

/** `ProjectOpened` plus the persisted state the instance resumed. */
export interface InstanceOpened extends ProjectOpened {
  projectId: string;
  /** Undo entries restored from the profile store (0 for a fresh instance). */
  restoredHistory: number;
  /**
   * The project needs a newer reader (`minReaderVersion`): as on the
   * desktop, it opens read-only, so edits stay in the instance and never
   * reach the session or the profile store.
   */
  readOnly: boolean;
  /** Which engine answers the instance's document queries. */
  engine: { evaluator: 'pm-wasm'; mode: string; fonts: 'registry' | 'none' };
}

export interface InstancePersisted { instance: number; projectId: string; revision: number; bytes: number; reason: PersistReason }

/** Per-instance counters the auditor run reads (zero fallbacks is the gate). */
export interface InstanceStats {
  instance: number;
  projectId: string;
  revision: number;
  /** The instance document's canonical hash (`canonicalProjectHash`), to compare with the host's copy. */
  hash?: string;
  historyDepth: number;
  /** `<op>:<reason>` → count, from the native API and the edit kernels. */
  fallbacks: Record<string, number>;
  /** Native queries and kernels answered. */
  calls: number;
  auditor: { audits: number; failures: number } | null;
  /** Dual answers (development modes): native vs legacy comparisons. */
  dual: { compared: number; mismatched: number; stale: number; comparedOps?: Record<string, number>; mismatchedOps: Record<string, number> } | null;
  /** Named failures the instance's engine reported (Use New Engine on). */
  failures: Array<{ code: string; message: string; retry: boolean }>;
  consistency: { checks: number; mismatches: number };
  extensions: { active: string[]; failed: Array<{ id: string; error: string }> };
}

export type ExtensionRegistration = Omit<ExtensionRegistered, 'instance'> & { instance: number; active: boolean };

export interface DocumentHostPoolApi {
  readonly protocol: typeof DOCUMENT_HOST_PROTOCOL;
  /** Opens (or returns) the project's instance. */
  open(projectId: string, options?: OpenOptions): Promise<InstanceOpened>;
  /** Routes a document-host tool call to the project's instance, opening it first. */
  call(projectId: string, request: AgentToolRequestEvent): Promise<AgentToolResponseEvent>;
  persist(projectId: string, reason: PersistReason): Promise<InstancePersisted>;
  /** Registers or hot-reloads an extension in the project's instance; resolves after it activated. */
  registerExtension(projectId: string, record: ExtensionRecord): Promise<ExtensionRegistration>;
  stats(projectId: string): Promise<InstanceStats>;
  /** Persists and stops the instance. */
  close(projectId: string): Promise<void>;
  /** Persists and stops every instance. */
  shutdown(): Promise<void>;
  /** Open instances by project id. */
  instances(): Array<{ projectId: string; instance: number; pid: number | null }>;
}

/* ── the pool ↔ worker wire (child-process IPC, structured clone) ─────── */

export type ToWorker =
  | { t: 'open'; id: number; projectId: string; instance: number; pinnedBy: string | null }
  | { t: 'tool'; id: number; request: AgentToolRequestEvent }
  | { t: 'persist'; id: number; reason: PersistReason }
  | { t: 'register'; id: number; record: ExtensionRecord }
  | { t: 'stats'; id: number }
  | { t: 'close'; id: number }
  /** A remote session patch for this worker's project. */
  | { t: 'remote'; patch: SessionPatch }
  /** The answer to a `service` request. */
  | { t: 'serviceReply'; id: number; ok: boolean; value?: unknown; error?: string }
  /** A transcription progress event. */
  | { t: 'progress'; progress: TranscribeProgress }
  /** The host's extension records changed (`DocumentHostServices.extensions.onChanged`). */
  | { t: 'extensionsChanged'; event: ExtensionsChangedEvent };

export type FromWorker =
  | { t: 'ready'; protocol: number; pid: number }
  | { t: 'reply'; id: number; ok: true; value: unknown }
  | { t: 'reply'; id: number; ok: false; failure: DocumentHostFailure }
  /** A host-service call from the worker. */
  | { t: 'service'; id: number; name: ServiceName; args: unknown[] }
  | { t: 'log'; line: string };

export type ServiceName =
  | 'store.read' | 'store.write' | 'store.remove'
  | 'session.join' | 'session.patch' | 'session.leave'
  | 'mediaPath.lookup'
  | 'transcription.status' | 'transcription.transcribe' | 'transcription.cancelTranscribe'
  | 'extensions.list' | 'extensions.bundle' | 'extensions.values';

/** Tool routes the hidden host keeps outside workers (mirrors pm_protocol::document_host). */
export const DOM_TOOLS = ['capture_panel', 'computer_use_panel', 'interact_panel', 'get_panel_state', 'open_panel', 'get_panel_layout', 'set_panel_layout'] as const;
export const JOB_TOOLS = ['export_video', 'get_render_status'] as const;

/** Explicit entries in the default document route (schema and arguments are
 * the harness's own). New document tools still use that default route. */
export const DOCUMENT_TOOLS = ['manage_media', '__composition_info', 'get_workspace_state'] as const;
