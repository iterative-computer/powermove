/**
 * The Agent panel, natively. It follows the original app’s panel for
 * state, copy and keyboard (thread bar, conversation with folded work, tool
 * groups, questions, errors with Try Again, composer with its run options)
 * and draws it with the native kit and pm-design tokens.
 */
import type { ComponentChildren } from "preact";
import { useEffect, useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { Json, SurfaceEvent } from "../../src/protocol";
import type {
  Access,
  Action,
  AgentModel,
  NativeFacts,
  Publication,
  Row,
  Snapshot,
} from "./contract";
import {
  ACCESS,
  FAMILY_SYMBOL,
  breakable,
  activity,
  effortLabel,
  entries,
  markdown,
  plain,
  presentError,
  providerName,
  relativeOpened,
  slashCopy,
  str,
  suggestions,
  type Activity,
  type Block,
  type Entry,
  type Message,
  type Run,
  type ToolDetail,
  type ToolsRow,
} from "./present";

const t = UI.token;
/** The composer and answer fields hold the model's draft (sent as actions),
 * not a host-bound value, so they take no binding. */
const Field = UI.TextField as (
  props: Omit<Parameters<typeof UI.TextField>[0], "binding"> & {
    binding?: string;
  },
) => ReturnType<typeof UI.TextField>;
type Send = (action: Action) => void;
type Style = Record<string, string | number | null | { token: string }>;
type Obj = Record<string, Json>;
const obj = (v: unknown): Obj =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Obj) : {};

/** The system's fixed-width face, for source text, paths and tool arguments. */
const MONO = "mono";

const column: Style = { display: "flex", flexDirection: "column" };
const row: Style = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
};
/** A flex child that may shrink below its text, so its text wraps (a
 * clipped box has no content minimum) and nothing runs past its edge. */
const shrink: Style = { flexShrink: 1, overflow: "hidden" };
/** Anything a person can press or focus is at least a hit target. */
const target: Style = {
  minWidth: t("metrics.hit"),
  minHeight: t("metrics.hit"),
};
const reading: Style = {
  color: t("colors.text"),
  fontSize: t("typography.reading"),
  lineHeight: t("typography.reading_leading"),
};
const quiet: Style = {
  color: t("colors.secondary"),
  fontSize: t("typography.body"),
  lineHeight: t("typography.reading_leading"),
};
const faint: Style = {
  color: t("colors.muted"),
  fontSize: t("typography.body"),
  lineHeight: t("typography.reading_leading"),
};
const strong: Style = {
  color: t("colors.text"),
  fontSize: t("typography.title"),
  fontWeight: t("typography.semibold_weight"),
};
const chip: Style = {
  fontFamily: MONO,
  fontSize: t("typography.minimum"),
  color: t("colors.secondary"),
  background: t("colors.fill"),
  borderWidth: t("metrics.border"),
  borderColor: t("colors.hairline"),
  borderRadius: t("metrics.inner_radius"),
  paddingLeft: t("metrics.gap"),
  paddingRight: t("metrics.gap"),
  whiteSpace: "nowrap",
  textOverflow: "ellipsis",
  ...shrink,
};
/** A card in the conversation: questions, errors, results. */
const card: Style = {
  ...column,
  gap: t("metrics.text_padding"),
  background: t("colors.panel_secondary"),
  borderRadius: t("metrics.panel_radius"),
  padding: t("metrics.inset"),
};
const hairline: Style = {
  borderBottomWidth: t("metrics.border"),
  borderColor: t("colors.separator"),
};

