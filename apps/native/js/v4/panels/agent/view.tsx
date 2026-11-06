import type { ComponentChildren } from "preact";
import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import type {
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
const copy = {
  editor: "Composition only",
  supervised: "Supervised",
  auto: "Auto",
  full: "Full access",
} as const;
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
  label,
  danger,
  onClick,
}: {
  label: string;
  danger?: boolean;
  onClick?: () => void;
}) {
  return (
    <UI.PushButton
      label={label}
      role={danger ? "destructive" : "normal"}
      disabled={!onClick}
      onClick={onClick}
    />
  );
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
      }}
      state={{ keyboardFocus: n.keyboardFocus === "conversation" }}
      style={{
        ...vertical,
        minWidth: t("metrics.hit"),
        minHeight: t("metrics.hit"),
      }}
    >
      {s.rows.items.map((row) => (
        <Message key={row.id} row={row} n={n} send={send} />
      ))}
    </UI.List>
  );
}
const str = (value: unknown) => (typeof value === "string" ? value : "");
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
        style={{ display: "flex", flexDirection: "row", justifyContent: "end" }}
      >
        <div
          style={{
            ...reading,
            background: t("colors.field"),
            borderRadius: t("metrics.panel_radius"),
            paddingLeft: t("metrics.inset"),
            paddingRight: t("metrics.inset"),
          }}
        >
          <text>{text}</text>
        </div>
      </div>
    );
  if (kind === "thought")
    return (
      <UI.DisclosureGroup
        id={row.id}
        label={
          v.live === true
            ? "Thinking…"
            : typeof v.endedAt === "number" && typeof v.startedAt === "number"
              ? `Thought for ${Math.max(1, Math.round((v.endedAt - v.startedAt) / 1000))} s`
              : "Thought"
        }
        open={row.open}
        onChange={(open) => send({ type: "disclose", id: row.commandId, open })}
        style={muted}
      >
        <text style={body}>{text}</text>
      </UI.DisclosureGroup>
    );
  if (kind === "question")
    return (
      <div
        role="listitem"
        label={`Question from the agent: ${questionTitle}`}
        metadata={{ rowId: row.id, apiGap: "pushButton" }}
        style={card(n.presentation === "popover")}
      >
        <text style={title}>{questionTitle}</text>
        {questions.map((q) =>
          q && typeof q === "object" && !Array.isArray(q) ? (
            <Placeholder gap="pushButton" label={str(q.question)}>
              <text style={body}>{str(q.question)}</text>
            </Placeholder>
          ) : null,
        )}
        {v.transport === "reply" && <Push label="Skip" />}
        <Push label="Answer" />
      </div>
    );
  if (kind === "approval")
    return (
      <div
        role="listitem"
        label={text}
        metadata={{ rowId: row.id, apiGap: "pushButton" }}
        style={card(n.presentation === "popover")}
      >
        <text style={title}>{text}</text>
        <Push label="Allow" />
        <Push label="Allow for This Thread" />
        <Push label="Deny" danger />
      </div>
    );
  if (kind === "visual")
    return (
      <Placeholder gap="preview" label={str(v.title)}>
        {v.status === "error" && (
          <div metadata={{ symbol: "exclamationmark.triangle.fill" }} />
        )}
        <text style={title}>{str(v.title)}</text>
        <text
          style={
            v.status === "error"
              ? { ...body, color: t("colors.danger") }
              : muted
          }
        >
          {v.status === "error"
            ? "The visualization couldn’t load."
            : "Loading visualization…"}
        </text>
        <UI.ToolbarItem label="Replay" disabled />
        <UI.ToolbarItem label="Expand" disabled />
      </Placeholder>
    );
  if (kind === "error" || v.error === true)
    return (
      <div
        role="listitem"
        label={text}
        metadata={{ rowId: row.id, symbol: "exclamationmark.triangle.fill" }}
      >
        <text style={{ ...body, color: t("colors.text") }}>{text}</text>
        <Push label="Try Again" />
      </div>
    );
  if (kind === "tool" || kind === "task") {
    const label = str(v.title) || str(v.label) || str(v.toolName) || text;
    const status = str(v.status);
    const state = (
      {
        error: "Failed",
        stopped: "Stopped",
        waiting: "Waiting",
        continued: "Continued",
      } as Record<string, string>
    )[status];
    return (
      <UI.DisclosureGroup
        id={row.id}
        label={label}
        open={row.open}
        onChange={(open) => send({ type: "disclose", id: row.commandId, open })}
        state={{ hover: n.hoverRow === row.id }}
        style={{
          color: t("colors.secondary"),
          fontSize: t("typography.body"),
          ...(n.hoverRow === row.id ? { background: t("colors.hover") } : {}),
        }}
      >
        <text style={body}>{str(v.output)}</text>
        {state && (
          <text
            style={
              status === "error"
                ? { ...body, color: t("colors.danger") }
                : muted
            }
            metadata={
              status === "error" ? { symbol: "xmark.circle.fill" } : undefined
            }
          >
            {state}
          </text>
        )}
        {status === "running" && (
          <UI.ToolbarItem
            label={`Stop ${label}`}
            onClick={() => send({ type: "stopTask", id: row.commandId })}
          >
            Stop
          </UI.ToolbarItem>
        )}
      </UI.DisclosureGroup>
    );
  }
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
export function AgentView({
  publication: { snapshot: s, native: n },
  send,
}: {
  publication: Publication;
  send: (action: Action) => void;
}) {
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
        ...(n.presentation === "popover"
          ? { width: t("metrics.agent_width") }
          : {}),
        background: t(
          n.presentation === "popover" ? "colors.floating" : "colors.panel",
        ),
        borderRadius: t("metrics.panel_radius"),
      }}
    >
      <div
        id="agent-header"
        role="group"
        label="Agent"
        title={
          n.presentation === "popover"
            ? "Drag to move or pin to workspace"
            : undefined
        }
        metadata={{ apiGap: "shell" }}
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          height: t("metrics.header"),
          paddingLeft: t("metrics.inset"),
        }}
      >
        <text style={title}>Agent</text>
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
          }}
        >
          <UI.ToolbarItem
            id="agent-threads"
            label={
              s.threadTitle
                ? `Switch thread, ${s.threadTitle}`
                : "Switch thread"
            }
            title={
              s.threadTitle
                ? `Switch thread, ${s.threadTitle}`
                : "Switch thread"
            }
            disabled
            metadata={{ apiGap: "threadPicker" }}
          />
          <UI.ToolbarItem
            disabled={s.composerDisabled}
            id="agent-new"
            label="New Thread"
            title="New Thread"
            onClick={() => {
              if (!s.composerDisabled) send({ type: "newThread" });
            }}
          />
          <UI.ToolbarItem
            id="agent-presentation"
            label={n.presentation === "popover" ? "Pin to Workspace" : "Close"}
            title={n.presentation === "popover" ? "Pin to Workspace" : "Close"}
            onClick={() =>
              send({
                type: "shell",
                action: n.presentation === "popover" ? "pin" : "close",
              })
            }
          />
        </div>
      </div>
      <div
        style={{
          ...vertical,
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
        {n.connection === "checking" ? (
          <UI.ProgressRow label="Checking connection…" />
        ) : n.connection === "waiting" ? (
          <>
            <UI.ProgressRow label="Waiting for sign-in…" />
            <Push
              label="Open Settings"
              onClick={() => send({ type: "openSettings" })}
            />
          </>
        ) : n.connection === "unavailable" ? (
          <UI.EmptyState
            label="Connect Your Agent"
            explanation={n.reason}
            action={
              <>
                <Push label="Try Again" />
                <Push
                  label="Connection Settings…"
                  onClick={() => send({ type: "openSettings" })}
                />
              </>
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
        state={{
          keyboardFocus: n.keyboardFocus === "composer",
          dropTarget: n.dropTarget,
        }}
        title="Return to send · Shift-Return for a new line · / for shortcuts"
        tabIndex={0}
        onKeyDown={(event) => {
          event.stopPropagation();
          send(composerKey(event));
        }}
        style={{
          ...vertical,
          background: t(n.dropTarget ? "colors.accent_wash" : "colors.field"),
          borderRadius: t("metrics.control_radius"),
          minHeight: t("metrics.hit"),
          minWidth: t("metrics.hit"),
          margin: t("metrics.inset"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
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
        <Placeholder gap="composer" label="Message Powermove agent">
          <text
            style={
              n.dropTarget
                ? { ...body, color: t("colors.accent_text") }
                : s.draft.text
                  ? reading
                  : muted
            }
          >
            {n.dropTarget ? "Drop to attach" : s.draft.text || placeholder}
          </text>
        </Placeholder>
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            height: t("metrics.hit"),
          }}
        >
          <UI.ToolbarItem
            id="agent-attach"
            label="Add Attachments"
            title="Add Attachments"
            disabled
            metadata={{ apiGap: "composer" }}
          />
          <UI.PopUpButton
            id="agent-access"
            label={`Agent access, ${copy[s.settings.access]}`}
            value={s.settings.access}
            binding={n.bindings.access}
            disabled
            metadata={{ apiGap: "accessMenu" }}
            options={Object.entries(copy).map(([v, label]) => ({ v, label }))}
          />
          <UI.ToolbarItem
            id="agent-model"
            label={`Model and reasoning effort, ${s.settings.model}, ${s.settings.effort}`}
            disabled
            metadata={{ apiGap: "modelPicker" }}
          >{`${s.settings.model} · ${s.settings.effort}`}</UI.ToolbarItem>
          {working ? (
            <UI.ToolbarItem
              id="agent-stop"
              label="Stop current run"
              title="Stop Current Run"
              disabled={s.stopping}
              onClick={() => {
                if (!s.stopping) send({ type: "stop" });
              }}
            />
          ) : (
            <UI.ToolbarItem
              id="agent-send"
              label="Send"
              disabled={disabled || !s.sendable}
              onClick={() => {
                if (!disabled && s.sendable) send({ type: "submit" });
              }}
            />
          )}
        </div>
        {s.slash.visible && (
          <Placeholder gap="composer" label="Shortcuts">
            {s.slash.choices.slice(0, 6).map((c) =>
              n.slashCopy[c.id] ? (
                <UI.ToolbarItem
                  key={c.id}
                  label={n.slashCopy[c.id].label}
                  onClick={() => send({ type: "chooseSlash", id: c.id })}
                >
                  {n.slashCopy[c.id].description}
                </UI.ToolbarItem>
              ) : null,
            )}
          </Placeholder>
        )}
      </div>
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
