import type { ComponentChildren } from "preact";
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import type {
  Access,
  Action,
  AgentModel,
  NativeFacts,
  Publication,
  Row,
  Snapshot,
} from "./contract";
import { gaps, type Gap } from "./gaps";

const t = UI.token;
const vertical = {
  display: "flex",
  flexDirection: "column",
  gap: t("metrics.inset"),
};
const reading = {
  color: t("colors.text"),
  fontSize: t("typography.reading"),
  lineHeight: t("typography.reading_leading"),
};
const body = { color: t("colors.secondary"), fontSize: t("typography.body") };
const muted = { color: t("colors.muted"), fontSize: t("typography.body") };
const title = {
  color: t("colors.text"),
  fontSize: t("typography.title"),
  fontWeight: t("typography.semibold_weight"),
};
/** Question/approval card (agent-panel.md §3, design review 2026-11-01). */
const card = (popover: boolean) => ({
  ...vertical,
  background: t(popover ? "colors.panel" : "colors.panel_secondary"),
  borderRadius: t("metrics.control_radius"),
  padding: t("metrics.inset"),
});
// The composer's pop-ups give way (and truncate) before its buttons do, so
// Send stays in a narrow popover.
const shrink = { flexShrink: 1, overflow: "hidden" } as const;
const keep = { flexShrink: 0 } as const;
const copy = {
  editor: "Composition only",
  supervised: "Supervised",
  auto: "Auto",
  full: "Full access",
} as const;
const effortCopy: Record<string, string> = {
  "": "Default effort",
  none: "No effort",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Max",
  ultra: "Ultra",
};
export function elapsed(ms: number) {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
function Placeholder({
  gap,
  label,
  children,
}: {
  gap: Gap;
  label: string;
  children?: ComponentChildren;
}) {
  return (
    <div
      role="group"
      label={label}
      metadata={{ apiGap: gap, detail: gaps[gap] }}
    >
      {children}
    </div>
  );
}
/** The kit owns system push/default/destructive button presentation. */
function Push({
  id,
  label,
  selected,
  onClick,
}: {
  id?: string;
  label: string;
  selected?: boolean;
  onClick?: () => void;
}) {
  return (
    <UI.PushButton
      id={id}
      label={label}
      role={selected ? "default" : "normal"}
      disabled={!onClick}
      onClick={onClick}
    />
  );
}
/** Steps a turn's work is made of: tools, tasks and thinking. */
const STEPS = new Set(["tool", "task", "thought"]);
type Part = { work: Row[] } | { row: Row };
/** Consecutive steps fold into one work line per turn (agent-panel.md §3). */
function parts(rows: Row[]): Part[] {
  const out: Part[] = [];
  for (const row of rows) {
    const last = out[out.length - 1];
    if (!STEPS.has(str(row.value.kind))) out.push({ row });
    else if (last && "work" in last) last.work.push(row);
    else out.push({ work: [row] });
  }
  return out;
}
function Rows({
  s,
  n,
  send,
}: {
  s: Snapshot;
  n: NativeFacts;
  send: (a: Action) => void;
}) {
  const all = parts(s.rows.items);
  return (
    <UI.List
      id="agent-conversation"
      label="Agent conversation"
      tabIndex={0}
      metadata={{
        apiGap: "variableList",
        count: s.rows.count,
        range: s.rows.range,
        textSelected: n.textSelected,
        // Bottom anchored: new content keeps the latest in view unless the
        // person scrolled up.
        scrollAnchor: "end",
      }}
      state={{ keyboardFocus: n.keyboardFocus === "conversation" }}
      style={{
        ...vertical,
        minWidth: t("metrics.hit"),
        minHeight: t("metrics.hit"),
      }}
    >
      {all.map((part, index) =>
        "work" in part ? (
          <Work
            key={`work-${part.work[0].id}`}
            rows={part.work}
            live={
              (s.runId !== null && index === all.length - 1) ||
              part.work.some(
                (r) => r.value.status === "running" || r.value.live === true,
              )
            }
            n={n}
            send={send}
          />
        ) : (
          <Message key={part.row.id} row={part.row} n={n} send={send} />
        ),
      )}
    </UI.List>
  );
}
const num = (value: unknown) =>
  typeof value === "number" && Number.isFinite(value) ? value : undefined;
/** v1's duration words: "<1s", "4s", "1m 12s", "1h 3m". */
export function duration(ms: number) {
  if (!Number.isFinite(ms) || ms < 0) return "";
  const seconds = Math.round(ms / 1000);
  if (seconds < 1) return "<1s";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
/** A reasoning stretch's chip: its opening sentence, on one line (v1). */
export function thoughtLead(text: string) {
  const flat = text.replace(/\*\*/g, "").replace(/\s+/g, " ").trim();
  const sentence = flat.match(/^.{12,}?[.!?](\s|$)/)?.[0]?.trim();
  return sentence && sentence.length < flat.length ? sentence : flat;
}
const span = (v: Row["value"]) => {
  const start = num(v.startedAt);
  const end = num(v.endedAt);
  return start !== undefined && end !== undefined ? end - start : undefined;
};
/** A step's symbol (SF Symbols), by the family of what it did (v1 families). */
function stepSymbol(v: Row["value"]) {
  if (v.kind === "thought") return "brain";
  if (v.kind === "task") return "person.2";
  const name = str(v.toolName).toLowerCase().split("__").at(-1) ?? "";
  const by: [RegExp, string][] = [
    [/^(probe_media|sample_media_frames|media_contact_sheet)$/, "film"],
    [/^(media_waveform|transcribe_media|generate_captions)$/, "waveform"],
    [/^(delegate_task|task_status|task_cancel|agent|task)/, "person.2"],
    [
      /panel|workspace|project_state|render_frames|apply_commands|edit_video|rollback|check_project/,
      "square.on.square",
    ],
    [
      /^(web|search|browse|fetch|grep|glob|find|ls$|list)|url/,
      "magnifyingglass",
    ],
    [/^(bash|command|shell|terminal|exec|run|process)/, "terminal"],
    [
      /^(edit|write|create|patch|apply|delete|move|multiedit)|file_change/,
      "pencil",
    ],
    [/^(read|view|cat)/, "doc.text"],
    [/^(compact|wait)/, "clock"],
    [/^(image|render|draw|screenshot)/, "photo"],
    [/^computer/, "cursorarrow.rays"],
  ];
  return by.find(([p]) => p.test(name))?.[1] ?? "wrench.and.screwdriver";
}
/** One stretch of a turn's work, in v1's grammar: a quiet header ("4 tool
 * calls · 1 failed · 21s"), then a row per step. Open while the turn runs
 * (or a subagent failed) and folded once it is done; the person may open it.
 * Reasoning alone needs no header: its row is its own disclosure. */
function Work({
  rows,
  live,
  n,
  send,
}: {
  key?: string;
  rows: Row[];
  live: boolean;
  n: NativeFacts;
  send: (a: Action) => void;
}) {
  const [chosen, choose] = useState<boolean | null>(null);
  const tools = rows.filter((r) => r.value.kind !== "thought");
  const failed = tools.filter((r) => r.value.status === "error").length;
  const open =
    chosen ??
    (live ||
      tools.some((r) => r.value.kind === "task" && r.value.status === "error"));
  if (!tools.length)
    return (
      <>
        {rows.map((row) => (
          <Step key={row.id} row={row} n={n} send={send} />
        ))}
      </>
    );
  const starts = rows
    .map((r) => num(r.value.startedAt))
    .filter((v) => v !== undefined);
  const ends = rows
    .map((r) => num(r.value.endedAt))
    .filter((v) => v !== undefined);
  const took =
    !live && starts.length && ends.length
      ? duration(Math.max(...ends) - Math.min(...starts))
      : "";
  const label = [
    `${tools.length} tool ${tools.length === 1 ? "call" : "calls"}`,
    !live && failed ? `${failed} failed` : "",
    took,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      role="group"
      label={label}
      style={{ display: "flex", flexDirection: "column" }}
    >
      <UI.ListItem
        id={`${rows[0].id}-work`}
        label={`${label}, ${open ? "expanded" : "collapsed"}`}
        onSelect={() => choose(!open)}
        style={line(muted.color)}
      >
        <div
          metadata={{ symbol: open ? "chevron.down" : "chevron.right" }}
          style={glyph(muted.color)}
        />
        <text style={muted}>{label}</text>
      </UI.ListItem>
      {open &&
        rows.map((row) => <Step key={row.id} row={row} n={n} send={send} />)}
    </div>
  );
}
/** A row's content laid on one line: glyph, words, trailing state. */
const line = (color: ReturnType<typeof t>) =>
  ({
    display: "flex",
    flexDirection: "row",
    alignItems: "center",
    gap: t("metrics.gap"),
    color,
  }) as const;
const glyph = (color: ReturnType<typeof t>) => ({
  color,
  symbolSize: t("metrics.symbol_inline"),
  flexShrink: 0,
});
const STATE: Record<string, string> = {
  error: "Failed",
  stopped: "Stopped",
  cancelled: "Stopped",
  waiting: "Waiting",
  continued: "Continued",
};
/** One step, as v1's tool rows: the glyph (a spinner while it runs), what it
 * was ("Run", "Edit", "Thought for 4s") and a hairline chip with what it was
 * done to (the command, the files, the query, the media; a thought's first
 * sentence), cut in the middle; then its state or its time. Opening it shows
 * the whole of it: the full argument and the output, or the error. */
function Step({
  row,
  n,
  send,
}: {
  key?: string;
  row: Row;
  n: NativeFacts;
  send: (a: Action) => void;
}) {
  const v = row.value;
  const thought = v.kind === "thought";
  const status = str(v.status);
  const running = thought ? v.live === true : status === "running";
  const took = span(v);
  const label = thought
    ? running
      ? "Thinking"
      : took !== undefined
        ? `Thought for ${duration(took)}`
        : "Thought"
    : str(v.title) || str(v.label) || str(v.toolName) || str(v.text);
  // What it worked on: the media or layer by its name in the project, else
  // the provider's argument, never only the tool's own id.
  const detail = str(v.detail);
  const argument = thought
    ? ""
    : str(v.subjectName) ||
      (detail && detail !== str(v.toolName) ? detail : "");
  const chip = (thought ? thoughtLead(str(v.label)) : argument)
    .replace(/\s+/g, " ")
    .trim();
  const result = thought
    ? str(v.label)
    : [argument, str(v.output)].filter(Boolean).join("\n\n");
  const state =
    STATE[status] ?? STATE[str((v.task as { status?: unknown })?.status)];
  const after =
    state ?? (!running && took !== undefined && !thought ? duration(took) : "");
  const open = row.open && !!result;
  const task = v.kind === "task" || v.toolName === "delegate_task";
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <UI.ListItem
        id={row.id}
        label={[label, chip, state].filter(Boolean).join(", ")}
        onSelect={() => {
          if (result)
            send({ type: "disclose", id: row.commandId, open: !row.open });
        }}
        state={{ hover: n.hoverRow === row.id }}
        style={line(body.color)}
      >
        {running && !thought ? (
          <UI.ProgressRow label={`${label} is running`}>{[]}</UI.ProgressRow>
        ) : (
          <div
            metadata={{ symbol: stepSymbol(v) }}
            style={glyph(muted.color)}
          />
        )}
        <text style={{ ...body, flexShrink: 0 }}>{label}</text>
        {chip && !(open && thought) && (
          <div
            data-pm-variant={thought ? undefined : "code"}
            style={{
              flexShrink: 1,
              borderWidth: t("metrics.border"),
              borderColor: t("colors.separator"),
              borderRadius: t("metrics.inner_radius"),
              paddingLeft: t("metrics.inner_radius"),
              paddingRight: t("metrics.inner_radius"),
            }}
          >
            <text
              style={{
                ...muted,
                fontSize: t("typography.minimum"),
                whiteSpace: "nowrap",
                textOverflow: "middle",
              }}
            >
              {chip}
            </text>
          </div>
        )}
        <div style={{ flexGrow: 1 }} />
        {after && (
          <div
            metadata={
              status === "error" ? { symbol: "xmark.circle.fill" } : undefined
            }
            style={line(
              t(status === "error" ? "colors.danger" : "colors.muted"),
            )}
          >
            <text
              style={
                status === "error"
                  ? { ...body, color: t("colors.danger") }
                  : muted
              }
            >
              {after}
            </text>
          </div>
        )}
        {running && task && (
          <UI.ToolbarItem
            label={`Stop ${label}`}
            onClick={() => send({ type: "stopTask", id: row.commandId })}
            style={{ color: t("colors.accent_text") }}
          >
            Stop
          </UI.ToolbarItem>
        )}
      </UI.ListItem>
      {open && (
        <div
          data-pm-variant={thought ? undefined : "code"}
          style={{
            background: t("colors.field"),
            borderRadius: t("metrics.control_radius"),
            padding: t("metrics.text_padding"),
          }}
        >
          <text
            style={
              thought ? body : { ...body, fontSize: t("typography.minimum") }
            }
          >
            {result}
          </text>
        </div>
      )}
    </div>
  );
}
const str = (value: unknown) => (typeof value === "string" ? value : "");
const MEDIA_KIND: Record<string, string> = {
  image: "Image",
  video: "Video",
  audio: "Sound",
};
function Message({
  row,
  n,
  send,
}: {
  key?: string;
  row: Row;
  n: NativeFacts;
  send: (a: Action) => void;
}) {
  const v = row.value;
  const text = str(v.text) || str(v.content) || str(v.label);
  const kind = str(v.kind);
  const questions = Array.isArray(v.questions) ? v.questions : [];
  const firstQuestion = questions[0];
  const questionTitle =
    str(v.title) ||
    text ||
    (firstQuestion &&
    typeof firstQuestion === "object" &&
    !Array.isArray(firstQuestion)
      ? str(firstQuestion.question)
      : "");
  if (kind === "question" && v.status === "answered") {
    const answers =
      v.answers && typeof v.answers === "object" && !Array.isArray(v.answers)
        ? Object.values(v.answers).map(str).filter(Boolean).join(", ")
        : "";
    const label = answers ? `Answered: ${answers}` : "Skipped";
    return (
      <div role="listitem" label={label} metadata={{ rowId: row.id }}>
        <text style={muted}>{label}</text>
      </div>
    );
  }
  if (kind === "question" && v.status === "closed")
    return (
      <div
        role="listitem"
        label={`Not answered: ${questionTitle}`}
        metadata={{ rowId: row.id }}
      >
        <text style={muted}>Not answered</text>
      </div>
    );

  if (v.role === "user")
    return (
      <div
        role="listitem"
        label={`You said: ${text}`}
        metadata={{ rowId: row.id, apiGap: "richText" }}
        style={{
          display: "flex",
          flexDirection: "row",
          justifyContent: "end",
          // agent-panel.md §3: the message takes at most 84 % of the width.
          paddingLeft: t("metrics.message_inset"),
        }}
      >
        <div
          style={{
            ...reading,
            background: t("colors.field"),
            borderRadius: t("metrics.panel_radius"),
            paddingTop: t("metrics.text_padding"),
            paddingBottom: t("metrics.text_padding"),
            paddingLeft: t("metrics.inset"),
            paddingRight: t("metrics.inset"),
          }}
        >
          <text>{text}</text>
        </div>
      </div>
    );
  if (kind === "question")
    return (
      <div
        role="listitem"
        label={`Question from the agent: ${questionTitle}`}
        metadata={{ rowId: row.id }}
        style={card(n.presentation === "popover")}
      >
        <text style={title}>{questionTitle}</text>
        {questions.map((q) =>
          q && typeof q === "object" && !Array.isArray(q) ? (
            <div role="group" label={str(q.question)} style={{ ...vertical }}>
              <text style={body}>{str(q.question)}</text>
              {(Array.isArray(q.options) ? q.options : []).map((o) => {
                const label =
                  o && typeof o === "object" && !Array.isArray(o)
                    ? str(o.label)
                    : "";
                const id = str(q.id);
                return (
                  <Push
                    id={`${row.id}-${id}-${label}`}
                    label={label}
                    selected={row.chosen[id]?.includes(label)}
                    onClick={() =>
                      send({
                        type: "chooseAnswer",
                        step: row.commandId,
                        question: id,
                        option: label,
                      })
                    }
                  />
                );
              })}
            </div>
          ) : null,
        )}
        {v.transport === "reply" && (
          <Push
            id={`${row.id}-skip`}
            label="Skip"
            onClick={() =>
              send({ type: "answer", step: row.commandId, skip: true })
            }
          />
        )}
        <Push
          id={`${row.id}-answer`}
          label="Answer"
          onClick={
            Object.values(row.chosen).some((a) => a.length > 0)
              ? () => send({ type: "answer", step: row.commandId, skip: false })
              : undefined
          }
        />
      </div>
    );
  // publish_visual_reply: each reply opens in its own sandboxed viewer.
  const visuals = Array.isArray(v.visualReplies) ? v.visualReplies : [];
  // Images, videos and sounds the reply names (the host lists them); each
  // opens in its own app, a web address only when the person asks.
  const media = Array.isArray(v.media) ? v.media : [];
  if (visuals.length || media.length)
    return (
      <div
        role="listitem"
        label={`Agent: ${text}`}
        metadata={{ rowId: row.id }}
        style={vertical}
      >
        {text && <text style={reading}>{text}</text>}
        {visuals.map((reply) => {
          const r =
            reply && typeof reply === "object" && !Array.isArray(reply)
              ? reply
              : {};
          return (
            <div
              key={str(r.id)}
              role="group"
              label={str(r.title)}
              metadata={{ visualReply: str(r.id) }}
              style={card(false)}
            >
              <text style={title}>{str(r.title)}</text>
              <text style={muted}>Interactive visualization</text>
              <Push
                id={`${row.id}-${str(r.id)}`}
                label="Open"
                onClick={() => send({ type: "openVisual", id: str(r.id) })}
              />
            </div>
          );
        })}
        {media.map((item, index) => {
          const m =
            item && typeof item === "object" && !Array.isArray(item)
              ? item
              : {};
          const path = str(m.path);
          const remote = m.remote === true;
          return (
            <div
              key={path}
              role="group"
              label={str(m.name)}
              metadata={{ media: path }}
              style={card(false)}
            >
              <text style={title}>{str(m.name)}</text>
              <text style={muted}>
                {(MEDIA_KIND[str(m.kind)] ?? "File") +
                  (remote ? " on the web" : "")}
              </text>
              <Push
                id={`${row.id}-media-${index}`}
                label={remote ? "Open in Browser" : "Open"}
                onClick={() => send({ type: "openMedia", path, reveal: false })}
              />
              {!remote && (
                <Push
                  id={`${row.id}-media-${index}-reveal`}
                  label="Show in Finder"
                  onClick={() =>
                    send({ type: "openMedia", path, reveal: true })
                  }
                />
              )}
            </div>
          );
        })}
      </div>
    );
  if (kind === "error" || v.error === true)
    return (
      <div
        role="listitem"
        label={text}
        metadata={{ rowId: row.id }}
        style={{ ...vertical, gap: t("metrics.gap"), alignItems: "start" }}
      >
        {/* The triangle in the danger colour leads the message (§3 Error item). */}
        <div
          metadata={{ symbol: "exclamationmark.triangle.fill" }}
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: t("metrics.gap"),
            color: t("colors.danger"),
            fontSize: t("typography.body"),
          }}
        >
          <text style={{ ...body, color: t("colors.text") }}>{text}</text>
        </div>
        <Push label="Try Again" />
      </div>
    );
  if (v.nativeResult || kind === "result")
    return (
      <Placeholder gap="preview" label="Rendered result">
        <text style={title}>Rendered result</text>
        <text style={reading}>{text}</text>
        <Push
          label="Keep Change"
          onClick={() => send({ type: "keepResult" })}
        />
        <Push label="Undo Change" />
        <Push label="Add to Timeline" />
        <Push label="Show in Finder" />
      </Placeholder>
    );
  return (
    <div
      role="listitem"
      label={`Agent: ${text}`}
      metadata={{ rowId: row.id, apiGap: "richText" }}
      style={reading}
    >
      <text>{text}</text>
    </div>
  );
}
/** A saved choice the catalog no longer lists stays visible rather than blank. */
function pairs(entries: [string, string][], current: string) {
  return entries.some(([v]) => v === current)
    ? entries
    : [...entries, [current, current] as [string, string]];
}
/** One row of the window's field menu (the kit's searchable menu). */
export interface MenuOption {
  label: string;
  value: string;
  checked?: boolean;
  /** The section it is listed under. */
  category?: string;
}
/** Opens the field menu under `anchor` (a node id); answers the chosen
 * value, or null when it closed unchosen. */