export function elapsed(ms: number) {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

// ── Prose ────────────────────────────────────────────────────────────────

/** A paragraph of styled runs, wrapped natively as one text. */
function Para({ runs, style }: { runs: Run[]; style?: Style }) {
  const text = runs.map((r) => r.t).join("");
  const styled = runs.some((r) => r.b || r.i || r.c || r.a);
  // Long unbroken tokens (paths, links, JSON) get places to wrap.
  const shown = runs.map((r) => ({ ...r, t: breakable(r.t) }));
  return styled ? (
    <div
      label={text}
      metadata={{ runs: shown as unknown as Json }}
      style={{ ...shrink, ...style }}
    />
  ) : (
    <text style={{ ...shrink, ...style }}>{breakable(text)}</text>
  );
}
/** Markdown as native text: paragraphs, headings, lists, quotes and code. */
function Prose({ text, style = reading }: { text: string; style?: Style }) {
  const blocks = markdown(text);
  return (
    <div style={{ ...column, ...shrink, gap: t("metrics.text_padding") }}>
      {blocks.map((b, i) => (
        <ProseBlock key={i} block={b} style={style} />
      ))}
    </div>
  );
}
function ProseBlock({
  block: b,
  style,
}: {
  key?: string | number;
  block: Block;
  style: Style;
}) {
  switch (b.kind) {
    case "p":
      return <Para runs={b.runs} style={style} />;
    case "h":
      return (
        <Para
          runs={b.runs}
          style={{ ...style, fontWeight: t("typography.semibold_weight") }}
        />
      );
    case "li":
      return (
        <div
          style={{
            ...row,
            alignItems: "start",
            gap: t("metrics.gap"),
            paddingLeft: b.depth ? t("metrics.agent_gap") : t("metrics.gap"),
          }}
        >
          <text style={{ ...style, color: t("colors.muted"), flexShrink: 0 }}>
            {b.marker}
          </text>
          <Para runs={b.runs} style={{ ...style, flexGrow: 1 }} />
        </div>
      );
    case "quote":
      return (
        <div
          style={{
            ...shrink,
            borderLeftWidth: t("metrics.border"),
            borderColor: t("colors.separator"),
            paddingLeft: t("metrics.inset"),
          }}
        >
          <Para
            runs={b.runs}
            style={{ ...style, color: t("colors.secondary") }}
          />
        </div>
      );
    case "code":
      return (
        <div
          style={{
            ...shrink,
            background: t("colors.fill"),
            borderRadius: t("metrics.inner_radius"),
            padding: t("metrics.text_padding"),
          }}
        >
          <text
            style={{
              fontFamily: MONO,
              fontSize: t("typography.minimum"),
              color: t("colors.text"),
              lineHeight: t("typography.dense_leading"),
            }}
          >
            {breakable(b.text)}
          </text>
        </div>
      );
    case "rule":
      return <div style={hairline} />;
  }
}

// ── Header: the thread bar ───────────────────────────────────────────────

function Header({ s, n, send }: { s: Snapshot; n: NativeFacts; send: Send }) {
  const threads = s.threads ?? [];
  const now = Date.now();
  const title = s.threadTitle || "New thread";
  const options = threads.length
    ? threads.map((thread) => ({
        v: thread.id,
        label: thread.title || "New thread",
        detail: thread.busy
          ? "Working…"
          : thread.id === s.threadId
            ? "Current"
            : relativeOpened(thread.updatedAt, now),
      }))
    : [{ v: s.threadId, label: title, detail: "Current" }];
  // Switching while a change is applied would leave it half done.
  const blocked = s.composerDisabled;
  return (
    <div
      id="agent-header"
      role="group"
      label="Agent threads"
      style={{
        ...row,
        flexShrink: 0,
        gap: t("metrics.gap"),
        height: t("metrics.header"),
        paddingLeft: t("metrics.text_padding"),
        paddingRight: t("metrics.text_padding"),
      }}
    >
      <UI.PopUpButton
        id="agent-threads"
        data-pm-variant="inline"
        label={
          blocked
            ? "Finish applying the current change to switch threads"
            : "Switch thread"
        }
        title={
          blocked
            ? "Finish applying the current change to switch threads"
            : "Switch thread"
        }
        value={s.threadId}
        display={title}
        binding={n.bindings.model}
        disabled={blocked}
        options={options}
        onChange={(e) => {
          const id = String(e.value);
          if (id && id !== s.threadId) send({ type: "switchThread", id });
        }}
        style={{ ...shrink, color: t("colors.secondary") }}
      />
      <div style={{ flexGrow: 1 }} />
      <UI.ToolbarItem
        id="agent-new"
        label="New thread"
        title="New thread"
        disabled={blocked}
        metadata={{ symbol: "plus" }}
        style={{ flexShrink: 0, symbolSize: t("metrics.symbol_panel") }}
        onClick={() => {
          if (!blocked) send({ type: "newThread" });
        }}
      />
    </div>
  );
}

// ── Empty state and connection ───────────────────────────────────────────

function Welcome({ s, n, send }: { s: Snapshot; n: NativeFacts; send: Send }) {
  const explanation = n.extensionContext
    ? "Build and improve your extensions."
    : s.settings.access === "editor"
      ? "Your ideas. Editable motion."
      : "Create, research, and build with your project.";
  return (
    <div
      id="agent-welcome"
      role="group"
      label="Make your next move"
      style={{
        ...column,
        flexGrow: 1,
        justifyContent: "center",
        gap: t("metrics.inset"),
      }}
    >
      <div style={{ ...column, gap: t("metrics.gap") }}>
        <text style={{ ...strong, fontSize: t("typography.heading") }}>
          Make your next move
        </text>
        <text style={quiet}>{explanation}</text>
      </div>
      <div style={{ ...column, gap: t("metrics.gap") }}>
        {suggestions(s.settings.access, n.extensionContext).map(
          ([label, prompt]) => (
            <button
              key={label}
              role="button"
              label={label}
              tabIndex={0}
              onClick={() =>
                send({
                  type: "draft",
                  text: prompt,
                  selection: [prompt.length, prompt.length],
                  base: s.draft.text,
                })
              }
              style={{
                ...target,
                ...row,
                height: t("metrics.control"),
                paddingLeft: t("metrics.text_padding"),
                paddingRight: t("metrics.text_padding"),
                background: t("colors.fill"),
                borderRadius: t("metrics.control_radius"),
              }}
            >
              <text style={{ ...quiet, ...shrink, flexGrow: 1 }}>{label}</text>
              <div
                metadata={{ symbol: "chevron.right" }}
                style={{
                  color: t("colors.muted"),
                  symbolSize: t("metrics.symbol_inline"),
                }}
              />
            </button>
          ),
        )}
      </div>
    </div>
  );
}
function Gate({
  s,
  n,
  send,
  compact,
}: {
  s: Snapshot;
  n: NativeFacts;
  send: Send;
  compact: boolean;
}) {
  const provider = s.settings.provider;
  const name = providerName(provider);
  const local = provider === "compatible";
  const checking = n.connection === "checking";
  const waiting = n.connection === "waiting";
  const fault = n.connection === "unavailable" && !n.signedOut;
  const heading = checking
    ? "Checking connection…"
    : waiting
      ? `Finish connecting ${name}`
      : local
        ? "Bring your own model"
        : `Connect ${name}`;
  const body = checking
    ? `Checking your ${name} connection.`
    : n.reason ||
      (local
        ? "Run a model on this Mac with Ollama or LM Studio, or use an API key from OpenRouter, OpenAI, Anthropic, Google and others."
        : `Use your ${name} subscription to power the Powermove agent.`);
  return (
    <div
      id="agent-connect"
      role="group"
      label="Connect your agent"
      style={{
        ...column,
        gap: t("metrics.inset"),
        ...(compact ? {} : { flexGrow: 1, justifyContent: "center" }),
      }}
    >
      <div
        role="group"
        label="Connection provider"
        style={{ ...row, gap: t("metrics.gap") }}
      >
        {s.catalog.providers.map(([id, label]) => (
          <UI.ToolbarItem
            key={id}
            id={`agent-connect-${id}`}
            label={id === "compatible" ? "API / local" : label}
            active={id === provider}
            onClick={() => {
              if (id !== provider) send({ type: "setProvider", provider: id });
            }}
            style={{
              paddingLeft: t("metrics.text_padding"),
              paddingRight: t("metrics.text_padding"),
              borderRadius: t("metrics.control_radius"),
            }}
          >
            {id === "compatible" ? "API / local" : label}
          </UI.ToolbarItem>
        ))}
      </div>
      {fault ? (
        <ErrorNotice
          raw={n.reason || `Powermove couldn’t start its ${name} service.`}
          fallbackTitle={`${name} couldn’t start`}
          retry={() => send({ type: "retryConnection" })}
          id="agent-connect-error"
        />
      ) : (
        <div style={{ ...card }} role="status" label={`${heading}. ${body}`}>
          <text style={strong}>{heading}</text>
          <text style={quiet}>{body}</text>
          <div style={{ ...row }}>
            <UI.PushButton
              id="agent-connect-action"
              role="default"
              label={
                checking
                  ? "Checking…"
                  : waiting
                    ? "Waiting for sign-in…"
                    : local
                      ? "Open Settings"
                      : `Connect ${name}`
              }
              disabled={checking || waiting}
              onClick={() =>
                send(local ? { type: "openSettings" } : { type: "connect" })
              }
            />
          </div>
        </div>
      )}
      {(!local || fault) && (
        <div style={row}>
          <button
            role="button"
            label="Connection settings"
            tabIndex={0}
            onClick={() => send({ type: "openSettings" })}
            style={{
              ...target,
              ...row,
              gap: t("metrics.gap"),
              borderRadius: t("metrics.inner_radius"),
            }}
          >
            <text style={faint}>Connection settings</text>
            <div
              metadata={{ symbol: "chevron.right" }}
              style={{
                color: t("colors.muted"),
                symbolSize: t("metrics.symbol_inline"),
              }}
            />
          </button>
        </div>
      )}
    </div>
  );
}

// ── Errors ───────────────────────────────────────────────────────────────

function ErrorNotice({
  raw,
  retry,
  retryDisabled,
  fallbackTitle,
  id,
}: {
  raw: string;
  retry?: () => void;
  retryDisabled?: boolean;
  fallbackTitle?: string;
  id: string;
}) {
  const [open, setOpen] = useState(false);
  const p = presentError(raw);
  const title =
    fallbackTitle && p.title === "Something needs attention"
      ? fallbackTitle
      : p.title;
  // When nothing was translated the message already is the raw text.
  const details = p.details !== p.message ? p.details : "";
  return (
    <div id={id} role="alert" label={`${title}. ${p.message}`} style={card}>
      <div style={{ ...row, gap: t("metrics.text_padding") }}>
        <div
          metadata={{ symbol: "exclamationmark.circle.fill" }}
          style={{
            color: t("colors.danger"),
            symbolSize: t("metrics.symbol_panel"),
            flexShrink: 0,
          }}
        />
        <text style={{ ...strong, ...shrink }}>{title}</text>
      </div>
      <div
        style={{
          ...column,
          gap: t("metrics.text_padding"),
          paddingLeft: t("metrics.hit"),
        }}
      >
        <text style={{ ...quiet, ...shrink }}>{p.message}</text>
        {(retry || details) && (
          <div style={{ ...row, gap: t("metrics.text_padding") }}>
            {retry && (
              <UI.PushButton
                id={`${id}-retry`}
                label="Try Again"
                disabled={retryDisabled}
                onClick={retryDisabled ? undefined : retry}
              />
            )}
            <div style={{ flexGrow: 1 }} />
            {details && (
              <button
                role="button"
                label={open ? "Hide details" : "Details"}
                aria-expanded={open}
                tabIndex={0}
                onClick={() => setOpen(!open)}
                style={{
                  ...target,
                  ...row,
                  gap: t("metrics.gap"),
                  height: t("metrics.hit"),
                  paddingLeft: t("metrics.text_padding"),
                  paddingRight: t("metrics.gap"),
                  borderRadius: t("metrics.inner_radius"),
                }}
              >
                <text style={faint}>Details</text>
                <div
                  metadata={{ symbol: open ? "chevron.up" : "chevron.down" }}
                  style={{
                    color: t("colors.muted"),
                    symbolSize: t("metrics.symbol_inline"),
                  }}
                />
              </button>
            )}
          </div>
        )}
        {details && open && (
          <div
            style={{
              ...shrink,
              background: t("colors.panel_secondary"),
              borderRadius: t("metrics.inner_radius"),
              padding: t("metrics.text_padding"),
            }}
          >
            <text
              style={{
                fontFamily: MONO,
                fontSize: t("typography.minimum"),
                color: t("colors.secondary"),
                lineHeight: t("typography.dense_leading"),
              }}
            >
              {breakable(details)}
            </text>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Conversation ─────────────────────────────────────────────────────────

interface Local {
  open: Record<string, boolean>;
  toggle: (key: string, value: boolean) => void;
  typed: Record<string, string>;
  type: (key: string, text: string) => void;
}

function UserMessage({
  m,
  answering,
}: {
  key?: string | number;
  m: Message;
  answering: boolean;
}) {
  const text = str(m.value.text);
  const files = Array.isArray(m.value.attachments) ? m.value.attachments : [];
  const shown =
    text ||
    (files.length
      ? `Attached ${files.length} file${files.length === 1 ? "" : "s"}`
      : "");
  return (
    <div
      role="listitem"
      label={`You said: ${shown}`}
      metadata={{ rowId: m.rowId }}
      // The bubble keeps a reply's width of room on its leading side.
      style={{
        ...row,
        justifyContent: "end",
        paddingLeft: t("metrics.section_gap"),
      }}
    >
      <div
        style={{
          ...column,
          ...shrink,
          gap: t("metrics.gap"),
          background: t("colors.field"),
          borderRadius: t("metrics.panel_radius"),
          paddingTop: t("metrics.text_padding"),
          paddingBottom: t("metrics.text_padding"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
          // The request being answered carries the accent, as the original's halo.
          ...(answering
            ? {
                borderWidth: t("metrics.border"),
                borderColor: t("colors.accent"),
              }
            : {}),
        }}
      >
        {files.length > 0 && (
          <div style={{ ...row, flexWrap: "wrap", gap: t("metrics.gap") }}>
            {files.map((f, i) => (
              <text key={i} style={chip}>
                {str(obj(f).name) || "File"}
              </text>
            ))}
          </div>
        )}
        {text && <Prose text={text} />}
      </div>
    </div>
  );
}
const STOPPED = /\b(cancell?ed|stopped)\b/i;
const MEDIA_KIND: Record<string, string> = {
  image: "Image",
  video: "Video",
  audio: "Sound",
};
function AssistantMessage({
  m,
  s,
  send,
}: {
  key?: string | number;
  m: Message;
  s: Snapshot;
  send: Send;
}) {
  const v = m.value;
  const text = str(v.text);
  const working = s.runId !== null;
  // A run the person stopped is not a fault: it reads as a plain line with
  // the retry it still deserves (the original's plain notice).
  if (v.error === true && STOPPED.test(text) && text.length < 200)
    return (
      <div
        role="listitem"
        label={text}
        metadata={{ rowId: m.rowId }}
        style={{ ...column, gap: t("metrics.text_padding") }}
      >
        <text style={{ ...quiet, ...shrink }}>{plain(text)}</text>
        <div style={row}>
          <UI.PushButton
            id={`error-${m.key}-retry`}
            label="Try Again"
            disabled={working}
            onClick={
              working ? undefined : () => send({ type: "retry", id: m.rowId })
            }
          />
        </div>
      </div>
    );
  if (v.error === true)
    return (
      <div
        role="listitem"
        label={text}
        metadata={{ rowId: m.rowId }}
        style={column}
      >
        <ErrorNotice
          id={`error-${m.key}`}
          raw={text}
          retry={() => send({ type: "retry", id: m.rowId })}
          retryDisabled={working}
        />
      </div>
    );
  if (v.nativeResult || v.kind === "result")
    return <Result s={s} send={send} text={text} />;
  const visuals = Array.isArray(v.visualReplies) ? v.visualReplies : [];
  const media = Array.isArray(v.media) ? v.media : [];
  return (
    <div
      role="listitem"
      label={`Agent: ${plain(text)}`}
      metadata={{ rowId: m.rowId }}
      style={{ ...column, gap: t("metrics.text_padding") }}
    >
      {text && <Prose text={text} />}
      {v.requiresProject === true && (
        <div style={row}>
          <UI.PushButton
            label="Continue with project tools"
            disabled={working}
            onClick={
              working
                ? undefined
                : () => {
                    send({ type: "setAccess", access: "auto" });
                    send({ type: "retry", id: m.rowId });
                  }
            }
          />
        </div>
      )}
      {visuals.map((reply) => {
        const r = obj(reply);
        return (
          <div
            key={str(r.id)}
            role="group"
            label={str(r.title)}
            style={{ ...card, ...row, gap: t("metrics.text_padding") }}
          >
            <div
              metadata={{ symbol: "chart.bar.doc.horizontal" }}
              style={{
                color: t("colors.secondary"),
                symbolSize: t("metrics.symbol_action"),
                flexShrink: 0,
              }}
            />
            <div style={{ ...column, ...shrink, flexGrow: 1 }}>
              <text style={strong}>{str(r.title)}</text>
              <text style={faint}>Interactive visualization</text>
            </div>
            <UI.PushButton
              label="Open"
              onClick={() => send({ type: "openVisual", id: str(r.id) })}
            />
          </div>
        );
      })}
      {media.map((item, index) => {
        const it = obj(item);
        const path = str(it.path);
        const remote = it.remote === true;
        return (
          <div
            key={path || index}
            role="group"
            label={str(it.name)}
            style={{ ...card, gap: t("metrics.gap") }}
          >
            <div style={{ ...column, ...shrink }}>
              <text style={{ ...strong, ...shrink }}>{str(it.name)}</text>
              <text style={faint}>
                {(MEDIA_KIND[str(it.kind)] ?? "File") +
                  (remote ? " on the web" : "")}
              </text>
            </div>
            <div style={{ ...row, gap: t("metrics.text_padding") }}>
              <UI.PushButton
                label={remote ? "Open in Browser" : "Open"}
                onClick={() => send({ type: "openMedia", path, reveal: false })}
              />
              {!remote && (
                <UI.PushButton
                  label="Show in Finder"
                  onClick={() =>
                    send({ type: "openMedia", path, reveal: true })
                  }
                />
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
function Result({ s, send, text }: { s: Snapshot; send: Send; text: string }) {
  const ready = s.phase === "result";
  return (
    <div role="listitem" label="Rendered result" style={card}>
      <text style={{ ...faint, fontWeight: t("typography.semibold_weight") }}>
        Rendered result
      </text>
      <text style={strong}>Review the actual result</text>
      <Prose text={text || "The rendered change is ready."} />
      <text style={faint}>
        Powermove source changes from this run are one Command-Z Undo step.
      </text>
      {ready && (
        <div
          style={{
            ...row,
            justifyContent: "end",
            gap: t("metrics.text_padding"),
          }}
        >
          <UI.PushButton
            label="Undo Change"
            onClick={() => send({ type: "undoResult" })}
          />
          <UI.PushButton
            role="default"
            label="Keep Change"
            onClick={() => send({ type: "keepResult" })}
          />
        </div>
      )}
    </div>
  );
}

/** One turn's trail: prose, reasoning, questions and tool groups. */
function Trail({
  m,
  s,
  send,
  local,
}: {
  key?: string | number;
  m: Message;
  s: Snapshot;
  send: Send;
  local: Local;
}) {
  const rows = activity(m.steps);
  const running = s.runId !== null && m.live;
  if (!rows.length && running && s.activity)
    return (
      <div
        role="status"
        label={s.activity}
        style={{ ...row, gap: t("metrics.text_padding") }}
      >
        <div
          metadata={{ symbol: "circle.dotted" }}
          style={{
            color: t("colors.muted"),
            symbolSize: t("metrics.symbol_inline"),
            flexShrink: 0,
          }}
        />
        <text style={{ ...faint, ...shrink }}>{plain(s.activity)}</text>
      </div>
    );
  return (
    <div style={{ ...column, gap: t("metrics.inset") }}>
      {rows.map((r, i) => (
        <Step
          key={r.key}
          r={r}
          s={s}
          send={send}
          local={local}
          live={running && i === rows.length - 1}
        />
      ))}
    </div>
  );
}
function Step({
  r,
  s,
  send,
  local,
  live,
}: {
  key?: string | number;
  r: Activity;
  s: Snapshot;
  send: Send;
  local: Local;
  live: boolean;
}) {
  switch (r.kind) {
    case "text":
      return (
        <div
          role="listitem"
          label={`Agent: ${plain(r.text)}`}
          metadata={{ rowId: r.key }}
        >
          <Prose text={r.text} />
        </div>
      );
    case "thought":
      // Reasoning stays inline, quieter than the reply.
      return (
        <div
          role="listitem"
          label={`Thought: ${plain(r.text)}`}
          metadata={{ rowId: r.key }}
        >
          <Prose text={r.text} style={faint} />
        </div>
      );
    case "question":
      return <Question row={r.row} send={send} local={local} />;
    case "tools":
      return <Tools r={r} s={s} send={send} local={local} live={live} />;
  }
}
const STATUS_WORD: Record<string, string> = {
  error: "Failed",
  stopped: "Stopped",
  waiting: "Waiting",
  continued: "Continued",
};
function Tools({
  r,
  send,
  local,
  live,
}: {
  r: ToolsRow;
  s: Snapshot;
  send: Send;
  local: Local;
  live: boolean;
}) {
  // A live group stays open while the run works; it settles closed once the
  // run moves on. A person's own toggle wins.
  const auto = live || r.details.some((d) => d.task && d.status === "error");
  const open = local.open[r.key] ?? auto;
  const headerless = r.toolCount === 0;
  const details = (
    <div
      role="list"
      label="Tool activity details"
      style={{ ...column, gap: t("metrics.gap") }}
    >
      {r.details.map((d, i) => (
        <ToolLine
          key={`${d.id}-${i}`}
          d={d}
          k={`${r.key}:${d.id}-${i}`}
          send={send}
          local={local}
        />
      ))}
      {r.files.length > 0 && !live && r.status !== "running" && (
        <div
          style={{
            ...row,
            flexWrap: "wrap",
            gap: t("metrics.gap"),
            paddingTop: t("metrics.gap"),
          }}
        >
          {r.files.map((f) => (
            <text key={f} style={chip}>
              {f}
            </text>
          ))}
        </div>
      )}
    </div>
  );
  if (headerless) return details;
  const word = r.status === "error" ? "Failed" : r.meta;
  return (
    <div
      role="group"
      label={r.summary}
      style={{ ...column, gap: t("metrics.gap") }}
    >
      <div style={row}>
        <button
          role="button"
          label={`${r.label}${word ? `, ${word}` : ""}`}
          title={r.tooltip ?? r.summary}
          aria-expanded={open}
          tabIndex={0}
          onClick={() => local.toggle(r.key, !open)}
          style={{
            ...target,
            ...row,
            gap: t("metrics.gap"),
            height: t("metrics.hit"),
            paddingRight: t("metrics.gap"),
            borderRadius: t("metrics.inner_radius"),
          }}
        >
          <div
            metadata={{ symbol: open ? "chevron.down" : "chevron.right" }}
            style={{
              color: t("colors.muted"),
              symbolSize: t("metrics.symbol_inline"),
              flexShrink: 0,
            }}
          />
          <text
            style={{
              ...quiet,
              color: t(
                r.status === "running" ? "colors.text" : "colors.secondary",
              ),
            }}
          >
            {r.status === "running" && r.toolCount ? `${r.label}…` : r.label}
          </text>
          {word && (
            <text
              style={{
                ...faint,
                color: t(
                  r.status === "error" ? "colors.danger" : "colors.muted",
                ),
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {word}
            </text>
          )}
        </button>
      </div>
      {open && details}
    </div>
  );
}
function ToolLine({
  d,
  k,
  send,
  local,
}: {
  key?: string | number;
  d: ToolDetail;
  k: string;
  send: Send;
  local: Local;
}) {
  const expandable = !!d.output;
  const auto =
    (!!d.task && d.status === "error") ||
    (d.kind === "thought" && d.status === "running");
  const open = expandable && (local.open[k] ?? auto);
  const running = d.status === "running" && d.kind === "tool";
  const symbol =
    d.task && d.status !== "running"
      ? d.task.status === "cancelled"
        ? "stop.fill"
        : d.status === "error"
          ? "xmark"
          : "checkmark"
      : running
        ? "circle.dotted"
        : FAMILY_SYMBOL[d.family];
  const word =
    STATUS_WORD[d.status] ??
    (d.task?.status === "cancelled"
      ? "Stopped"
      : d.task?.status === "waiting"
        ? "Waiting"
        : d.span);
  const line = (
    <>
      <div
        metadata={{ symbol: open ? "chevron.down" : symbol }}
        style={{
          color: t(d.status === "error" ? "colors.danger" : "colors.muted"),
          symbolSize: t("metrics.symbol_inline"),
          flexShrink: 0,
        }}
      />
      <text style={{ ...quiet, color: t("colors.text"), flexShrink: 0 }}>
        {d.label}
      </text>
      {d.detail && !(open && d.kind === "thought") && (
        <text
          style={
            d.kind === "thought"
              ? {
                  ...faint,
                  ...shrink,
                  whiteSpace: "nowrap",
                  textOverflow: "ellipsis",
                }
              : chip
          }
        >
          {d.detail}
        </text>
      )}
      <div style={{ flexGrow: 1 }} />
      {word && (
        <text
          style={{
            ...faint,
            color: t(d.status === "error" ? "colors.danger" : "colors.muted"),
            flexShrink: 0,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {word}
        </text>
      )}
    </>
  );
  const lineStyle: Style = {
    ...row,
    gap: t("metrics.gap"),
    minHeight: t("metrics.hit"),
    paddingRight: t("metrics.gap"),
    borderRadius: t("metrics.inner_radius"),
  };
  return (
    <div
      role="listitem"
      label={`${d.label}${d.detail ? ` ${d.detail}` : ""}`}
      style={{ ...column, gap: t("metrics.gap") }}
    >
      <div style={{ ...row, gap: t("metrics.gap") }}>
        {expandable ? (
          <button
            role="button"
            label={d.label}
            aria-expanded={open}
            tabIndex={0}
            onClick={() => local.toggle(k, !open)}
            style={{ ...target, ...lineStyle, ...shrink, flexGrow: 1 }}
          >
            {line}
          </button>
        ) : (
          <div style={{ ...lineStyle, ...shrink, flexGrow: 1 }}>{line}</div>
        )}
        {d.task && d.status === "running" && (
          <UI.PushButton
            label="Stop"
            title={`Stop ${d.label}`}
            onClick={() => send({ type: "stopTask", id: d.rowId })}
          />
        )}
      </div>
      {d.task && d.status === "running" && !open && d.progress && (
        <text style={{ ...faint, ...shrink, paddingLeft: t("metrics.hit") }}>
          {d.progress}
        </text>
      )}
      {open && (
        <div
          style={{
            ...shrink,
            marginLeft: t("metrics.hit"),
            background:
              d.kind === "thought" || d.task ? null : t("colors.fill"),
            borderRadius: t("metrics.inner_radius"),
            padding:
              d.kind === "thought" || d.task ? null : t("metrics.text_padding"),
          }}
        >
          {d.kind === "thought" || d.task ? (
            <Prose text={d.output} style={faint} />
          ) : (
            <text
              style={{
                fontFamily: MONO,
                fontSize: t("typography.minimum"),
                color: t("colors.secondary"),
                lineHeight: t("typography.dense_leading"),
              }}
            >
              {breakable(
                d.output.length > 6000
                  ? `${d.output.slice(0, 6000)}\n…`
                  : d.output,
              )}
            </text>
          )}
        </div>
      )}
    </div>
  );
}

/** A question from the agent, answered in place. Choices and a typed answer
 * combine; nothing is sent until Answer. */
function Question({
  row: r,
  send,
  local,
}: {
  row: Row;
  send: Send;
  local: Local;
}) {
  const v = r.value as Obj;
  const status = str(v.status);
  const open = status === "open";
  const questions = (Array.isArray(v.questions) ? v.questions : []).map(obj);
  const answers = obj(v.answers);
  const typedKey = (q: string) => `${r.id}:${q}`;
  const answered =
    Object.values(r.chosen).some((a) => a.length > 0) ||
    questions.some((q) => (local.typed[typedKey(str(q.id))] ?? "").trim());
  return (
    <div
      role="group"
      label={`Question from the agent: ${plain(str(questions[0]?.question))}`}
      metadata={{ rowId: r.id }}
      style={{
        ...card,
        gap: t("metrics.inset"),
        ...(open ? {} : { background: t("colors.fill") }),
      }}
    >
      {questions.map((q) => {
        const id = str(q.id);
        const options = (Array.isArray(q.options) ? q.options : []).map(obj);
        const chosen = r.chosen[id] ?? [];
        const multiple = q.multiSelect === true;
        return (
          <div
            key={id}
            role="group"
            label={plain(str(q.question))}
            style={{ ...column, gap: t("metrics.text_padding") }}
          >
            {str(q.header) && (
              <text
                style={{
                  ...faint,
                  fontWeight: t("typography.semibold_weight"),
                }}
              >
                {plain(str(q.header))}
              </text>
            )}
            <Prose text={str(q.question)} />
            {open ? (
              <>
                {options.length > 0 && (
                  <div
                    role="radiogroup"
                    label={plain(str(q.question))}
                    style={{ ...column, gap: t("metrics.pill_inset") }}
                  >
                    {options.map((o) => {
                      const label = str(o.label);
                      const on = chosen.includes(label);
                      return (
                        <button
                          key={label}
                          id={`${r.id}-${id}-${label}`}
                          role="button"
                          label={plain(label)}
                          aria-pressed={on}
                          tabIndex={0}
                          onClick={() =>
                            send({
                              type: "chooseAnswer",
                              step: r.commandId,
                              question: id,
                              option: label,
                            })
                          }
                          style={{
                            ...target,
                            ...row,
                            alignItems: "start",
                            gap: t("metrics.text_padding"),
                            minHeight: t("metrics.control"),
                            paddingTop: t("metrics.gap"),
                            paddingBottom: t("metrics.gap"),
                            paddingLeft: t("metrics.text_padding"),
                            paddingRight: t("metrics.text_padding"),
                            borderRadius: t("metrics.control_radius"),
                            ...(on
                              ? { background: t("colors.fill_strong") }
                              : {}),
                          }}
                        >
                          <div
                            metadata={{
                              symbol: multiple
                                ? on
                                  ? "checkmark.square.fill"
                                  : "square"
                                : on
                                  ? "largecircle.fill.circle"
                                  : "circle",
                            }}
                            style={{
                              color: t(
                                on ? "colors.accent_text" : "colors.muted",
                              ),
                              symbolSize: t("metrics.symbol_panel"),
                              flexShrink: 0,
                            }}
                          />
                          <div style={{ ...column, ...shrink, flexGrow: 1 }}>
                            <Para
                              runs={[{ t: plain(label) }]}
                              style={{
                                ...reading,
                                color: t(
                                  on ? "colors.text" : "colors.secondary",
                                ),
                              }}
                            />
                            {str(o.description) && (
                              <text style={{ ...faint, ...shrink }}>
                                {plain(str(o.description))}
                              </text>
                            )}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                )}
                {multiple && options.length > 0 && (
                  <text style={faint}>Choose any that apply</text>
                )}
                {q.allowOther !== false && (
                  <Field
                    id={`${r.id}-${id}-other`}
                    label={`Answer: ${plain(str(q.question))}`}
                    value={local.typed[typedKey(id)] ?? ""}
                    metadata={{
                      placeholder: options.length
                        ? "Or type an answer"
                        : "Type an answer",
                      secret: q.secret === true,
                    }}
                    onInput={(e) => {
                      const text = String(e.value ?? "");
                      local.type(typedKey(id), text);
                      send({
                        type: "typeAnswer",
                        step: r.commandId,
                        question: id,
                        text,
                      });
                    }}
                    onChange={(e) => {
                      const text = String(e.value ?? "");
                      local.type(typedKey(id), text);
                      send({
                        type: "typeAnswer",
                        step: r.commandId,
                        question: id,
                        text,
                      });
                    }}
                    onKeyDown={(e: SurfaceEvent) => {
                      if (
                        e.key === "Enter" &&
                        !e.shiftKey &&
                        !e.composing &&
                        answered
                      )
                        send({
                          type: "answer",
                          step: r.commandId,
                          skip: false,
                        });
                    }}
                  />
                )}
              </>
            ) : status === "answered" && str(answers[id]) ? (
              <div
                role="group"
                label={`Answered: ${str(answers[id])}`}
                style={{ ...row, gap: t("metrics.gap") }}
              >
                <div
                  metadata={{ symbol: "checkmark" }}
                  style={{
                    color: t("colors.muted"),
                    symbolSize: t("metrics.symbol_inline"),
                    flexShrink: 0,
                  }}
                />
                <Para
                  runs={[{ t: str(answers[id]) }]}
                  style={{ ...quiet, ...shrink }}
                />
              </div>
            ) : (
              <text
                role="status"
                label={status === "answered" ? "Skipped" : "Not answered"}
                style={faint}
              >
                {status === "answered" ? "Skipped" : "Not answered"}
              </text>
            )}
          </div>
        );
      })}
      {open && (
        <div
          style={{
            ...row,
            justifyContent: "end",
            gap: t("metrics.text_padding"),
          }}
        >
          {v.transport === "reply" && (
            <UI.PushButton
              id={`${r.id}-skip`}
              label="Skip"
              onClick={() =>
                send({ type: "answer", step: r.commandId, skip: true })
              }
            />
          )}
          <UI.PushButton
            id={`${r.id}-answer`}
            role="default"
            label="Answer"
            disabled={!answered}
            onClick={
              answered
                ? () => send({ type: "answer", step: r.commandId, skip: false })
                : undefined
            }
          />
        </div>
      )}
    </div>
  );
}

function Conversation({
  s,
  send,
  local,
}: {
  s: Snapshot;
  send: Send;
  local: Local;
}) {
  const list = entries(s);
  const running = s.runId !== null;
  // The request being answered: the last prompt of a running turn.
  const answering = running
    ? [...list]
        .reverse()
        .find(
          (e) =>
            e.kind === "message" &&
            e.message.role === "user" &&
            !e.message.steering,
        )?.key
    : undefined;
  const render = (e: Entry): ComponentChildren => {
    if (e.kind === "working")
      return (
        <div
          key={e.key}
          role="status"
          label={e.label}
          style={{ ...hairline, paddingBottom: t("metrics.inset") }}
        >
          <text style={{ ...faint, fontVariantNumeric: "tabular-nums" }}>
            {e.label}
          </text>
        </div>
      );
    if (e.kind === "work") {
      const open = local.open[e.key] ?? false;
      return (
        <div key={e.key} style={{ ...column, gap: t("metrics.agent_gap") }}>
          <button
            role="button"
            label={e.label}
            aria-expanded={open}
            tabIndex={0}
            onClick={() => local.toggle(e.key, !open)}
            style={{
              ...target,
              ...row,
              ...hairline,
              gap: t("metrics.gap"),
              paddingBottom: t("metrics.inset"),
            }}
          >
            <text style={{ ...faint, fontVariantNumeric: "tabular-nums" }}>
              {e.label}
            </text>
            <div
              metadata={{ symbol: open ? "chevron.down" : "chevron.right" }}
              style={{
                color: t("colors.muted"),
                symbolSize: t("metrics.symbol_inline"),
              }}
            />
          </button>
          {open && e.items.map(item)}
        </div>
      );
    }
    return item(e.message);
  };
  const item = (m: Message) =>
    m.role === "user" ? (
      <UserMessage key={m.key} m={m} answering={m.key === answering} />
    ) : m.role === "assistant" ? (
      <AssistantMessage key={m.key} m={m} s={s} send={send} />
    ) : (
      <Trail key={m.key} m={m} s={s} send={send} local={local} />
    );
  return <>{list.map(render)}</>;
}

// ── Composer ─────────────────────────────────────────────────────────────

/** A saved choice the catalog no longer lists stays visible rather than blank. */
function pairs(entries: [string, string][], current: string) {
  const options = entries.map(([v, label]) => ({ v, label }));
  return entries.some(([v]) => v === current)
    ? options
    : [...options, { v: current, label: current }];
}
function Composer({ s, n, send }: { s: Snapshot; n: NativeFacts; send: Send }) {
  const working = s.runId !== null;
  const available = n.connection === "connected";
  const disabled = !available || s.composerDisabled;
  const placeholder = !available
    ? "Connect an agent to start"
    : working
      ? "Add direction…"
      : "Describe a change…";
  const hasDraft = s.draft.text.trim().length > 0 || s.attachments.length > 0;
  const sendLabel = working ? "Send direction" : "Send";
  const slash = s.slash.visible ? s.slash.choices.slice(0, 8) : [];
  const choose = (id: string, value: string | null) => {
    // Shortcuts the host carries out as the composer's own buttons do.
    const clear = () =>
      send({ type: "draft", text: "", selection: [0, 0], base: s.draft.text });
    if (id === "attach") {
      clear();
      send({ type: "attach" });
    } else if (id === "new") {
      clear();
      send({ type: "newThread" });
    } else send({ type: "chooseSlash", id });
    void value;
  };
  return (
    <div
      id="agent-composer"
      role="group"
      label="Message composer"
      state={{ dropTarget: n.dropTarget }}
      style={{
        ...column,
        flexShrink: 0,
        gap: t("metrics.gap"),
        background: t(
          n.dropTarget ? "colors.accent_wash" : "colors.panel_secondary",
        ),
        borderRadius: t("metrics.popover_radius"),
        padding: t("metrics.gap"),
        marginLeft: t("metrics.text_padding"),
        marginRight: t("metrics.text_padding"),
        marginBottom: t("metrics.text_padding"),
      }}
    >
      {slash.length > 0 && (
        <div
          id="agent-shortcuts"
          role="listbox"
          label="Shortcuts"
          style={{
            ...column,
            gap: t("metrics.pill_inset"),
            padding: t("metrics.gap"),
          }}
        >
          {slash.map((c, i) => {
            const copy = n.slashCopy[c.id] ?? slashCopy(c.id, c.value, s);
            const on = i === s.slash.index;
            return (
              <div
                key={c.id}
                role="option"
                label={copy.label}
                aria-selected={on}
                onClick={() => choose(c.id, c.value)}
                style={{
                  ...row,
                  gap: t("metrics.text_padding"),
                  height: t("metrics.control"),
                  paddingLeft: t("metrics.text_padding"),
                  paddingRight: t("metrics.text_padding"),
                  borderRadius: t("metrics.control_radius"),
                  ...(on ? { background: t("colors.fill_strong") } : {}),
                }}
              >
                <text
                  style={{ ...quiet, color: t("colors.text"), flexShrink: 0 }}
                >
                  {copy.label}
                </text>
                <text
                  style={{
                    ...faint,
                    ...shrink,
                    whiteSpace: "nowrap",
                    textOverflow: "ellipsis",
                  }}
                >
                  {copy.description}
                </text>
              </div>
            );
          })}
        </div>
      )}
      <div
        id="agent-field"
        style={{
          ...column,
          gap: t("metrics.gap"),
          background: t("colors.floating"),
          borderRadius: t("metrics.panel_radius"),
          borderWidth: t("metrics.border"),
          borderColor: t("colors.hairline"),
          paddingTop: t("metrics.gap"),
          paddingBottom: t("metrics.gap"),
          paddingLeft: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        }}
      >
        {s.attachments.length > 0 && (
          <div
            style={{
              ...row,
              flexWrap: "wrap",
              gap: t("metrics.gap"),
              paddingLeft: t("metrics.gap"),
              paddingRight: t("metrics.gap"),
            }}
          >
            {s.attachments.map((a) => (
              <div
                key={a.id}
                role="group"
                label={`${a.name}, attachment`}
                metadata={{ armed: s.draft.armed === a.id }}
                style={{
                  ...row,
                  ...shrink,
                  gap: t("metrics.pill_inset"),
                  height: t("metrics.hit"),
                  paddingLeft: t("metrics.gap"),
                  background: t(
                    s.draft.armed === a.id
                      ? "colors.selected_fill"
                      : "colors.fill",
                  ),
                  borderRadius: t("metrics.inner_radius"),
                }}
              >
                <button
                  role="button"
                  label={`${a.name}, attachment`}
                  title={a.name}
                  tabIndex={0}
                  onClick={() => send({ type: "openAttachment", id: a.id })}
                  style={{
                    ...target,
                    ...row,
                    ...shrink,
                    gap: t("metrics.gap"),
                  }}
                >
                  <div
                    metadata={{ symbol: "doc" }}
                    style={{
                      color: t("colors.muted"),
                      symbolSize: t("metrics.symbol_inline"),
                      flexShrink: 0,
                    }}
                  />
                  <text
                    style={{
                      ...quiet,
                      ...shrink,
                      whiteSpace: "nowrap",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {a.name}
                  </text>
                </button>
                <UI.ToolbarItem
                  label={`Remove ${a.name}`}
                  title={`Remove ${a.name}`}
                  metadata={{ symbol: "xmark" }}
                  style={{
                    symbolSize: t("metrics.symbol_inline"),
                    flexShrink: 0,
                  }}
                  onClick={() => send({ type: "removeAttachment", id: a.id })}
                />
              </div>
            ))}
          </div>
        )}
        {n.dropTarget ? (
          <text
            style={{
              ...quiet,
              color: t("colors.accent_text"),
              paddingLeft: t("metrics.text_padding"),
            }}
          >
            Drop to attach
          </text>
        ) : (
          <Field
            multiline
            id="agent-composer-text"
            label="Message Powermove agent"
            title="Return to send · Shift-Return for a new line · / for shortcuts"
            value={s.draft.text}
            disabled={disabled}
            metadata={{ placeholder, submitOnReturn: true }}
            state={{ keyboardFocus: n.keyboardFocus === "composer" }}
            tabIndex={0}
            onInput={(event) => {
              const text = String(event.value ?? "");
              send({
                type: "draft",
                text,
                selection: [text.length, text.length],
                base: s.draft.text,
              });
              // The shortcuts menu answers to the composer it belongs to.
              if (text.startsWith("/") && !s.slash.visible)
                send({ type: "focus", focused: true });
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
              background: t("colors.floating"),
              minWidth: t("metrics.hit"),
            }}
          />
        )}
        <div
          style={{
            ...row,
            gap: t("metrics.gap"),
            height: t("metrics.control"),
          }}
        >
          <UI.ToolbarItem
            id="agent-attach"
            label="Add attachments"
            title="Add images, video, audio or documents"
            metadata={{ symbol: "plus" }}
            disabled={disabled || s.attachments.length >= 6}
            style={{ flexShrink: 0, symbolSize: t("metrics.symbol_panel") }}
            onClick={() => send({ type: "attach" })}
          />
          <div style={{ flexGrow: 1 }} />
          {working && (
            <button
              id="agent-stop"
              role="button"
              label="Stop current run"
              title="Stop current run"
              tabIndex={0}
              disabled={s.stopping}
              metadata={{ symbol: "stop.fill" }}
              onClick={() => {
                if (!s.stopping) send({ type: "stop" });
              }}
              style={{
                ...target,
                ...row,
                justifyContent: "center",
                flexShrink: 0,
                width: t("metrics.control"),
                height: t("metrics.control"),
                borderRadius: t("metrics.control_radius"),
                background: t("colors.text"),
                color: t("colors.floating"),
                symbolSize: t("metrics.symbol_inline"),
              }}
            />
          )}
          {(!working || hasDraft) && (
            <button
              id="agent-send"
              role="button"
              label={sendLabel}
              title={sendLabel}
              tabIndex={0}
              disabled={disabled || !s.sendable}
              metadata={{ symbol: "return" }}
              onClick={() => {
                if (!disabled && s.sendable) send({ type: "submit" });
              }}
              style={{
                ...target,
                ...row,
                justifyContent: "center",
                flexShrink: 0,
                width: t("metrics.control"),
                height: t("metrics.control"),
                borderRadius: t("metrics.control_radius"),
                symbolSize: t("metrics.symbol_panel"),
                background: t(
                  !disabled && s.sendable ? "colors.accent" : "colors.fill",
                ),
                color: t(
                  !disabled && s.sendable
                    ? "colors.selected_text"
                    : "colors.muted",
                ),
              }}
            />
          )}
        </div>
      </div>
      <div
        id="agent-options"
        title="Model and reasoning effort"
        style={{
          ...row,
          gap: t("metrics.gap"),
          height: t("metrics.control"),
          paddingLeft: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        }}
      >
        <UI.PopUpButton
          data-pm-variant="inline"
          id="agent-provider"
          label="Provider"
          value={s.settings.provider}
          binding={n.bindings.model}
          disabled={working || s.composerDisabled}
          onChange={(e) =>
            send({ type: "setProvider", provider: String(e.value) })
          }
          options={pairs(s.catalog.providers, s.settings.provider)}
          style={{ ...shrink, color: t("colors.secondary") }}
        />
        <UI.PopUpButton
          data-pm-variant="inline"
          id="agent-model"
          label="Model"
          value={s.settings.model}
          binding={n.bindings.model}
          disabled={working || s.composerDisabled}
          onChange={(e) =>
            send({
              type: "setModel",
              model: String(e.value),
              effort: s.settings.effort,
            })
          }
          options={pairs(s.catalog.models, s.settings.model)}
          style={{ ...shrink, color: t("colors.secondary") }}
        />
        {s.catalog.efforts.length > 1 && (
          <UI.PopUpButton
            data-pm-variant="inline"
            id="agent-effort"
            label="Reasoning effort"
            value={s.settings.effort}
            binding={n.bindings.model}
            disabled={working || s.composerDisabled}
            onChange={(e) =>
              send({
                type: "setModel",
                model: s.settings.model,
                effort: String(e.value),
              })
            }
            options={s.catalog.efforts.map((v) => ({
              v,
              label: effortLabel(v),
            }))}
            style={{ ...shrink, color: t("colors.muted") }}
          />
        )}
        <div style={{ flexGrow: 1 }} />
        <UI.PopUpButton
          data-pm-variant="inline"
          id="agent-access"
          label={`Agent access: ${ACCESS[s.settings.access].label}`}
          title={ACCESS[s.settings.access].detail}
          value={s.settings.access}
          binding={n.bindings.access}
          disabled={working || s.composerDisabled}
          onChange={(e) =>
            send({ type: "setAccess", access: e.value as Access })
          }
          options={(Object.keys(ACCESS) as Access[]).map((v) => ({
            v,
            label: ACCESS[v].label,
            detail: ACCESS[v].detail,
            // Full access asks for consent the native host can't show yet.
            ...(v === "full" && s.settings.access !== "full"
              ? { disabled: true }
              : {}),
          }))}
          style={{
            flexShrink: 0,
            color: t(
              s.settings.access === "full"
                ? "colors.warning"
                : "colors.secondary",
            ),
          }}
        />
      </div>
    </div>
  );
}

// ── The panel ────────────────────────────────────────────────────────────

export function AgentView({
  publication: { snapshot: s, native: n },
  send,
}: {
  publication: Publication;
  send: Send;
}) {
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [away, setAway] = useState(false);
  const [jumps, setJumps] = useState(0);
  // "Working for" ticks between the model's publications, from the last
  // elapsed time it published.
  const [tick, setTick] = useState({
    at: Date.now(),
    now: Date.now(),
    ms: s.elapsedMs,
  });
  if (tick.ms !== s.elapsedMs)
    setTick({ at: Date.now(), now: Date.now(), ms: s.elapsedMs });
  useEffect(() => {
    if (s.runId === null) return;
    let alive = true;
    const step = () => {
      if (!alive) return;
      setTick((t) => ({ ...t, now: Date.now() }));
      setTimeout(step, 1000);
    };
    const first = setTimeout(step, 1000);
    return () => {
      alive = false;
      clearTimeout(first);
    };
  }, [s.runId]);
  if (!n.visible) return null;
  const local: Local = {
    open,
    toggle: (key, value) => setOpen({ ...open, [key]: value }),
    typed,
    type: (key, text) => setTyped({ ...typed, [key]: text }),
  };
  const working = s.runId !== null;
  const connected = n.connection === "connected";
  const empty = s.rows.count === 0 && !working;
  const gate = !connected && !working;
  // Nothing to keep yet: the connection is the whole panel.
  const setup = gate && empty && !s.draft.text && !s.attachments.length;
  const popover = n.presentation === "popover";
  return (
    <div
      id="agent-panel"
      role="group"
      label="Powermove agent"
      metadata={{ presentation: n.presentation }}
      style={{
        ...column,
        flexGrow: 1,
        ...(popover ? { width: t("metrics.agent_width") } : {}),
        background: t(popover ? "colors.floating" : "colors.panel"),
      }}
    >
      <Header s={s} n={n} send={send} />
      {s.saveError && (
        <div
          role="status"
          label="Thread history couldn’t be saved. Keep this window open."
          style={{
            ...row,
            gap: t("metrics.gap"),
            paddingLeft: t("metrics.inset"),
            paddingRight: t("metrics.inset"),
            paddingBottom: t("metrics.gap"),
          }}
        >
          <div
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
            style={{
              color: t("colors.warning"),
              symbolSize: t("metrics.symbol_inline"),
              flexShrink: 0,
            }}
          />
          <text style={{ ...quiet, ...shrink, color: t("colors.warning") }}>
            Thread history couldn’t be saved. Keep this window open.
          </text>
        </div>
      )}
      <div
        id="agent-conversation"
        role="group"
        label={setup ? "Agent setup" : "Agent conversation"}
        tabIndex={0}
        data-pm-follow="end"
        data-pm-follow-token={`${s.threadId}:${jumps}`}
        onScroll={(e) => setAway(obj(e.value).away === true)}
        metadata={{
          count: s.rows.count,
          range: s.rows.range as unknown as Json,
        }}
        style={{
          ...column,
          ...target,
          flexGrow: 1,
          overflow: "scroll",
          gap: t("metrics.agent_gap"),
          paddingTop: t("metrics.gap"),
          paddingBottom: t("metrics.inset"),
          paddingLeft: t("metrics.agent_gap"),
          paddingRight: t("metrics.agent_gap"),
        }}
      >
        {gate && <Gate s={s} n={n} send={send} compact={!setup} />}
        {!setup &&
          (n.loadingHistory ? (
            <UI.ProgressRow label="Loading earlier messages…" />
          ) : null)}
        {!setup &&
          (empty ? (
            <Welcome s={s} n={n} send={send} />
          ) : (
            <Conversation
              s={
                s.runId === null || s.elapsedMs === null
                  ? s
                  : {
                      ...s,
                      elapsedMs: s.elapsedMs + Math.max(0, tick.now - tick.at),
                    }
              }
              send={send}
              local={local}
            />
          ))}
      </div>
      {s.stopError && (
        <div
          role="status"
          label="Couldn’t stop this task."
          style={{
            ...row,
            gap: t("metrics.gap"),
            paddingLeft: t("metrics.agent_gap"),
            paddingRight: t("metrics.agent_gap"),
            paddingBottom: t("metrics.gap"),
          }}
        >
          <div
            metadata={{ symbol: "exclamationmark.triangle.fill" }}
            style={{
              color: t("colors.danger"),
              symbolSize: t("metrics.symbol_inline"),
              flexShrink: 0,
            }}
          />
          <text style={{ ...quiet, ...shrink, color: t("colors.danger") }}>
            Couldn’t stop this task.
          </text>
        </div>
      )}
      {!setup && away && (
        <div
          style={{
            ...row,
            justifyContent: "center",
            paddingBottom: t("metrics.gap"),
          }}
        >
          <UI.ToolbarItem
            id="agent-jump"
            label="Scroll to latest"
            title="Scroll to latest"
            metadata={{ symbol: "chevron.down" }}
            onClick={() => {
              setAway(false);
              setJumps(jumps + 1);
            }}
            style={{
              background: t("colors.fill"),
              borderRadius: t("metrics.popover_radius"),
              symbolSize: t("metrics.symbol_inline"),
            }}
          />
        </div>
      )}
      {!setup && <Composer s={s} n={n} send={send} />}
    </div>
  );
}
/** Mounted extension; onError is required so rejected work is never silently lost. */
export function AgentPanel({
  model,
  onError,
}: {
  model: AgentModel;
  onError: (error: unknown) => void;
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
