/**
 * What the Agent panel shows, worked out from the model's rows. Pure: no
 * view, no host. Ported from the original app's panel so the two read the
 * same (activity-rows.ts, work-groups.ts, errors/presentation.ts,
 * slash-commands.ts and the conversation's copy).
 */
import type { Json } from "../../src/protocol";
import type { Row, Snapshot } from "./contract";

type Obj = Record<string, Json>;
const obj = (v: unknown): Obj =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Obj) : {};
export const str = (v: unknown) => (typeof v === "string" ? v : "");
const num = (v: unknown) =>
  typeof v === "number" && Number.isFinite(v) ? v : undefined;

// ── Durations ────────────────────────────────────────────────────────────

/** "4s", "1m 12s"; sub-second work reads "<1s", never "0s". */
export function durationLabel(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "";
  const seconds = Math.round(ms / 1000);
  if (seconds < 1) return "<1s";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
/** "Working for 12s", "Worked for 2m 30s". */
export function workDuration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return seconds < 60
    ? `${seconds}s`
    : seconds < 3600
      ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
      : `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m ${seconds % 60}s`;
}
/** A thread's last activity, as the thread picker says it. */
export function relativeOpened(at: number, now: number): string {
  if (!at) return "";
  const minutes = Math.max(0, Math.round((now - at) / 60000));
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(at).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

// ── Tools ────────────────────────────────────────────────────────────────

export type ToolFamily =
  | "thread"
  | "run"
  | "edit"
  | "read"
  | "search"
  | "image"
  | "computer"
  | "panel"
  | "think"
  | "watch"
  | "listen"
  | "tool";
const FAMILY_RULES: [RegExp, ToolFamily][] = [
  [/^(probe_media|sample_media_frames|media_contact_sheet)$/, "watch"],
  [/^(media_waveform|transcribe_media|generate_captions)$/, "listen"],
  [/^export_captions$/, "panel"],
  [/^check_project$/, "panel"],
  [
    /^(delegate_task|orchestrator_capabilities|task_status|task_cancel|task_send)$/,
    "think",
  ],
  [/^thread_(launch|list|read|send|watch|unwatch|cancel)$/, "thread"],
  [
    /panel|workspace|project_state|render_frames|apply_commands|edit_video|rollback/,
    "panel",
  ],
  [/^(web|search|browse|fetch|grep|glob|find|ls$|list)|url/, "search"],
  [/^(bash|command|shell|terminal|exec|run|process)/, "run"],
  [
    /^(edit|write|create|patch|apply|delete|move|multiedit|notebook)|file_change/,
    "edit",
  ],
  [/^(read|view|cat)/, "read"],
  [/^(compact|wait$)/, "think"],
  [/^(image|render|draw|screenshot)/, "image"],
  [/^computer/, "computer"],
  [/^(agent|task|todo|plan)/, "think"],
];
const short = (toolName: unknown) => {
  const name = str(toolName).trim().toLowerCase();
  return name.startsWith("mcp__") ? name.split("__").at(-1) || name : name;
};
export function toolFamily(toolName?: unknown): ToolFamily {
  const name = short(toolName);
  if (!name) return "tool";
  for (const [pattern, family] of FAMILY_RULES)
    if (pattern.test(name)) return family;
  return "tool";
}
/** The SF Symbol for a tool family (the original's glyph set, natively). */
export const FAMILY_SYMBOL: Record<ToolFamily, string> = {
  thread: "bubble.left.and.bubble.right",
  run: "terminal",
  edit: "pencil",
  read: "doc.text",
  search: "magnifyingglass",
  image: "photo",
  computer: "display",
  panel: "sidebar.left",
  think: "sparkles",
  watch: "film",
  listen: "waveform",
  tool: "wrench.and.screwdriver",
};
const ACTION_RULES: [RegExp, string][] = [
  [/^delegate_task$/, "asked subagents"],
  [/^task_status$/, "checked on subagents"],
  [/^task_cancel$/, "stopped a subagent"],
  [/^task_send$/, "redirected a subagent"],
  [/^thread_launch$/, "started a thread"],
  [/^thread_send$/, "messaged a thread"],
  [/^thread_(watch|unwatch)$/, "followed a thread"],
  [/^thread_(read|list)$/, "checked threads"],
  [/^thread_cancel$/, "stopped a thread"],
  [/^orchestrator_capabilities$/, "checked available models"],
  [/^compact/, "compacted the conversation"],
  [/^view_image$/, "viewed images"],
  [/^wait$/, "waited"],
  [/^(agent|task)$/, "worked with subagents"],
  [/^get_project_state$/, "inspected the project"],
  [/^list_media$/, "inspected project media"],
  [/^replace_media$/, "replaced media"],
  [/^import_media$/, "imported media"],
  [/^manage_media$/, "managed project media"],
  [/^get_panel_layout$/, "checked the panel layout"],
  [/^open_panel$/, "opened a panel"],
  [/^get_panel_state$/, "checked panel controls"],
  [/^interact_panel$/, "used panel controls"],
  [/^capture_panel$/, "captured a panel"],
  [/^computer_use_panel$/, "tested panel controls"],
  [/^get_workspace_state$/, "inspected the workspace"],
  [/^render_frames$/, "previewed composition frames"],
  [/^probe_media$/, "probed media"],
  [/^sample_media_frames$/, "sampled footage"],
  [/^media_contact_sheet$/, "built contact sheets"],
  [/^media_waveform$/, "mapped silences"],
  [/^transcribe_media$/, "transcribed speech"],
  [/^check_project$/, "checked the project"],
  [/^generate_captions$/, "captioned speech"],
  [/^export_captions$/, "exported captions"],
  [/^apply_commands$/, "edited the composition"],
  [/^edit_video$/, "edited video clips"],
  [/^rollback_changes$/, "undid agent changes"],
  [/^(web|search|browse|fetch)|url/, "searched the web"],
  [/^(bash|command|shell|terminal|exec|run|process)/, "ran commands"],
  [/^(edit|write|create|patch|file|apply|delete|move)/, "edited files"],
  [/^(read|grep|glob|ls|list|find|view)/, "read files"],
  [/^(image|render|draw|screenshot)/, "generated images"],
  [/^computer/, "operated the computer"],
];
export function toolAction(toolName?: unknown): string {
  const name = short(toolName);
  if (!name) return "used a tool";
  for (const [pattern, action] of ACTION_RULES)
    if (pattern.test(name)) return action;
  return `used ${name.replace(/[_-]+/g, " ")}`;
}
export function joinActions(actions: string[]): string {
  if (actions.length === 0) return "used a tool";
  if (actions.length === 1) return actions[0]!;
  if (actions.length === 2) return `${actions[0]} and ${actions[1]}`;
  return `${actions.slice(0, -1).join(", ")}, and ${actions.at(-1)}`;
}
const THREAD_LABELS: Record<string, string> = {
  thread_launch: "Started a thread",
  thread_send: "Messaged a thread",
  thread_watch: "Following a thread",
  thread_unwatch: "Stopped following a thread",
  thread_read: "Read a thread",
  thread_list: "Checked threads",
  thread_cancel: "Stopped a thread",
};
/** The chip for a thought is its opening: first sentence, one line. */
export function thoughtLead(text: string): string {
  const flat = String(text ?? "")
    .replace(/\*\*/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const sentence = flat.match(/^.{12,}?[.!?](\s|$)/)?.[0]?.trim();
  const lead = sentence && sentence.length < flat.length ? sentence : flat;
  return lead.length > 96 ? `${lead.slice(0, 95).trimEnd()}…` : lead;
}
const basename = (path: string) =>
  path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;

/** A tool's own words once it settles (a transcription still pending…). */
function settledLabel(step: Obj): string {
  const label = str(step.label) || str(step.title);
  if (step.outcome === "pending") return "Started a transcription";
  if (step.outcome === "needs-model") return "Needs a transcription model";
  return (
    label || toolAction(step.toolName).replace(/^./, (c) => c.toUpperCase())
  );
}

// ── Activity rows (one turn's trail) ─────────────────────────────────────

export type Status = "running" | "partial" | "error" | "done";
export interface ToolDetail {
  /** The row the model knows (for Stop on a subagent). */
  rowId: string;
  kind: "tool" | "thought";
  id: string;
  label: string;
  status: string;
  detail: string;
  output: string;
  family: ToolFamily;
  span: string;
  task?: Obj;
  progress?: string;
}
export interface ToolsRow {
  kind: "tools";
  key: string;
  label: string;
  summary: string;
  status: Status;
  failedCount: number;
  toolCount: number;
  meta: string;
  details: ToolDetail[];
  files: string[];
  tooltip?: string;
}
export type Activity =
  | { kind: "text"; key: string; text: string; row: Row }
  | { kind: "thought"; key: string; text: string; live: boolean }
  | { kind: "question"; key: string; row: Row }
  | ToolsRow;

interface Step {
  row: Row;
  v: Obj;
}
function groupedToolRow(work: Step[]): ToolsRow {
  const steps = work.filter((s) => s.v.kind === "tool");
  const thoughts = work.filter((s) => s.v.kind === "thought");
  const actions = [
    ...new Set(
      steps.map((s) =>
        s.v.outcome === "pending"
          ? "started a transcription"
          : s.v.outcome === "needs-model"
            ? "asked for a transcription model"
            : toolAction(s.v.toolName),
      ),
    ),
  ];
  const failedCount = steps.filter((s) => s.v.status === "error").length;
  const settled = steps.filter((s) => s.v.status !== "running").length;
  const thinking = thoughts.some((s) => s.v.live === true);
  const status: Status =
    steps.some((s) => s.v.status === "running") || thinking
      ? "running"
      : failedCount === settled && failedCount > 0
        ? "error"
        : failedCount > 0
          ? "partial"
          : "done";
  const started = work
    .map((s) => num(s.v.startedAt))
    .filter((n): n is number => n !== undefined);
  const ended = work
    .map((s) => num(s.v.endedAt))
    .filter((n): n is number => n !== undefined);
  const span =
    started.length && ended.length && status !== "running"
      ? durationLabel(Math.max(...ended) - Math.min(...started))
      : "";
  const taskCount = steps.filter((s) => s.v.task).length;
  const callCount = steps.length - taskCount;
  const label = steps.length
    ? [
        taskCount ? `${taskCount} ${taskCount === 1 ? "task" : "tasks"}` : "",
        callCount
          ? `${callCount} tool ${callCount === 1 ? "call" : "calls"}`
          : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : thinking
      ? "Thinking"
      : span
        ? `Thought for ${span}`
        : "Thought";
  const summary = steps.length
    ? joinActions(actions).replace(/^./, (c) => c.toUpperCase())
    : label;
  const meta =
    status === "running"
      ? ""
      : [status === "partial" ? `${failedCount} failed` : "", span]
          .filter(Boolean)
          .join(" · ");
  const details: ToolDetail[] = work
    .map((s): ToolDetail => {
      const v = s.v;
      const s0 = num(v.startedAt),
        s1 = num(v.endedAt);
      const each =
        s0 !== undefined && s1 !== undefined ? durationLabel(s1 - s0) : "";
      if (v.kind === "thought") {
        const live = v.live === true;
        return {
          rowId: s.row.commandId,
          kind: "thought",
          id: str(v.id),
          label: live ? "Thinking" : each ? `Thought for ${each}` : "Thought",
          status: live ? "running" : "done",
          detail: thoughtLead(str(v.label)),
          output: str(v.label),
          family: "think",
          span: "",
        };
      }
      const name = short(v.toolName);
      const thread = obj(v.thread);
      const task = v.task ? obj(v.task) : undefined;
      return {
        rowId: s.row.commandId,
        kind: "tool",
        id: str(v.id),
        label: THREAD_LABELS[name] ?? settledLabel(v),
        status: str(v.status),
        // What it worked on: the media or layer by its name in the project
        // (the host names it), else the provider's argument, never only the
        // tool's own id.
        detail:
          str(thread.title) ||
          str(v.subjectName) ||
          (str(v.detail) && str(v.detail) !== str(v.toolName)
            ? str(v.detail)
            : ""),
        // Opened: the whole argument, then the output or the error.
        output: [
          str(v.detail) && str(v.detail) !== str(v.toolName)
            ? str(v.detail)
            : "",
          typeof v.output === "string" ? v.output : "",
        ]
          .filter(Boolean)
          .join("\n\n"),
        family: toolFamily(v.toolName),
        span: each,
        ...(task ? { task, progress: str(task.progress) } : {}),
      };
    })
    .filter((d) => !!d.label);
  const files = [
    ...new Set(
      steps
        .filter(
          (s) =>
            toolFamily(s.v.toolName) === "edit" &&
            str(s.v.detail) &&
            s.v.status !== "error",
        )
        .flatMap((s) =>
          str(s.v.detail)
            .split(/,\s*/)
            .map((p) => basename(p.trim()))
            .filter(Boolean),
        ),
    ),
  ];
  const tooltip =
    status !== "running" && steps.length > 1
      ? [
          summary,
          ...steps
            .filter((s) => settledLabel(s.v))
            .map((s) =>
              failedCount > 0
                ? `${s.v.status === "error" ? "Failed" : s.v.status === "continued" ? "Continued" : "Succeeded"} · ${settledLabel(s.v)}`
                : settledLabel(s.v),
            ),
        ].join("\n")
      : undefined;
  return {
    kind: "tools",
    key: `tools-${work[0]?.row.id ?? "activity"}`,
    label,
    summary,
    status,
    failedCount,
    toolCount: steps.length,
    meta,
    details,
    files,
    ...(tooltip ? { tooltip } : {}),
  };
}
/**
 * One turn's steps as the original reads them: calls share one group;
 * prose, reasoning and questions break it, so the trail stays in order.
 */
export function activity(steps: Row[]): Activity[] {
  const out: Activity[] = [];
  let work: Step[] = [];
  const flush = () => {
    if (work.length) out.push(groupedToolRow(work));
    work = [];
  };
  for (const row of steps) {
    const v = row.value;
    const kind = str(v.kind);
    if (kind === "text") {
      const text = str(v.text);
      if (!text.trim()) continue;
      flush();
      out.push({ kind: "text", key: row.id, text, row });
    } else if (kind === "thought") {
      if (!str(v.label).trim()) continue;
      flush();
      out.push({
        kind: "thought",
        key: row.id,
        text: str(v.label),
        live: v.live === true,
      });
    } else if (kind === "question") {
      flush();
      out.push({ kind: "question", key: row.id, row });
    } else if (kind === "tool") {
      work.push({ row, v });
    } else flush();
  }
  flush();
  return out;
}

// ── The conversation ─────────────────────────────────────────────────────

export interface Message {
  key: string;
  /** The model's id for the row (Try Again names the failed turn). */
  rowId: string;
  role: "user" | "assistant" | "trace";
  value: Obj;
  steps: Row[];
  durationMs?: number;
  steering: boolean;
  /** The running turn's trace (rows still streaming). */
  live: boolean;
}
export type Entry =
  | { kind: "message"; key: string; message: Message }
  | { kind: "work"; key: string; label: string; items: Message[] }
  | { kind: "working"; key: string; label: string };

/** Archived steps carry their trace message; group them back into it. */
export function messages(s: Snapshot): Message[] {
  const out: Message[] = [];
  let live: Message | undefined;
  for (const row of s.rows.items) {
    const v = row.value as Obj;
    if (row.id.startsWith("live:")) {
      if (!live) {
        live = {
          key: `live-${row.id}`,
          rowId: "",
          role: "trace",
          value: {},
          steps: [],
          steering: false,
          live: true,
        };
        out.push(live);
      }
      live.steps.push(row);
      continue;
    }
    const trace = obj((row as Row & { trace?: Json }).trace);
    if (str(trace.id)) {
      const last = out.at(-1);
      if (
        last &&
        last.role === "trace" &&
        last.rowId === `history:${str(trace.id)}`
      )
        last.steps.push(row);
      else {
        const duration = num(trace.durationMs);
        out.push({
          key: row.id,
          rowId: `history:${str(trace.id)}`,
          role: "trace",
          value: {},
          steps: [row],
          ...(duration !== undefined ? { durationMs: duration } : {}),
          steering: trace.steering === true,
          live: false,
        });
      }
      continue;
    }
    // A bare step (an older publication or a window that begins mid-trace).
    if (v.kind && !v.role) {
      const last = out.at(-1);
      if (last && last.role === "trace" && !last.live) last.steps.push(row);
      else
        out.push({
          key: row.id,
          rowId: "",
          role: "trace",
          value: {},
          steps: [row],
          steering: false,
          live: false,
        });
      continue;
    }
    const role = v.role === "user" ? "user" : "assistant";
    out.push({
      key: row.id,
      rowId: row.id,
      role,
      value: v,
      steps: [],
      steering: v.steering === true,
      live: false,
    });
  }
  return out;
}
const isPrompt = (m: Message) => m.role === "user" && !m.steering;
function trailing(steps: Row[]) {
  let cut = steps.length;
  while (cut > 0 && steps[cut - 1]!.value.kind === "text") cut--;
  return { work: steps.slice(0, cut), reply: steps.slice(cut) };
}
function fold(
  body: Message[],
): { work: Message[]; reply: Message[]; durationMs?: number } | null {
  const last = body.at(-1);
  if (!last) return null;
  const work: Message[] = [];
  const reply: Message[] = [];
  let end = body.length;
  if (last.role === "assistant") {
    reply.push(last);
    end -= 1;
    const before = body[end - 1];
    if (before?.role === "trace" && !before.steering) {
      const split = trailing(before.steps);
      if (split.reply.length) {
        end -= 1;
        if (split.work.length)
          work.push({
            ...before,
            key: `${before.key}:work`,
            steps: split.work,
          });
        reply.unshift({
          ...before,
          key: `${before.key}:reply`,
          steps: split.reply,
        });
      }
    }
  } else if (last.role === "trace") {
    const split = trailing(last.steps);
    if (!split.reply.length) return null;
    end -= 1;
    if (split.work.length)
      work.push({ ...last, key: `${last.key}:work`, steps: split.work });
    reply.push({ ...last, key: `${last.key}:reply`, steps: split.reply });
  } else return null;
  work.unshift(...body.slice(0, end));
  // Visual replies are deliverables: they stay in view.
  const artifacts = work.filter(
    (m) =>
      Array.isArray(m.value.visualReplies) && m.value.visualReplies.length > 0,
  );
  for (let i = work.length - 1; i >= 0; i--)
    if (artifacts.includes(work[i]!)) work.splice(i, 1);
  reply.unshift(...artifacts);
  if (!work.length) return null;
  const durations = body
    .filter((m) => m.role === "trace" && m.durationMs !== undefined)
    .map((m) => m.durationMs!);
  return {
    work,
    reply,
    ...(durations.length ? { durationMs: Math.max(...durations) } : {}),
  };
}
/**
 * A finished turn reads prompt → "Worked for 2m 30s" → answer; the work it
 * did folds behind that row. The running turn stays open with its timer.
 */
export function entries(s: Snapshot, list = messages(s)): Entry[] {
  const out: Entry[] = [];
  const running = s.runId !== null;
  const plain = (m: Message) =>
    out.push({ kind: "message", key: m.key, message: m });
  let i = 0;
  while (i < list.length && !isPrompt(list[i]!)) plain(list[i++]!);
  let lastPrompt = -1;
  while (i < list.length) {
    const start = i;
    let end = start + 1;
    while (end < list.length && !isPrompt(list[end]!)) end++;
    i = end;
    lastPrompt = out.length;
    plain(list[start]!);
    const body = list.slice(start + 1, end);
    const live = running && end === list.length;
    const grouped = live ? null : fold(body);
    if (!grouped) {
      body.forEach(plain);
      continue;
    }
    out.push({
      kind: "work",
      key: `work-${list[start]!.key}`,
      label:
        grouped.durationMs === undefined
          ? "Work"
          : `Worked for ${workDuration(grouped.durationMs)}`,
      items: grouped.work,
    });
    grouped.reply.forEach(plain);
  }
  if (running) {
    const label = s.stopping
      ? "Stopping…"
      : `Working for ${workDuration(s.elapsedMs ?? 0)}`;
    // The timer sits under the request it answers, above the work.
    const at = lastPrompt >= 0 ? lastPrompt + 1 : out.length;
    out.splice(at, 0, { kind: "working", key: "working", label });
  }
  return out;
}

// ── Errors ───────────────────────────────────────────────────────────────

export interface ErrorPresentation {
  title: string;
  message: string;
  details: string;
}
/** A provider's JSON error (`{"type":"error","error":{"message":…}}`) in words. */
function unwrap(raw: string): string {
  const text = raw.trim();
  const start = text.indexOf("{");
  if (start < 0) return text;
  try {
    const value = obj(JSON.parse(text.slice(start)));
    const inner = obj(value.error);
    const message =
      str(inner.message) ||
      str(value.message) ||
      str(value.detail) ||
      str(inner.type) ||
      str(value.error);
    if (!message) return text;
    const prefix = text
      .slice(0, start)
      .trim()
      .replace(/[:\s]+$/, "");
    return prefix && !/^error$/i.test(prefix)
      ? `${prefix}: ${message}`
      : message;
  } catch {
    return text;
  }
}
/** Diagnostics stay available; nobody has to decipher a transport log. */
export function presentError(raw: string): ErrorPresentation {
  const details = raw.trim();
  const text = unwrap(details);
  const result = (title: string, message: string) => ({
    title,
    message,
    details,
  });
  const outdated = text.match(
    /(Claude Code|Codex(?: CLI)?)\s+v?(\d+(?:\.\d+)+)\s+does not support this model;?\s*version\s+v?(\d+(?:\.\d+)+)\s+or newer/i,
  );
  if (
    outdated ||
    /(?:requires?|needs?) a newer version of (Claude Code|Codex)/i.test(text)
  ) {
    const claude = /claude/i.test(outdated?.[1] ?? text);
    return result(
      `Update ${claude ? "Claude Code" : "Codex"} to use this model`,
      "This model needs a newer version than the one Powermove has. Install the update, then try again.",
    );
  }
  if (/active writer|saved agent session couldn[’']t reopen/i.test(text))
    return result(
      "Agent session couldn’t reopen",
      "Try sending your request again. If it repeats, restart Powermove to reconnect the agent.",
    );
  if (
    /already (?:active|running).*conversation|run .*already active/i.test(text)
  )
    return result(
      "Conversation is busy",
      "Wait for the current run to finish, or stop it before sending your request again.",
    );
  if (
    /no rollout found|(?:thread|session).{0,80}(?:not found|could not be resumed)/i.test(
      text,
    )
  )
    return result(
      "Couldn’t reopen the conversation",
      "Send your request again. If this continues, start a new conversation.",
    );
  if (/rate.?limit|\b429\b|overloaded|usage limit/i.test(text))
    return result(
      "Agent is temporarily unavailable",
      "Wait a moment, then send your request again. Check your account if you’ve reached a usage limit.",
    );
  if (
    /not signed in|not logged in|login required|unauthorized|\b401\b|authentication|sign.{0,8}again/i.test(
      text,
    )
  )
    return result(
      "Sign-in needs attention",
      "Reconnect your agent account in Settings, then send your request again.",
    );
  if (/too long|timed? out|timeout/i.test(text))
    return result(
      "The request timed out",
      "Check the current result before trying again; the agent may have completed part of the work.",
    );
  if (/ENOSPC|disk.{0,10}full|no space left/i.test(text))
    return result(
      "Your disk is full",
      "Free up some storage, then try the operation again.",
    );
  if (/EACCES|EPERM|permission denied/i.test(text))
    return result(
      "File access was denied",
      "Choose a location you can access, or check the app’s file permissions in System Settings.",
    );
  if (
    /network|ECONNRESET|ENOTFOUND|fetch failed|connection.{0,30}(?:interrupted|closed|refused)|stream disconnected/i.test(
      text,
    )
  )
    return result(
      "Connection interrupted",
      "Check your internet connection, then try again.",
    );
  if (
    /invalid (?:result|section)|returned.{0,30}invalid|response format.*rejected|without a result/i.test(
      text,
    )
  )
    return result(
      "The agent couldn’t finish the result",
      "Review any changes already made, then send your request again.",
    );
  const readable = text.replace(/^(?:The agent failed:\s*|Error:\s*)+/i, "");
  const technical =
    /(?:\b(?:TypeError|ReferenceError|SyntaxError)\b|\bat \S+ \(|thread\/|code -\d+|\b[A-Z_]{4,}:|^[[{])/.test(
      readable,
    );
  return result(
    "Something needs attention",
    !technical && readable && readable.length <= 400
      ? readable
      : "This operation couldn’t finish. Try again; if it continues, open Details to see what happened.",
  );
}

// ── Markdown, as native text ─────────────────────────────────────────────

export interface Run {
  t: string;
  b?: true;
  i?: true;
  c?: true;
  a?: true;
}
export type Block =
  | { kind: "p"; runs: Run[] }
  | { kind: "h"; level: number; runs: Run[] }
  | { kind: "li"; marker: string; depth: number; runs: Run[] }
  | { kind: "quote"; runs: Run[] }
  | { kind: "code"; text: string }
  | { kind: "rule" };

/** Inline emphasis, code and links as styled runs; no markup is executed. */
export function inline(text: string): Run[] {
  const runs: Run[] = [];
  const push = (run: Run) => {
    if (!run.t) return;
    const last = runs.at(-1);
    if (
      last &&
      !!last.b === !!run.b &&
      !!last.i === !!run.i &&
      !!last.c === !!run.c &&
      !!last.a === !!run.a
    )
      last.t += run.t;
    else runs.push(run);
  };
  const pattern =
    /(`+)([^`]+?)\1|\*\*([^*]+?)\*\*|__([^_]+?)__|(?<![\w*])\*([^*\s][^*]*?)\*(?!\w)|(?<![\w_])_([^_\s][^_]*?)_(?![\w])|\[([^\]]+)\]\(([^)\s]+)\)|<(https?:\/\/[^>\s]+)>/g;
  let at = 0;
  for (const m of text.matchAll(pattern)) {
    push({ t: text.slice(at, m.index) });
    if (m[2] !== undefined) push({ t: m[2], c: true });
    else if (m[3] !== undefined || m[4] !== undefined)
      for (const r of inline(m[3] ?? m[4]!)) push({ ...r, b: true });
    else if (m[5] !== undefined || m[6] !== undefined)
      for (const r of inline(m[5] ?? m[6]!)) push({ ...r, i: true });
    else if (m[7] !== undefined) push({ t: m[7], a: true });
    else if (m[9] !== undefined) push({ t: m[9], a: true });
    at = m.index! + m[0].length;
  }
  push({ t: text.slice(at) });
  return runs;
}
export function markdown(source: string): Block[] {
  const blocks: Block[] = [];
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  let para: string[] = [];
  const flush = () => {
    if (para.length) blocks.push({ kind: "p", runs: inline(para.join(" ")) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const fence = line.match(/^\s*(```|~~~)/);
    if (fence) {
      flush();
      const body: string[] = [];
      while (++i < lines.length && !lines[i]!.trimStart().startsWith(fence[1]!))
        body.push(lines[i]!);
      blocks.push({ kind: "code", text: body.join("\n") });
      continue;
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    const heading = line.match(/^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/);
    if (heading) {
      flush();
      blocks.push({
        kind: "h",
        level: heading[1]!.length,
        runs: inline(heading[2]!),
      });
      continue;
    }
    if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush();
      blocks.push({ kind: "rule" });
      continue;
    }
    const item = line.match(/^(\s*)([-*+]|\d{1,3}[.)])\s+(.*)$/);
    if (item) {
      flush();
      const depth = Math.min(
        3,
        Math.floor(item[1]!.replace(/\t/g, "  ").length / 2),
      );
      const marker = /\d/.test(item[2]!) ? item[2]!.replace(")", ".") : "•";
      blocks.push({ kind: "li", marker, depth, runs: inline(item[3]!) });
      continue;
    }
    const quote = line.match(/^\s*>\s?(.*)$/);
    if (quote) {
      flush();
      blocks.push({ kind: "quote", runs: inline(quote[1]!) });
      continue;
    }
    // A list item's wrapped continuation joins it.
    const last = blocks.at(-1);
    if (!para.length && last?.kind === "li" && /^\s{2,}\S/.test(line)) {
      last.runs.push(...inline(` ${line.trim()}`));
      continue;
    }
    para.push(line.trim());
  }
  flush();
  return blocks;
}
/** Long unbroken tokens (paths, links, JSON) get zero-width break points
 * after their punctuation, so native text wraps them instead of clipping. */
export function breakable(text: string): string {
  return text.replace(/\S{24,}/g, (word) =>
    word.replace(/([,:;/\\.\-_&?=}\])])/g, "$1\u200b"),
  );
}
/** One line of plain words (a chip, a heading): markers removed. */
export function plain(text: string): string {
  return inline(text)
    .map((r) => r.t)
    .join("")
    .replace(/\s+/g, " ")
    .trim();
}

// ── Copy shared with the original app ────────────────────────────────────

export const ACCESS = {
  editor: {
    label: "Composition only",
    detail: "Edit the composition. No shell, files or web.",
  },
  supervised: {
    label: "Supervised",
    detail: "Ask before anything outside the project sandbox.",
  },
  auto: {
    label: "Auto",
    detail: "Codex and Claude approve routine actions; API models still ask.",
  },
  full: {
    label: "Full access",
    detail: "Run commands and edit files anywhere without prompts.",
  },
} as const;
export const EFFORT: Record<string, string> = {
  none: "None",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};
export function effortLabel(id: string) {
  return EFFORT[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
}
export function suggestions(
  access: string,
  app: boolean,
): [label: string, prompt: string][] {
  return app
    ? [
        [
          "Create an extension",
          "Help me design and build a new Powermove extension",
        ],
        [
          "Improve an extension",
          "Help me improve one of my existing Powermove extensions",
        ],
        [
          "Check compatibility",
          "Help me check an extension for Store sandbox compatibility",
        ],
      ]
    : access === "editor"
      ? [
          [
            "Animate a title",
            "Animate the selected title with a confident entrance using editable keyframes",
          ],
          [
            "Shape the scene",
            "Refine the composition with editable shapes, thoughtful spacing, and a clear visual hierarchy",
          ],
          [
            "Arrange my workspace",
            "Organize my panels into a focused animation workspace",
          ],
        ]
      : [
          [
            "Find useful footage",
            "Research and download useful licensed footage for this composition, then import the strongest choices",
          ],
          [
            "Build an integration",
            "Build and test the project-local integration needed to complete this project",
          ],
          [
            "Finish the project",
            "Use any useful project tools and web research to finish this composition end to end",
          ],
        ];
}
/** The shortcuts menu's rows (the model lists which apply). */
export function slashCopy(
  id: string,
  value: string | null,
  s: Pick<Snapshot, "catalog" | "runId">,
): { label: string; description: string } {
  if (value !== null) {
    const command = id.slice(0, id.indexOf("-"));
    const named =
      command === "model"
        ? s.catalog.models.find(([v]) => v === value)?.[1]
        : command === "provider"
          ? s.catalog.providers.find(([v]) => v === value)?.[1]
          : command === "effort"
            ? value === "xhigh"
              ? "Extra high"
              : effortLabel(value)
            : undefined;
    return {
      label: named ?? value,
      description:
        command === "model" && !named
          ? "Use Claude model ID"
          : `Set ${command}`,
    };
  }
  const working = s.runId !== null;
  const copy: Record<string, string> = {
    new: working
      ? "Start a new conversation and leave this one running"
      : "Start a new conversation",
    model: "Choose a model",
    effort: "Set reasoning effort",
    provider: "Switch AI provider",
    attach: "Attach files or images",
    settings: "Open agent connections",
    stop: "Stop the current run",
  };
  return { label: `/${id}`, description: copy[id] ?? "" };
}
export function providerName(provider: string) {
  return provider === "compatible"
    ? "API or local model"
    : provider === "claude"
      ? "Claude"
      : "ChatGPT";
}