export type Menu = (request: {
  anchor: number;
  placeholder: string;
  options: MenuOption[];
}) => Promise<unknown>;
/** v1's access modes, in v1's order (legacy/assistant/spatial.ts). */
const ACCESS: Access[] = ["editor", "supervised", "auto", "full"];
/** v1's slash commands' copy (slash-commands.ts). */
const SLASH: Record<string, string> = {
  new: "/new",
  model: "/model",
  effort: "/effort",
  provider: "/provider",
  attach: "/attach",
  settings: "/settings",
  stop: "/stop",
};
/** A choice the menu shows for one slash command or one of its values. */
function slashLabel(s: Snapshot, id: string, value: string | null) {
  if (value === null) return SLASH[id] ?? `/${id}`;
  const command = id.slice(0, id.length - value.length - 1);
  const list =
    command === "model"
      ? s.catalog.models
      : command === "provider"
        ? s.catalog.providers
        : [];
  const label =
    command === "effort"
      ? (effortCopy[value] ?? value)
      : (list.find(([v]) => v === value)?.[1] ?? value);
  return `/${command} ${label}`;
}
/** A borderless trigger in a bar or header that opens the window's menu:
 * its value, then the pop-up chevron (as a bar NSPopUpButton). */
function Trigger({
  id,
  label,
  text,
  disabled,
  open,
  widest,
}: {
  id: string;
  label: string;
  text: string;
  disabled?: boolean;
  open: (anchor: number) => void;
  /** The most it may take (the header's thread title, `m.value_column`). */
  widest?: ReturnType<typeof t>;
}) {
  return (
    <UI.ToolbarItem
      id={id}
      label={label}
      aria-haspopup="menu"
      disabled={disabled}
      onClick={(event) => {
        if (!disabled) open(Number(event.currentTarget));
      }}
      style={{
        ...line(body.color),
        flexShrink: 1,
        ...(widest ? { maxWidth: widest } : {}),
      }}
    >
      <text
        style={{
          ...body,
          flexShrink: 1,
          whiteSpace: "nowrap",
          textOverflow: "ellipsis",
        }}
      >
        {text}
      </text>
      <div
        metadata={{ symbol: "chevron.up.chevron.down" }}
        style={glyph(muted.color)}
      />
    </UI.ToolbarItem>
  );
}
export function AgentView({
  publication: { snapshot: s, native: n },
  send,
  menu,
}: {
  publication: Publication;
  send: (action: Action) => void;
  menu?: Menu;
}) {
  // The slash menu opens once per draft that asks for it; closing it
  // unchosen leaves the draft as typed.
  const slashOpened = useRef<string | null>(null);
  const slashDraft =
    s.slash.visible && s.slash.choices.length ? s.draft.text : null;
  useLayoutEffect(() => {
    if (slashDraft === null) {
      slashOpened.current = null;
      return;
    }
    if (!menu || slashOpened.current === slashDraft) return;
    slashOpened.current = slashDraft;
    const composer = composerNode.current;
    if (composer === null) return;
    void menu({
      anchor: composer,
      placeholder: "Search commands",
      options: s.slash.choices.map((c) => ({
        label: slashLabel(s, c.id, c.value),
        value: c.id,
      })),
    }).then((id) => {
      if (typeof id === "string") send({ type: "chooseSlash", id });
    });
  }, [slashDraft]);
  const composerNode = useRef<number | null>(null);
  const composerFocused = useRef(false);
  if (!n.visible) return null;
  const working = s.runId !== null;
  const available = n.connection === "connected";
  const disabled = !available || s.composerDisabled;
  const placeholder = !available
    ? "Connect an agent to start"
    : working
      ? "Add direction…"
      : "Describe a change…";
  const explanation = n.extensionContext
    ? "Build and improve your extensions."
    : s.settings.access === "editor"
      ? "Your ideas. Editable motion."
      : "Create, research, and build with your project.";
  const popover = n.presentation === "popover";
  const ground = t(popover ? "colors.floating" : "colors.panel");
  // The model pop-up reads "{model} · {effort}" (agent-panel.md §3).
  const modelName =
    pairs(s.catalog.models, s.settings.model).find(
      ([v]) => v === s.settings.model,
    )?.[1] || "Default";
  const modelText =
    s.catalog.efforts.length > 1 && s.settings.effort
      ? `${modelName} · ${effortCopy[s.settings.effort] ?? s.settings.effort}`
      : modelName;
  return (
    <div
      id="agent-panel"
      role="group"
      label="Agent conversation"
      metadata={{
        apiGap: "shell",
        presentation: n.presentation,
        provisional: true,
        atMinimumDockWidth: n.atMinimumDockWidth,
      }}
      style={{
        display: "flex",
        flexDirection: "column",
        ...(popover ? { width: t("metrics.agent_width") } : {}),
        background: ground,
        borderRadius: t("metrics.panel_radius"),
      }}
    >
      <div
        id="agent-header"
        data-pm-slot="header"
        role="group"
        label="Agent"
        title={popover ? "Drag to move or pin to workspace" : undefined}
        metadata={{ apiGap: "shell" }}
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          height: t("metrics.header"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
        }}
      >
        <text style={{ ...title, flexShrink: 0 }}>Agent</text>
        <div
          style={{
            flexShrink: 1,
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
          }}
        >
          <Trigger
            id="agent-threads"
            label={`Switch thread, ${s.threadTitle || "New thread"}`}
            // Docked, the header shares the dock's tab row: the trigger
            // names the list, and the tab keeps its name.
            text={popover ? s.threadTitle || "New thread" : "Threads"}
            widest={t("metrics.value_column")}
            disabled={!menu}
            open={(anchor) => {
              const threads = s.threads ?? [];
              const current = threads.find((t) => t.id === s.threadId);
              void menu?.({
                anchor,
                placeholder: "Search threads…",
                options: [
                  ...(s.composerDisabled
                    ? []
                    : [{ label: "New Thread", value: "new" }]),
                  ...threads.map((t) => ({
                    label: t.busy ? `${t.title} · Working…` : t.title,
                    value: `thread:${t.id}`,
                    checked: t.id === s.threadId,
                    category: "Threads",
                  })),
                  // Apart from the list, last: the one destructive row.
                  ...(current
                    ? [
                        {
                          label: "Delete Thread…",
                          value: `delete:${current.id}`,
                          category: "This Thread",
                        },
                      ]
                    : []),
                ],
              }).then((choice) => {
                if (choice === "new") send({ type: "newThread" });
                else if (
                  typeof choice === "string" &&
                  choice.startsWith("thread:")
                ) {
                  const id = choice.slice(7);
                  if (id !== s.threadId) send({ type: "switchThread", id });
                } else if (
                  typeof choice === "string" &&
                  choice.startsWith("delete:")
                )
                  send({ type: "deleteThread", id: choice.slice(7) });
              });
            }}
          />
          <UI.ToolbarItem
            disabled={s.composerDisabled}
            id="agent-new"
            label="New Thread"
            title="New Thread"
            metadata={{ symbol: "square.and.pencil" }}
            style={{ symbolSize: t("metrics.symbol_panel") }}
            onClick={() => {
              if (!s.composerDisabled) send({ type: "newThread" });
            }}
          />
          {popover && (
            <UI.ToolbarItem
              id="agent-presentation"
              label="Pin to Workspace"
              title="Pin to Workspace"
              metadata={{ symbol: "pin" }}
              style={{ symbolSize: t("metrics.symbol_panel") }}
              onClick={() => send({ type: "shell", action: "pin" })}
            />
          )}
        </div>
      </div>
      <div
        style={{
          ...vertical,
          // The conversation takes the free height and scrolls inside it;
          // the composer stays at the foot.
          flexGrow: 1,
          overflow: "hidden",
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
        }}
      >
        {s.saveError && (
          <div
            role="status"
            label="Thread history couldn’t be saved. Keep this window open."
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
          >
            <text style={{ ...body, color: t("colors.warning") }}>
              Thread history couldn’t be saved. Keep this window open.
            </text>
          </div>
        )}
        {/* A live run keeps its conversation, whatever its account says now. */}
        {!working && n.connection === "checking" ? (
          <UI.ProgressRow label="Checking connection…" />
        ) : !working && n.connection === "waiting" ? (
          <>
            <UI.ProgressRow label="Waiting for sign-in…" />
            <Push
              label="Open Settings"
              onClick={() => send({ type: "openSettings" })}
            />
          </>
        ) : !working && n.connection === "unavailable" ? (
          <UI.EmptyState
            label="Connect Your Agent"
            explanation={n.reason}
            action={
              // A pair of buttons sits side by side, as in macOS alerts.
              <div
                style={{
                  display: "flex",
                  flexDirection: "row",
                  gap: t("metrics.gap"),
                }}
              >
                <Push
                  label="Connection Settings…"
                  onClick={() => send({ type: "openSettings" })}
                />
                <Push
                  label="Try Again"
                  onClick={() => send({ type: "retryConnection" })}
                />
              </div>
            }
          />
        ) : (
          <>
            {n.loadingHistory && (
              <UI.ProgressRow label="Loading earlier messages…" />
            )}
            {s.rows.count === 0 ? (
              <UI.EmptyState
                label="Make Your Next Move"
                explanation={explanation}
                action={null}
              />
            ) : (
              <Rows s={s} n={n} send={send} />
            )}
            {working && (
              <UI.ProgressRow
                label={
                  s.stopping
                    ? "Stopping…"
                    : `Working · ${elapsed(s.elapsedMs ?? 0)}`
                }
              />
            )}
          </>
        )}
        {s.stopError && (
          <div
            role="status"
            label="Couldn’t stop this task."
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
          >
            <text style={{ ...body, color: t("colors.danger") }}>
              Couldn’t stop this task.
            </text>
          </div>
        )}
        {s.showJump && (
          <UI.ToolbarItem
            label="Scroll to Latest"
            disabled
            metadata={{ apiGap: "variableList" }}
          />
        )}
      </div>
      <div
        id="agent-composer"
        role="group"
        label="Message Powermove agent"
        metadata={{
          apiGap: "composer",
          placeholder,
          hint: "Return to send, Shift-Return for a new line",
          selection: s.draft.selection,
          maxLinesToken: "metrics.composer_max_lines",
          disabled,
        }}
        state={{ dropTarget: n.dropTarget }}
        title="Return to send · Shift-Return for a new line · / for shortcuts"
        style={{
          display: "flex",
          flexDirection: "column",
          // Two concentric surfaces, as v1: a quiet tray holding the field
          // (the draft with attach and send) over the run's options.
          background: t(n.dropTarget ? "colors.accent_wash" : "colors.field"),
          borderRadius: t("metrics.panel_radius"),
          minWidth: t("metrics.hit"),
          margin: t("metrics.inset"),
          padding: t("metrics.gap"),
        }}
      >
        <div
          id="agent-field"
          style={{
            ...vertical,
            background: ground,
            borderRadius: t("metrics.inner_radius"),
            minHeight: t("metrics.hit"),
            paddingLeft: t("metrics.gap"),
            paddingRight: t("metrics.gap"),
          }}
        >
          {s.attachments.map((a) => (
            <div
              key={a.id}
              role="group"
              label={`${a.name}, attachment`}
              metadata={{ apiGap: "composer", armed: s.draft.armed === a.id }}
            >
              <UI.ToolbarItem
                label={`${a.name}, attachment`}
                onClick={() => send({ type: "openAttachment", id: a.id })}
              >
                {a.name}
              </UI.ToolbarItem>
              <UI.ToolbarItem
                label={`Remove ${a.name}`}
                title={`Remove ${a.name}`}
                onClick={() => send({ type: "removeAttachment", id: a.id })}
              />
            </div>
          ))}
          {n.dropTarget ? (
            <text style={{ ...body, color: t("colors.accent_text") }}>
              Drop to attach
            </text>
          ) : (
            <UI.TextField
              multiline
              id="agent-composer-text"
              label="Message Powermove agent"
              value={s.draft.text}
              disabled={disabled}
              metadata={{ placeholder, submitOnReturn: true }}
              state={{ keyboardFocus: n.keyboardFocus === "composer" }}
              tabIndex={0}
              onInput={(event) => {
                // The slash menu hangs from the composer, and shows only while
                // it is focused: typing in it says it is (the host reports
                // no focus of its own for a hosted editor).
                composerNode.current = Number(event.currentTarget);
                if (!composerFocused.current) {
                  composerFocused.current = true;
                  send({ type: "focus", focused: true });
                }
                const text = String(event.value ?? "");
                send({
                  type: "draft",
                  text,
                  selection: [text.length, text.length],
                  base: s.draft.text,
                });
              }}
              onChange={(event) => {
                // The native editor reports the text as it is typed, so Send
                // follows the draft.
                const text = String(event.value ?? "");
                if (text !== s.draft.text)
                  send({
                    type: "draft",
                    text,
                    selection: [text.length, text.length],
                    base: s.draft.text,
                  });
              }}
              onKeyDown={(event) => {
                event.stopPropagation();
                send(composerKey(event));
              }}
              style={{
                ...reading,
                background: ground,
                borderRadius: t("metrics.inner_radius"),
                height: t("metrics.hit"),
                minWidth: t("metrics.hit"),
              }}
            />
          )}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "space-between",
              height: t("metrics.hit"),
            }}
          >
            <UI.ToolbarItem
              id="agent-attach"
              style={keep}
              label="Add Attachments"
              attr={{ symbol: "plus" }}
              title="Add Attachments"
              disabled={s.composerDisabled || s.attachments.length >= 6}
              onClick={() => send({ type: "attach" })}
            />
            {working ? (
              <UI.ToolbarItem
                id="agent-stop"
                label="Stop current run"
                title="Stop Current Run"
                metadata={{ symbol: "stop.circle.fill" }}
                style={{ ...keep, symbolSize: t("metrics.symbol_action") }}
                disabled={s.stopping}
                onClick={() => {
                  if (!s.stopping) send({ type: "stop" });
                }}
              />
            ) : (
              <UI.ToolbarItem
                id="agent-send"
                label="Send"
                title="Send"
                metadata={{ symbol: "arrow.up.circle.fill" }}
                style={{ ...keep, symbolSize: t("metrics.symbol_action") }}
                disabled={disabled || !s.sendable}
                onClick={() => {
                  if (!disabled && s.sendable) send({ type: "submit" });
                }}
              />
            )}
          </div>
        </div>
        <div
          id="agent-options"
          style={{
            display: "flex",
            flexDirection: "row",
            // One row at any width: the model's words give way first, then
            // the access mode's (they truncate, never wrap).
            alignItems: "center",
            justifyContent: "space-between",
            gap: t("metrics.gap"),
            height: t("metrics.hit"),
            paddingTop: t("metrics.pill_inset"),
          }}
        >
          <Trigger
            id="agent-model"
            label={`Model and reasoning effort, ${modelText}`}
            text={modelText}
            disabled={!menu || working || s.composerDisabled}
            open={(anchor) => {
              void menu?.({
                anchor,
                placeholder: "Search models",
                options: [
                  ...pairs(s.catalog.providers, s.settings.provider).map(
                    ([v, label]) => ({
                      label,
                      value: `provider:${v}`,
                      checked: v === s.settings.provider,
                      category: "Provider",
                    }),
                  ),
                  ...pairs(s.catalog.models, s.settings.model).map(
                    ([v, label]) => ({
                      label,
                      value: `model:${v}`,
                      checked: v === s.settings.model,
                      category: "Model",
                    }),
                  ),
                  ...(s.catalog.efforts.length > 1
                    ? s.catalog.efforts.map((v) => ({
                        label: effortCopy[v] ?? v,
                        value: `effort:${v}`,
                        checked: v === s.settings.effort,
                        category: "Reasoning Effort",
                      }))
                    : []),
                ],
              }).then((choice) => {
                if (typeof choice !== "string") return;
                const [kind, value] = [
                  choice.slice(0, choice.indexOf(":")),
                  choice.slice(choice.indexOf(":") + 1),
                ];
                if (kind === "provider")
                  send({ type: "setProvider", provider: value });
                else if (kind === "model")
                  send({
                    type: "setModel",
                    model: value,
                    effort: s.settings.effort,
                  });
                else if (kind === "effort")
                  send({
                    type: "setModel",
                    model: s.settings.model,
                    effort: value,
                  });
              });
            }}
          />
          <Trigger
            id="agent-access"
            label={`Agent access, ${copy[s.settings.access]}`}
            text={copy[s.settings.access]}
            disabled={!menu || working || s.composerDisabled}
            open={(anchor) => {
              void menu?.({
                anchor,
                placeholder: "Search",
                options: ACCESS.map((v) => ({
                  label: copy[v],
                  value: v,
                  checked: v === s.settings.access,
                  category: "How should agent actions be approved?",
                })),
              }).then((choice) => {
                if (typeof choice === "string" && choice !== s.settings.access)
                  send({ type: "setAccess", access: choice as Access });
              });
            }}
          />
        </div>
      </div>
    </div>
  );
}
/** Mounted extension; onError is required so rejected work is never silently lost. */
export function AgentPanel({
  model,
  onError,
  menu,
}: {
  model: AgentModel;
  onError: (error: unknown) => void;
  menu?: Menu;
}) {
  const [publication, setPublication] = useState(() => model.inspect());
  useLayoutEffect(() => {
    const update = () => {
      try {
        setPublication(model.inspect());
      } catch (error) {
        onError(error);
      }
    };
    const stop = model.subscribe(update);
    update();
    return stop;
  }, [model, onError]);
  return (
    <AgentView
      publication={publication}
      menu={menu}
      send={(action) => {
        void model.dispatch(action).catch(onError);
      }}
    />
  );
}
const namedKeys: Record<string, string> = {
  backspace: "Backspace",
  delete: "Delete",
  tab: "Tab",
};
export function composerKey(event: SurfaceEvent): Action {
  const { key, metaKey, ctrlKey, shiftKey, altKey, repeat, composing } = event;
  return {
    type: "key",
    key: {
      key: namedKeys[key ?? ""] ?? key,
      metaKey,
      ctrlKey,
      shiftKey,
      altKey,
      repeat,
      composing,
    },
  };
}
