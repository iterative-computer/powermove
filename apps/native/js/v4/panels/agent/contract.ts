import type { Json, EventDetail } from "../../src/protocol";

/** Wire facts emitted by pm-panel-agentpanel/src/v4.rs, not a JS document model. */
export interface Row {
  id: string;
  commandId: string;
  value: Record<string, Json>;
  /** Choices made on an open question, by question id. */
  chosen: Record<string, string[]>;
  open: boolean;
  /** An archived step's run: its trace message, how long it worked and
   * whether it was steering (the view folds a finished turn's work). */
  trace?: { id: string; durationMs: number | null; steering: boolean };
}
export interface ThreadSummary {
  id: string;
  title: string;
  busy: boolean;
  updatedAt: number;
}
export type Access = "editor" | "supervised" | "auto" | "full";
export interface Snapshot {
  projectId: string;
  threadId: string;
  threadTitle?: string;
  runId: string | null;
  phase: string;
  sendable: boolean;
  composerDisabled: boolean;
  stopping: boolean;
  backgroundRuns: number;
  saveError: string | null;
  stopError: string | null;
  elapsedMs: number | null;
  /** What the run is doing before its first step lands. */
  activity?: string;
  /** The thread picker's rows. */
  threads?: ThreadSummary[];
  showJump: boolean;
  draft: { text: string; selection: [number, number]; armed: string | null };
  attachments: {
    id: string;
    name: string;
    type: Json;
    payloadAvailable: boolean;
  }[];
  settings: {
    provider: string;
    model: string;
    effort: string;
    access: Access;
    scope: string;
  };
  catalog: {
    models: [string, string][];
    providers: [string, string][];
    efforts: string[];
  };
  slash: {
    visible: boolean;
    index: number;
    choices: { id: string; value: string | null }[];
  };
  rows: { count: number; range: [number, number]; items: Row[] };
}
/** These facts belong to the shell/provider/native input coordinator, not Panel. */
export interface NativeFacts {
  connection: "checking" | "waiting" | "connected" | "unavailable";
  /** Unavailable because nobody is signed in: offer to connect. */
  signedOut?: boolean;
  /** What to tell the user when `connection` isn't "connected". */
  reason: string;
  presentation: "popover" | "docked";
  atMinimumDockWidth: boolean;
  visible: boolean;
  extensionContext: boolean;
  loadingHistory: boolean;
  dropTarget: boolean;
  needsAnswer: boolean;
  keyboardFocus: "composer" | "conversation" | null;
  textSelected: boolean;
  hoverRow: string | null;
  increaseContrast: boolean;
  reduceTransparency: boolean;
  reduceMotion: boolean;
  /** Display copy comes from the model/provider catalog, never from command ids. */
  slashCopy: Record<string, { label: string; description: string }>;
  /** Host-issued binding handles, used only after native registration ack. */
  bindings: { access: string; model: string };
}
export type Action =
  | {
      type: "draft";
      text: string;
      selection: [number, number];
      /** The draft the person saw when they typed (from the publication). */
      base?: string;
    }
  | { type: "focus"; focused: boolean }
  | { type: "key"; key: EventDetail }
  | {
      type:
        | "attach"
        | "submit"
        | "stop"
        | "newThread"
        | "openSettings"
        | "keepResult"
        | "undoResult"
        | "connect";
    }
  /** Try Again: the request before this failed turn (or the last one). */
  | { type: "retry"; id?: string }
  | {
      type: "shell";
      action: "open" | "close" | "pin" | "beginDrag" | "cancelDrag";
    }
  | { type: "shell"; action: "commitDrag"; dock: string | null }
  | {
      type:
        | "switchThread"
        | "removeAttachment"
        | "openAttachment"
        | "openVisual"
        | "chooseSlash"
        | "stopTask";
      id: string;
    }
  | { type: "disclose"; id: string; open: boolean }
  | { type: "setAccess"; access: Access }
  | { type: "setProvider"; provider: string }
  | { type: "setModel"; model: string; effort: string }
  | { type: "retryConnection" }
  | { type: "chooseAnswer"; step: string; question: string; option: string }
  | { type: "typeAnswer"; step: string; question: string; text: string }
  | { type: "answer"; step: string; skip: boolean }
  | { type: "artifact"; path: string; import: boolean }
  | { type: "openMedia"; path: string; reveal: boolean };
export interface Publication {
  generation: number;
  sequence: number;
  snapshot: Snapshot;
  native: NativeFacts;
}
export interface AgentModel {
  inspect(): Publication;
  subscribe(callback: () => void): () => void;
  dispatch(action: Action): Promise<void>;
}
