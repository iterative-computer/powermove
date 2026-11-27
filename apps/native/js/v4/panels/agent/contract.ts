import type { Citation } from "./citations";
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
  parentId?: string | null;
  kind?: string | null;
  id: string;
  title: string;
  busy: boolean;
  updatedAt: number;
}
export type Access = "editor" | "supervised" | "auto" | "full";
export interface ExtensionResult {
  extensionId: string;
  panelId: string | null;
  name: string;
  action: "created" | "updated" | "removed";
  status: "ready" | "error" | "removed";
}
export interface NativeResult {
  runId: string;
  summary: string;
  changed: boolean;
  revision?: number;
  errors?: Json[];
  extensionChangeSets?: string[];
  verificationError?: string;
  extensions?: ExtensionResult[];
}
export interface Snapshot {
  plan?: Json;
  result?: NativeResult | null;
  resultActions?: { apply: boolean; keep: boolean; undo: boolean };
  projectId: string;
  threadId: string;
  threadTitle?: string;
  /** Every thread of this project, the latest first (the thread picker). */
  threads?: ThreadSummary[];
  runId: string | null;
  starting?: boolean;
  nextMessageId?: string;
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
  showJump: boolean;
  draft: {
    citations?: Citation[];
    text: string;
    selection: [number, number];
    armed: string | null;
    /** Advances when the host consumes this thread’s composer draft. */
    sent?: number;
  };
  attachments: {
    id: string;
    name: string;
    type: Json;
    payloadAvailable: boolean;
    thumbnail?: string | null;
    duration?: string | null;
    screenPick?: boolean | null;
    caption?: string | null;
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
    /** The effort the current model runs at when none is chosen. */
    defaultEffort?: string;
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
  /** The provider is currently discovering its models. */
  findingModels?: boolean;
  /** The discovered choices for each provider, including its default. */
  modelGroups?: Record<string, [id: string, label: string, isDefault: boolean][]>;
  /** The account each agent harness runs on, as the chooser heads its
   * models ("ChatGPT", "OpenAI API key", …). */
  accounts?: Record<string, string>;
  /** Aero on the Powermove account: its usage monitor's lines (credits,
   * plan, reset day; none until the backend answers), whether the account
   * is on Free, and when its credits are spent, the reset day and the
   * bigger plan. */
  aero?: {
    usage: [label: string, value: string][];
    /** Signed in on Free: Aero needs Aerolite (quiet, never a fault). */
    needsPlan?: boolean;
    /** Running low on credits this period (80%+ warning, 95%+ critical). */
    lowCredits?: {
      remaining: number;
      included: number;
      percentUsed: number;
      level: "warning" | "critical";
      resets: string | null;
    } | null;
    /** No credits left this period. */
    outOfCredits?: { resets: string | null; upgrade: string | null } | null;
  };
  /** The configured compatible model's display name for its menu heading. */
  connectedModelName?: string | null;
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
  | { type: "citations"; citations: Citation[] }
  | { type: "openResultPanel"; extensionId: string; panelId: string }
  | { type: "attachInput"; paths: string[] }
  | {
      type: "draft";
      text: string;
      selection: [number, number];
      /** The draft the person saw when they typed (from the publication). */
      base?: string;
      composerEpoch?: number;
    }
  | { type: "focus"; focused: boolean }
  | { type: "key"; key: EventDetail; composerEpoch?: number }
  | { type: "submit"; text?: string; composerEpoch?: number }
  | {
      type:
        | "attach"
        | "pickRegion"
        | "stop"
        | "newThread"
        | "openSettings"
        | "subscribe"
        | "applyPlan"
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
        | "deleteThread"
        | "removeAttachment"
        | "openAttachment"
        | "openVisual"
        | "chooseSlash"
        | "stopTask";
      id: string;
    }
  | { type: "disclose"; id: string; open: boolean }
  | { type: "setAccess"; access: Access }
  | { type: "setProvider"; provider: string; model?: string }
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
