/**
 * The Agent panel, natively. It follows the original app’s panel for
 * state, copy and keyboard (thread bar, conversation with folded work, tool
 * groups, questions, errors with Try Again, composer with its run options)
 * and draws it with the native kit and pm-design tokens.
 */
import { Component, createContext, type ComponentChildren } from "preact";
import { useContext, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import { citationLabel, quoteSelection, type Citation } from "./citations";
import * as UI from "../../src/components";
import { json, type Json, type SurfaceEvent } from "../../src/protocol";
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
  slashCopy,
  str,
  suggestions,
  threadName,
  nestedThreads,
  threadPath,
  type Activity,
  type Block,
  type Entry,
  type Message,
  type Run,
  type ToolDetail,
  type ToolsRow,
  modelDisplayName,
} from "./present";

/** Pure view memoization using native Preact. Importing preact/compat installs
 * DOM event normalization globally, which changes native field event plans. */
function memo<P extends object>(
  view: (props: P) => ComponentChildren,
  equal: (a: P, b: P) => boolean = (a, b) => {
    const keys = Object.keys(a) as (keyof P)[];
    return (
      keys.length === Object.keys(b).length && keys.every((k) => a[k] === b[k])
    );
  },
) {
  return class extends Component<P> {
    shouldComponentUpdate(next: P) {
      return !equal(this.props, next);
    }
    render() {
      return view(this.props);
    }
  };
}

const t = UI.token;
/** The composer and answer fields hold the model's draft (sent as actions),
 * not a host-bound value, so they take no binding. */
const Field = UI.TextField as (
  props: Omit<Parameters<typeof UI.TextField>[0], "binding"> & {
    binding?: string;
  },
) => ReturnType<typeof UI.TextField>;
type Send = (action: Action) => unknown;
type Style = Record<string, string | number | null | { token: string }>;
type Obj = Record<string, Json>;
const obj = (v: unknown): Obj =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Obj) : {};

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
/** A hairline chip in the code face (`data-pm-variant="code"`), cut in the
 * middle so a path keeps its start and its file name. The box is a `div`:
 * a text leaf draws no fill or edge. */
const chipBox: Style = {
  ...row,
  background: t("colors.fill"),
  borderWidth: t("metrics.border"),
  borderColor: t("colors.hairline"),
  borderRadius: t("metrics.inner_radius"),
  paddingLeft: t("metrics.gap"),
  paddingRight: t("metrics.gap"),
  ...shrink,
};
const chipText: Style = {
  fontSize: t("typography.minimum"),
  color: t("colors.secondary"),
  whiteSpace: "nowrap",
  textOverflow: "middle",
  ...shrink,
};
function Chip({ children }: { children: string; key?: string | number }) {
  return (
    <div style={chipBox}>
      <text data-pm-variant="code" style={chipText}>
        {children}
      </text>
    </div>
  );
}
const citationChip: Style = {
  fontSize: t("typography.minimum"),
  color: t("colors.secondary"),
  background: t("colors.fill"),
  borderWidth: t("metrics.border"),
  borderColor: t("colors.hairline"),
  borderRadius: t("metrics.inner_radius"),
  paddingLeft: t("metrics.gap"),
  paddingRight: t("metrics.gap"),
  whiteSpace: "nowrap",
  textOverflow: "middle",
  ...shrink,
};
export function attachmentKind(a: Record<string, unknown>) {
  const name = String(a.name ?? "").toLowerCase();
  const type = String(a.type ?? "");
  if (a.screenPick || /^Frame .*selection\.png$/i.test(name)) return "pick";
  if (type.startsWith("image/") || /\.(png|jpe?g|heic|webp|gif)$/.test(name))
    return "image";
  if (type.startsWith("video/") || /\.(mov|mp4|webm|m4v)$/.test(name))
    return "video";
  if (type.startsWith("audio/") || /\.(mp3|wav|m4a|aiff|flac)$/.test(name))
    return "audio";
  return "document";
}
export function AttachmentTile({
  a,
  armed = false,
  small = false,
  send,
}: {
  key?: string | number;
  a: Record<string, any>;
  armed?: boolean;
  small?: boolean;
  send: Send;
}) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const focusEpoch = useRef(0);
  const focus = () => {
    focusEpoch.current++;
    setFocused(true);
  };
  const blur = () => {
    const epoch = ++focusEpoch.current;
    // Let focus move from the tile to its remove control before hiding it.
    setTimeout(() => {
      if (focusEpoch.current === epoch) setFocused(false);
    }, 0);
  };
  const kind = attachmentKind(a);
  const visual = ["image", "video", "pick"].includes(kind);
  const pill = kind === "pick" && !small;
  const side = t(
    small || pill ? "metrics.agent_attachment_sent" : "metrics.agent_attachment",
  );
  const [title, ...details] = pill
    ? String(a.caption || a.name).split(" · ")
    : [];
  const thumbnailRadius = t(
    pill ? "metrics.media_small_radius" : "metrics.inner_radius",
  );
  const symbol =
    kind === "pick"
      ? "rectangle.dashed"
      : kind === "image"
        ? "photo"
        : kind === "video"
          ? "film"
          : kind === "audio"
            ? "waveform"
            : /\.(js|ts|tsx|py|rs|json|html|css)$/i.test(a.name)
              ? "chevron.left.forwardslash.chevron.right"
              : /\.(csv|xlsx?)$/i.test(a.name)
                ? "tablecells"
                : /\.(pdf|rtf|docx?)$/i.test(a.name)
                  ? "doc.richtext"
                  : "doc.text";
  return (
    <div
      role="group"
      label={`${a.name}, attachment`}
      metadata={{ armed, kind }}
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
      style={{ ...column, flexShrink: 1, position: "relative" }}
    >
      <UI.ToolbarItem
        role="button"
        label={`Open ${a.name}`}
        title={a.name}
        tabIndex={0}
        onClick={() => send({ type: "openAttachment", id: a.id })}
        onFocus={focus}
        onBlur={blur}
        style={{
          ...row,
          ...target,
          gap: t("metrics.gap"),
          width: pill ? null : visual ? side : t("metrics.thumbnail_min"),
          height: pill ? null : side,
          background: t(armed ? "colors.selected_fill" : "colors.fill"),
          borderRadius: t("metrics.inner_radius"),
          position: "relative",
          overflow: "hidden",
          padding: pill
            ? t("metrics.pill_inset")
            : visual
              ? null
              : t("metrics.gap"),
          ...(pill && {
            borderWidth: t("metrics.border"),
            borderColor: t("colors.hairline"),
            paddingRight: t("metrics.gap"),
          }),
        }}
      >
        <div
          metadata={a.thumbnail ? undefined : { symbol }}
          style={{
            ...row,
            justifyContent: "center",
            width: visual ? side : t("metrics.hit"),
            height: visual ? side : t("metrics.hit"),
            flexShrink: 0,
            color: t("colors.secondary"),
            symbolSize: t("metrics.symbol_panel"),
            position: "relative",
            overflow: "hidden",
            borderRadius: thumbnailRadius,
          }}
        >
          {a.thumbnail && (
            <img
              resource={a.thumbnail}
              alt={a.name}
              metadata={{ pmKit: { presence: true } }}
              style={{
                position: "absolute",
                inset: "fill",
                borderRadius: thumbnailRadius,
              }}
            />
          )}
          {a.thumbnail && armed && (
            <div
              style={{
                position: "absolute",
                inset: "fill",
                borderRadius: thumbnailRadius,
                background: t("colors.selected_fill"),
                opacity: t("metrics.drag_opacity"),
              }}
            />
          )}
          {kind === "pick" && (
            <div
              metadata={{ symbol: "rectangle.dashed" }}
              style={{
                position: "absolute",
                right: t("metrics.pill_inset"),
                bottom: t("metrics.pill_inset"),
                color: t("colors.text"),
                background: t("colors.floating"),
                borderRadius: t("metrics.pill_inset"),
                symbolSize: t("metrics.symbol_inline"),
                width: t("metrics.symbol_inline"),
                height: t("metrics.symbol_inline"),
              }}
            />
          )}
        </div>
        {pill && (
          <>
            <div
              style={{ ...column, ...shrink, maxWidth: t("metrics.thumbnail_min") }}
            >
              <text
                style={{
                  ...quiet,
                  color: t("colors.text"),
                  whiteSpace: "nowrap",
                  textOverflow: "ellipsis",
                  ...shrink,
                }}
              >
                {title}
              </text>
              <text
                style={{
                  ...faint,
                  fontSize: t("typography.minimum"),
                  whiteSpace: "nowrap",
                  textOverflow: "ellipsis",
                  ...shrink,
                }}
              >
                {details.join(" · ") || "Region"}
              </text>
            </div>
            {/* Keep the pill the same width as the remove button comes and goes. */}
            <div style={{ width: t("metrics.hit"), flexShrink: 0 }} />
          </>
        )}
        {!visual && (
          <div style={{ ...column, ...shrink }}>
            <text
              style={{
                ...quiet,
                color: t("colors.text"),
                whiteSpace: "nowrap",
                textOverflow: "middle",
                ...shrink,
              }}
            >
              {a.name}
            </text>
            <text style={faint}>
              {a.duration ||
                (kind === "audio"
                  ? "Audio"
                  : String(a.name).split(".").at(-1)?.toUpperCase() || "File")}
            </text>
          </div>
        )}
        {kind === "video" && (
          <text
            style={{
              ...faint,
              fontSize: t("typography.minimum"),
              position: "absolute",
              bottom: t("metrics.pill_inset"),
              right: t("metrics.pill_inset"),
              background: t("colors.floating"),
              borderRadius: t("radius.xs"),
            }}
          >
            {a.duration || "Video"}
          </text>
        )}
      </UI.ToolbarItem>
      {!small && (hovered || focused || armed) && (
        <UI.ToolbarItem
          label={`Remove ${a.name}`}
          title={`Remove ${a.name}`}
          onFocus={focus}
          onBlur={blur}
          metadata={{ symbol: "xmark.circle.fill" }}
          style={{
            position: "absolute",
            right: t(pill ? "metrics.gap" : "metrics.pill_inset"),
            top: t("metrics.pill_inset"),
            ...(pill
              ? { bottom: t("metrics.pill_inset") }
              : {
                  background: t("colors.floating"),
                  borderRadius: t("metrics.media_small_radius"),
                }),
            symbolSize: t("metrics.symbol_inline"),
          }}
          onClick={() => send({ type: "removeAttachment", id: a.id })}
        />
      )}
    </div>
  );
}
const cardControl: Style = { borderRadius: t("radius.xs") };
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

const CitationContext = createContext<{
  threadId: string;
  cite: (q: Citation) => void;
  jump?: Citation & { nonce: number };
  reveal: (q: Citation) => void;
} | null>(null);
function QuoteSource({
  id,
  text,
  style = reading,
  code = false,
  userMessage = false,
}: {
  id: string;
  text: string;
  style?: Style;
  code?: boolean;
  userMessage?: boolean;
}) {
  const context = useContext(CitationContext);
  const [selection, setSelection] = useState<Citation | null>(null);
  const [anchor, setAnchor] = useState({ x: 0, y: 0 });
  useLayoutEffect(() => setSelection(null), [id, context?.threadId]);
  if (!context) return <Prose text={text} style={style} code={code} />;
  const cite = () => {
    if (selection) {
      context.cite(selection);
      setSelection(null);
    }
  };
  return (
    <Prose
      code={code}
      text={text}
      style={style}
      source={{
        id: `quote-source-${id}`,
        label: "Transcript text",
        tabIndex: 0,
        metadata: {
          quoteSource: `${context.threadId}:${id}`,
          ...(userMessage ? { userMessage: true } : {}),
          ...(context.jump?.messageId === id
            ? {
                revealQuote: [
                  context.jump.start,
                  context.jump.end,
                  context.jump.nonce,
                ],
              }
            : {}),
        },
        onSelect: (e) => {
          setSelection(
            e.selection
              ? quoteSelection(id, String(e.value ?? text), ...e.selection)
              : null,
          );
          setAnchor({ x: e.x ?? 0, y: e.y ?? 0 });
        },
        onKeyDown: (e) => {
          if (
            selection &&
            e.metaKey &&
            e.shiftKey &&
            (e.key === "." || e.key === ">")
          ) {
            e.stopPropagation();
            cite();
          }
        },
      }}
    >
      {selection && (
        <UI.ToolbarItem
          label="Cite selection"
          onPointerDown={(e) => e.stopPropagation()}
          title="Cite selection · ⌘⇧."
          metadata={{ symbol: "quote.opening" }}
          onClick={cite}
          style={{
            ...citationChip,
            color: t("colors.accent_text"),
            position: "absolute",
            left: anchor.x,
            top: anchor.y,
          }}
        >
          Cite
        </UI.ToolbarItem>
      )}
    </Prose>
  );
}
function CitationChips({ s, send }: { key?: string; s: Snapshot; send: Send }) {
  const [editing, setEditing] = useState<number | null>(null);
  const [comment, setComment] = useState("");
  const [hovered, setHovered] = useState<number | null>(null);
  const context = useContext(CitationContext);
  const quotes = s.draft.citations ?? [];
  const save = () => {
    send({
      type: "citations",
      citations: quotes.map((q, i) => (i === editing ? { ...q, comment } : q)),
    });
    setEditing(null);
  };
  if (!quotes.length) return null;
  return (
    <div
      label="Quoted context"
      style={{ ...row, flexWrap: "wrap", gap: t("metrics.gap") }}
    >
      {quotes.map((q, i) => (
        <div
          key={i}
          role="group"
          onPointerEnter={() => setHovered(i)}
          onPointerLeave={() => setHovered(null)}
          label={`Quoted: ${q.text}`}
          title={q.comment || q.text}
          style={{ ...column, ...citationChip, color: t("colors.accent_text") }}
        >
          <div style={row}>
            <UI.ToolbarItem
              label={`Quoted: ${q.text}`}
              title={q.comment || q.text}
              metadata={{ symbol: "quote.opening" }}
              onClick={() => {
                // The source prose owns native selection and scroll-to-visible.
                context?.reveal(q);
              }}
            >
              {citationLabel(q.text)}
              {q.comment ? " •" : ""}
            </UI.ToolbarItem>
            <UI.ToolbarItem
              label="Edit comment"
              metadata={{ symbol: "pencil" }}
              onClick={() => {
                setComment(q.comment);
                setEditing(i);
              }}
            />
            <UI.ToolbarItem
              label="Remove quote"
              onFocus={() => setHovered(i)}
              onBlur={() => setHovered(null)}
              style={{ opacity: hovered === i ? 1 : 0 }}
              metadata={{ symbol: "xmark" }}
              onClick={() =>
                send({
                  type: "citations",
                  citations: quotes.filter((_, index) => index !== i),
                })
              }
            />
          </div>
          {editing === i && (
            <Field
              label="Add a comment about this quote"
              multiline
              value={comment}
              metadata={{
                placeholder: "Add a comment about this quote",
                submitOnReturn: true,
                focusOnMount: true,
              }}
              onInput={(e) => setComment(String(e.value ?? ""))}
              onChange={save}
              onKeyDown={(e) => {
                if (e.key === "Enter") save();
                if (e.key === "Escape") setEditing(null);
                e.stopPropagation();
              }}
            />
          )}
        </div>
      ))}
    </div>
  );
}

/** A paragraph of styled runs, wrapped natively as one text. */
function Para({ runs, style }: { runs: Run[]; style?: Style }) {
  const text = runs.map((r) => r.t).join("");
  const styled = runs.some((r) => r.b || r.i || r.c || r.a);
  // Long unbroken tokens (paths, links, JSON) get places to wrap.
  const shown = runs.map((r) => ({
    ...r,
    t: breakable(r.t),
  }));
  return styled ? (
    <div
      label={text}
      metadata={{ runs: shown as unknown as Json, quoteText: text }}
      style={{ ...shrink, ...style }}
    />
  ) : (
    <text metadata={{ quoteText: text }} style={{ ...shrink, ...style }}>
      {breakable(text)}
    </text>
  );
}
/** Markdown as native text: paragraphs, headings, lists, quotes and code. */
const Prose = memo(function Prose({
  text,
  style = reading,
  source,
  children,
  code = false,
}: {
  code?: boolean;
  text: string;
  style?: Style;
  children?: ComponentChildren;
  source?: {
    id: string;
    label: string;
    tabIndex: number;
    metadata: Record<string, Json>;
    onSelect: (e: SurfaceEvent) => void;
    onKeyDown: (e: SurfaceEvent) => void;
  };
}) {
  // As v1 reads: paragraphs apart, a list's items close together.
  const groups: Block[][] = [];
  for (const block of markdown(text)) {
    const last = groups.at(-1);
    if (block.kind === "li" && last?.[0]?.kind === "li") last.push(block);
    else groups.push([block]);
  }
  return (
    <div
      {...source}
      style={{
        ...column,
        ...shrink,
        gap: t("metrics.inset"),
        ...(source ? { ...target, position: "relative" } : {}),
      }}
    >
      {code ? (
        <text
          metadata={{ quoteText: text }}
          data-pm-variant="code"
          style={style}
        >
          {breakable(text)}
        </text>
      ) : (
        groups.map((g, i) =>
          g.length === 1 ? (
            <ProseBlock key={i} block={g[0]!} style={style} />
          ) : (
            <div
              key={i}
              style={{ ...column, ...shrink, gap: t("metrics.pill_inset") }}
            >
              {g.map((b, j) => (
                <ProseBlock key={j} block={b} style={style} />
              ))}
            </div>
          ),
        )
      )}
      {children}
    </div>
  );
});

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
          <text
            style={{ ...style, color: t("colors.secondary"), flexShrink: 0 }}
          >
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
            borderRadius: t("radius.xs"),
            padding: t("metrics.text_padding"),
          }}
        >
          <text
            metadata={{ quoteText: b.text }}
            data-pm-variant="code"
            style={{
              fontSize: t("typography.minimum"),
              color: t("colors.text"),
              lineHeight: t("typography.dense_leading"),
            }}
          >
            {breakable(b.text)}
          </text>
        </div>
      );
    case "table":
      return (
        <div
          role="table"
          label="Table"
          style={{ ...column, overflow: "scroll" }}
        >
          {[b.headers, ...b.rows].map((cells, index) => (
            <div
              key={index}
              role="row"
              style={{ ...row, alignItems: "start", gap: t("metrics.gap") }}
            >
              {cells.map((runs, col) => (
                <div
                  key={col}
                  role={index === 0 ? "columnheader" : "cell"}
                  style={{
                    ...shrink,
                    flexGrow: 1,
                    minWidth: t("metrics.value_column"),
                    padding: t("metrics.gap"),
                  }}
                >
                  <Para
                    runs={runs}
                    style={{
                      ...style,
                      ...(index === 0
                        ? { fontWeight: t("typography.semibold_weight") }
                        : {}),
                    }}
                  />
                </div>
              ))}
            </div>
          ))}
        </div>
      );
    case "rule":
      return <div style={hairline} />;
  }
}

// ── Menus and the thread bar ────────────────────────────────────────────

/** Aero is offered: the picker is harness, then model. */
export function harnessMode(s: Snapshot): boolean {
  return s.catalog.providers.some(([id]) => id === "aero");
}
/** The provider (Settings › Providers) a harness's model runs on: Aero's
 * every model is Powermove inference. */
export function providerGroup(harness: string, model: string): string {
  if (harness === "chatgpt") return "OpenAI";
  if (harness === "claude") return "Anthropic";
  return model === FAKE_MODEL ? "Development" : "Powermove";
}
/** The development switch's demo model. */
const FAKE_MODEL = "fake/model";
/** An Aero catalogue model's own mark, from its provider (`pm/<provider>/<model>`):
 * Claude's for Anthropic, OpenAI's blossom, Pierre's neutral sparkle for the rest
 * (Google's marks need its approval). */
export function aeroModelMark(model: string): string {
  const provider = model.startsWith("pm/") ? model.split("/")[1] : "";
  return provider === "anthropic" ? "claude" : provider === "openai" ? "chatgpt" : "compatible";
}
/** The model the chooser shows: the chosen one, else the one that runs
 * when none is chosen (the provider's default, else the first listed). */
export function harnessModel(s: Snapshot, n?: NativeFacts): string {
  const chosen = s.settings.model;
  if (chosen && chosen !== "default") return chosen;
  const group = n?.modelGroups?.[s.settings.provider];
  return group?.find(([, , on]) => on)?.[0] ?? s.catalog.models[0]?.[0] ?? "";
}
/** The reasoning level the model runs at, always one of its own levels:
 * the chosen one when the model offers it, else the model's default, else
 * its first level ("" for a model with none). */
export function currentEffort(s: Snapshot, efforts: string[]): string {
  const chosen = s.settings.effort;
  if (chosen && efforts.includes(chosen)) return chosen;
  const fallback = s.catalog.defaultEffort;
  return fallback && efforts.includes(fallback) ? fallback : (efforts[0] ?? "");
}
/** How long a picked level shows in the pill without the kernel confirming
 * it, before the pill reads the kernel's again. */
const PICK_SETTLE_MS = 2000;
/** The mark the chooser and the pill show for a harness's model: Claude
 * Code's own, Codex's OpenAI blossom, and the Powermove mark for Aero. */
function providerMark(harness: string, group: string): string {
  if (harness === "claude") return "claude";
  if (harness === "chatgpt") return "chatgpt";
  return group === "Powermove" ? "powermove" : "compatible";
}
/** A model label's context note ("Opus 5.5 (1M context)" → "1M"), when it
 * names one. */
export function modelContext(label: string): string | undefined {
  return label.match(/\(([^)]*?)\s*context[^)]*\)/i)?.[1]?.trim() || undefined;
}
/** A model label without its context note. */
export function modelBase(label: string): string {
  return label.replace(/\s*\([^)]*context[^)]*\)/i, "");
}
/** How the chooser and the pill name a model: its base name, with a short
 * context qualifier ("Opus 5.5 (1M)") when another listed model shares
 * that base name, so two variants never read alike. */
export function chooserName(models: [string, string][], id: string, label: string): string {
  const base = modelBase(label);
  const context = modelContext(label);
  const shared = models.some(([other, text]) => other !== id && modelBase(text) === base);
  return context && shared ? `${base} (${context})` : base;
}
/** The account heading a harness's model is listed under in the chooser. */
export function accountHeading(harness: string, model: string, n?: NativeFacts): string {
  if (harness === "chatgpt") return n?.accounts?.chatgpt ?? "ChatGPT";
  if (harness === "claude") return n?.accounts?.claude ?? "Claude";
  return model === FAKE_MODEL ? "Development" : "Powermove";
}
/** A chooser heading's small mark: its provider's. */
function headingIcon(heading: string): string {
  if (heading === "ChatGPT" || heading === "OpenAI API key") return "provider.chatgpt";
  if (heading === "Claude" || heading === "Anthropic API key") return "provider.claude";
  return heading === "Powermove" ? "provider.powermove" : "provider.compatible";
}
/** Each harness's own mark in the chooser's rail. */
function harnessMark(harness: string): string {
  return harness === "aero" ? "powermove" : harness === "chatgpt" ? "chatgpt" : "claude";
}
/** The chosen harness has no account yet (Aero: the Powermove account): the
 * chooser leads to Settings › Providers. */
export function harnessSignedOut(s: Snapshot, n: NativeFacts): boolean {
  return (
    harnessMode(s) &&
    s.runId === null &&
    (n.connection === "waiting" || (n.connection === "unavailable" && !!n.signedOut))
  );
}
/** Aero signed in on Free: it needs a plan. Honest and quiet (no fault, no
 * Try Again); Subscribe… leads to Settings › Account. */
export function aeroNeedsPlan(s: Snapshot, n: NativeFacts): boolean {
  return s.settings.provider === "aero" && n.connection === "unavailable" && !n.signedOut && !!n.aero?.needsPlan;
}
/** Quiet credit notice copy for Aero once 80% or 95% of monthly credits are used. */
export function aeroLowCreditsCopy(
  low: NonNullable<NonNullable<NativeFacts["aero"]>["lowCredits"]>,
): string {
  const fmt = (v: number) => v.toLocaleString("en-US");
  const head = low.level === "critical" ? "Almost out of credits" : "Low on credits";
  const reset = low.resets ? ` · resets ${low.resets}` : "";
  return `${head}: ${fmt(low.remaining)} of ${fmt(low.included)} left${reset}`;
}
/** The one quiet line under a signed-out composer: what the chosen harness
 * runs on, and where to sign in. One line at the narrowest dock. */
export function signInCopy(harness: string): string {
  if (harness === "chatgpt") return "Sign in to ChatGPT in Settings › Providers";
  if (harness === "claude") return "Sign in to Claude in Settings › Providers";
  return "Sign in to Powermove in Settings › Providers";
}
/** One row of the window's field menu (the kit's searchable menu). */
export interface MenuOption {
  label: string;
  value: string;
  checked?: boolean;
  detail?: string;
  default?: boolean;
  /** The section it is listed under. */
  category?: string;
  /** Stable provider ID for model rows. */
  provider?: string;
  /** Choosing it keeps the menu open (a provider: its models follow). */
  stays?: boolean;
  /** Quiet status text, not a choice; with a provider, a greyed row in
   * that provider's group (its `detail` on the right). */
  informational?: boolean;
  /** Where a chooser row stands (harness, heading, model, connect,
   * effort, action). */
  kind?: string;
  /** The row's small mark (a symbol name). */
  icon?: string;
  /** Shown greyed, never chosen. */
  disabled?: boolean;
}
/** Opens the field menu at `anchor` (a node id); answers the chosen
 * value, or null when it closed unchosen. */
export type Menu = (request: {
  anchor: number;
  placeholder: string;
  options: MenuOption[];
  /** Composer pop-ups track their button rather than the legacy field menu.
   * `chooser`: one of the pill's own menus (the model picker, or with
   * `menu: "effort"` the reasoning menu). */
  picker?: "model" | "access" | "add" | "chooser";
  /** Which of the pill's menus a `chooser` request opens. */
  menu?: "effort";
  /** The model picker's usage monitor (Aero's credits, plan, reset day). */
  usage?: [string, string][];
  search?: boolean;
  /** New rows for the menu still open after a row that stays was chosen. */
  update?: boolean;
  /** Change the open model menu's rows without replacing its pending choice. */
  refresh?: boolean;
}) => Promise<unknown>;
/** A borderless trigger in a bar or header that opens the window's menu:
 * its value, then the pop-up chevron (as a bar NSPopUpButton). */
function Trigger({
  id,
  label,
  text,
  disabled,
  open,
  tone = "colors.secondary",
  fixed,
  compact = false,
}: {
  id: string;
  label: string;
  text: string;
  disabled?: boolean;
  open: (anchor: number) => void;
  tone?: string;
  /** A short label (an effort) keeps its width; the model's name gives way. */
  fixed?: boolean;
  compact?: boolean;
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
        ...row,
        ...(fixed ? { flexShrink: 0 } : shrink),
        gap: t("metrics.pill_inset"),
        paddingLeft: t(compact ? "metrics.pill_inset" : "metrics.gap"),
        paddingRight: t(compact ? "metrics.pill_inset" : "metrics.gap"),
        borderRadius: t("metrics.inner_radius"),
        color: t(tone),
      }}
    >
      <text
        style={{
          ...quiet,
          ...shrink,
          color: t(tone),
          whiteSpace: "nowrap",
          textOverflow: "ellipsis",
        }}
      >
        {text}
      </text>
      <div
        metadata={{ symbol: "chevron.up.chevron.down" }}
        style={{
          color: t("colors.muted"),
          symbolSize: t("metrics.symbol_inline"),
          flexShrink: 0,
        }}
      />
    </UI.ToolbarItem>
  );
}
/** The composer's pill: `[mark] Model  Effort ⌄`, the model picker and the
 * reasoning menu in one quiet control (w110: Aside's interaction, our look).
 * At rest it is text, the model in the secondary ink and its effort in the
 * muted one, at body size; its width is its text's, whatever the harness.
 * The part under the pointer, or whose menu is open, gets the pill (the
 * host draws it: `metadata.pill`), which glides between the two parts;
 * while it shows, the model's own chevron opens beside its name and pushes
 * the effort along, so the two read as separate. The open part's chevron
 * turns to point up. A model with no effort levels is the model part alone. */
function ModelChooser({
  mark,
  model,
  effort,
  disabled,
  opened,
  openModels,
  openEffort,
  signedOut = false,
}: {
  /** The provider mark before the name; none while signed out. */
  mark: string | null;
  model: string;
  /** Null for a model with no effort levels. */
  effort: string | null;
  disabled: boolean;
  opened: string | null;
  openModels: (anchor: number) => void;
  openEffort: (anchor: number) => void;
  /** The Powermove harness has no account: the name reads "Sign in to
   * Powermove" (or "Connect a provider") and its menu leads to Settings ›
   * Providers. */
  signedOut?: boolean;
}) {
  const ink = t(disabled ? "colors.faint" : "colors.secondary");
  const quietInk = t(disabled ? "colors.faint" : "colors.muted");
  const split = effort !== null;
  const chevron = (open: boolean, reveal: boolean) => (
    <div
      metadata={{ symbol: "composer.chevron", turn: open, ...(reveal ? { pill: "reveal" } : {}) }}
      style={{
        flexShrink: 0,
        width: t("metrics.composer_symbol"),
        height: t("metrics.composer_symbol"),
        symbolSize: t("metrics.composer_symbol"),
        marginLeft: t("space.xs"),
        color: quietInk,
      }}
    />
  );
  // No tooltips: the part's own text names it, and a tooltip would stand
  // over the composer under the pointer.
  const part = (
    which: "model" | "effort",
    label: string,
    open: (anchor: number) => void,
    flexible: boolean,
    children: ComponentChildren,
  ) => (
    <button
      id={`agent-${which}`}
      role="button"
      tabIndex={0}
      label={label}
      disabled={disabled}
      open={opened === which}
      aria-haspopup="menu"
      aria-expanded={opened === which}
      metadata={{ menuAnchor: true, pill: "part" }}
      preventDefault={{ keydown: ["Enter", " "] }}
      onClick={(e) => {
        if (!disabled) open(Number(e.currentTarget));
      }}
      onKeyDown={(e) => {
        if (disabled || e.repeat || e.metaKey || e.ctrlKey || e.altKey) return;
        if (e.key === "Enter" || e.key === " ") open(Number(e.currentTarget));
      }}
      style={{
        ...target,
        ...row,
        ...(flexible ? shrink : { flexShrink: 0 }),
        height: t("metrics.hit"),
        paddingLeft: t("space.sm"),
        paddingRight: t("space.sm"),
        borderRadius: t("metrics.inner_radius"),
      }}
    >
      {children}
    </button>
  );
  return (
    <div id="agent-chooser" metadata={{ pill: "group" }} style={{ ...row, ...shrink }}>
      {part(
        "model",
        signedOut ? model : `Model, ${model}`,
        openModels,
        true,
        <>
          {mark !== null && (
            <div
              metadata={{ symbol: `provider.${mark}` }}
              style={{
                flexShrink: 0,
                width: t("metrics.symbol_panel"),
                height: t("metrics.symbol_panel"),
                symbolSize: t("metrics.symbol_panel"),
                marginRight: t("space.sm"),
                color: ink,
              }}
            />
          )}
          <text
            style={{
              ...shrink,
              fontSize: t("typography.body"),
              color: ink,
              whiteSpace: "nowrap",
              textOverflow: "ellipsis",
            }}
          >
            {modelBase(model)}
          </text>
          {chevron(opened === "model", split)}
        </>,
      )}
      {split &&
        part(
          "effort",
          `Reasoning effort, ${effort}`,
          openEffort,
          false,
          <>
            <text
              style={{
                flexShrink: 0,
                fontSize: t("typography.body"),
                color: quietInk,
                whiteSpace: "nowrap",
              }}
            >
              {effort}
            </text>
            {chevron(opened === "effort", false)}
          </>,
        )}
    </div>
  );
}
function Header({
  s,
  n,
  send,
  menu,
}: {
  s: Snapshot;
  n: NativeFacts;
  send: Send;
  menu?: Menu;
}) {
  const popover = n.presentation === "popover";
  const title = threadName(s.threadTitle || "");
  // Switching while a change is applied would leave it half done.
  const blocked = s.composerDisabled;
  return (
    // The header slot: the popover draws it on its own ground; docked, it
    // shares the dock's tab row.
    <div
      id="agent-header"
      data-pm-slot="header"
      role="group"
      label="Agent threads"
      style={{
        ...row,
        flexShrink: 0,
        gap: t("metrics.pill_inset"),
        height: t("metrics.header"),
        ...(popover ? {
          paddingLeft: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        } : {}),
      }}
    >
      <Trigger
        id="agent-threads"
        label={
          blocked
            ? "Finish applying the current change to switch threads"
            : `Switch thread, ${title}`
        }
        // Docked, the trigger names the list and the tab keeps its name.
        text={popover ? title : "Threads"}
        compact={!popover}
        fixed={!popover}
        tone="colors.text"
        disabled={!menu || blocked}
        open={(anchor) => {
          const threads = s.threads ?? [];
          const current = threads.find((t) => t.id === s.threadId);
          void menu?.({
            anchor,
            placeholder: "Search threads…",
            options: [
              ...(blocked ? [] : [{ label: "New Thread", value: "new" }]),
              ...nestedThreads(threads).map(({ thread: t, depth }) => ({
                label: `${"  ".repeat(depth)}${depth ? "↳ " : ""}${threadName(t.title)}${t.busy ? " · Working…" : ""}`,
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
            if (typeof choice !== "string") return;
            if (choice === "new") send({ type: "newThread" });
            else if (choice.startsWith("thread:")) {
              const id = choice.slice(7);
              if (id !== s.threadId && !blocked)
                send({ type: "switchThread", id });
            } else if (choice.startsWith("delete:"))
              send({ type: "deleteThread", id: choice.slice(7) });
          });
        }}
      />
      {popover && <div style={{ flexGrow: 1 }} />}
      <UI.ToolbarItem
        id="agent-new"
        label="New Thread"
        title="New Thread"
        disabled={blocked}
        metadata={{ symbol: "plus" }}
        style={{ flexShrink: 0, symbolSize: t("metrics.symbol_panel") }}
        onClick={() => {
          if (!blocked) send({ type: "newThread" });
        }}
      />
      {popover && (
        <UI.ToolbarItem
          id="agent-presentation"
          label="Pin to Workspace"
          title="Pin to Workspace"
          metadata={{ symbol: "pin" }}
          style={{ flexShrink: 0, symbolSize: t("metrics.symbol_panel") }}
          onClick={() => send({ type: "shell", action: "pin" })}
        />
      )}
    </div>
  );
}

function SubthreadBar({ s, send }: { s: Snapshot; send: Send }) {
  const path = threadPath(s.threads ?? [], s.threadId);
  if (path.length < 2) return null;
  const active = path.at(-1)!;
  return (
    <div
      id="agent-subthread"
      role="group"
      label="Thread path"
      style={{
        ...column,
        gap: t("metrics.gap"),
        background: t("colors.fill"),
        borderRadius: t("metrics.inner_radius"),
        marginLeft: t("metrics.text_padding"),
        marginRight: t("metrics.text_padding"),
        padding: t("metrics.gap"),
      }}
    >
      <div style={{ ...row, flexWrap: "wrap", gap: t("metrics.gap") }}>
        {path.slice(0, -1).map((thread) => (
          <UI.ToolbarItem
            key={thread.id}
            label={`Back to ${threadName(thread.title)}`}
            onClick={() => send({ type: "switchThread", id: thread.id })}
            disabled={s.composerDisabled}
          >
            <text style={quiet}>{threadName(thread.title)} ›</text>
          </UI.ToolbarItem>
        ))}
        <text style={quiet}>{threadName(active.title)}</text>
      </div>
      <text style={faint}>
        {active.kind === "subagent"
          ? active.busy
            ? "A subagent is working here. Your messages go to it."
            : "A subagent worked here. Send a message to start a new turn."
          : "The agent started this thread."}
      </text>
    </div>
  );
}

// ── Empty state and connection ───────────────────────────────────────────

function Welcome({
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
        flexShrink: 0,
        paddingTop: t("metrics.agent_gap"),
        paddingBottom: t("metrics.inset"),
        gap: t(compact ? "space.hair" : "metrics.inset"),
      }}
    >
      <div style={{ ...column, gap: t("metrics.gap") }}>
        <text
          id="agent-welcome-title"
          style={{
            ...strong,
            fontSize: t(compact ? "typography.title" : "typography.heading"),
            ...(compact ? { lineHeight: t("typography.dense_leading") } : {}),
          }}
        >
          Make your next move
        </text>
        {!compact && !aeroNeedsPlan(s, n) && (
          <text id="agent-welcome-description" style={quiet}>
            {explanation}
          </text>
        )}
        {aeroNeedsPlan(s, n) && (
          <div id="agent-welcome-plan" style={{ ...row, gap: t("metrics.gap") }}>
            <text style={quiet}>Aero needs Aerolite.</text>
            <button
              id="agent-welcome-subscribe"
              role="button"
              tabIndex={0}
              label="Subscribe"
              onClick={() => send({ type: "subscribe" })}
              style={{ ...target, color: t("colors.accent_text") }}
            >
              <text style={{ ...strong, color: t("colors.accent_text") }}>Subscribe…</text>
            </button>
          </div>
        )}
      </div>
      <div
        id="agent-suggestions"
        style={{ ...column, gap: t(compact ? "space.hair" : "metrics.gap") }}
      >
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
              // A raised neutral button with a hairline edge, legible on
              // the popover's floating fill and on the docked panel alike;
              // the label in body text, the chevron in secondary. Hover
              // and press wash it (the host's control washes).
              style={{
                ...target,
                ...row,
                gap: t("metrics.gap"),
                flexShrink: 0,
                minHeight: t(compact ? "metrics.hit" : "metrics.control"),
                paddingTop: t(compact ? "space.hair" : "metrics.gap"),
                paddingBottom: t(compact ? "space.hair" : "metrics.gap"),
                paddingLeft: t("metrics.text_padding"),
                paddingRight: t("metrics.text_padding"),
                background: t("colors.raised"),
                borderWidth: t("metrics.border"),
                borderColor: t("colors.hairline"),
                borderRadius: t("metrics.control_radius"),
              }}
            >
              <text
                style={{
                  ...shrink,
                  flexGrow: 1,
                  color: t("colors.text"),
                  fontSize: t("typography.body"),
                }}
              >
                {label}
              </text>
              <div
                metadata={{ symbol: "chevron.right" }}
                style={{
                  flexShrink: 0,
                  color: t("colors.secondary"),
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
  // Signed out is the plain ask (v1.2.0's gate): the reason is for a fault.
  const body = checking
    ? `Checking your ${name} connection.`
    : (!n.signedOut && n.reason) ||
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
      {/* v1.2.0's provider switch: the kit's one segmented control. */}
      <div style={{ ...row, justifyContent: "center" }}>
        <UI.SegmentedControl
          id="agent-connect-provider"
          label="Connection provider"
          value={provider}
          options={s.catalog.providers.map(([id, label]) => ({
            id,
            label: id === "compatible" ? "API / local" : label,
          }))}
          onChange={(id) => {
            if (id !== provider) send({ type: "setProvider", provider: id });
          }}
        />
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
              style={cardControl}
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
        <div style={{ ...row, justifyContent: "center" }}>
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
                style={cardControl}
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
                  borderRadius: t("radius.xs"),
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
              borderRadius: t("radius.xs"),
              padding: t("metrics.text_padding"),
            }}
          >
            <text
              data-pm-variant="code"
              style={{
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

function UserMessageView({
  m,
  send,
  pending,
  retry,
}: {
  key?: string | number;
  m: Message;
  send: Send;
  pending?: boolean;
  retry?: () => void;
}) {
  const text = str(m.value.text);
  const files = Array.isArray(m.value.attachments) ? m.value.attachments : [];
  const citations = Array.isArray(m.value.citations) ? m.value.citations : [];
  const delivery =
    m.value.delivery === "queued"
      ? "Queued for next step"
      : m.value.delivery === "steering"
        ? "Steering"
        : m.value.delivery === "failed"
          ? "Couldn’t send"
          : null;
  const shown =
    text ||
    (files.length
      ? `Attached ${files.length} file${files.length === 1 ? "" : "s"}`
      : "");
  return (
    <div
      role="listitem"
      label={delivery ? `You said: ${shown} (${delivery})` : `You said: ${shown}`}
      metadata={{ rowId: m.rowId }}
      // The bubble keeps a reply's width of room on its leading side.
      style={{
        ...row,
        justifyContent: "end",
        paddingLeft: t("metrics.section_gap"),
      }}
    >
      <div
        style={{ ...column, ...shrink, alignItems: "end", gap: t("metrics.gap") }}
      >
        {files.length > 0 && (
          <div
            style={{
              ...row,
              justifyContent: "end",
              flexWrap: "wrap",
              gap: t("metrics.gap"),
            }}
          >
            {files.map((f, i) => (
              <AttachmentTile key={i} a={obj(f)} small send={send} />
            ))}
          </div>
        )}
        {(citations.length > 0 || text || pending || delivery) && (
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
            }}
          >
            {citations.map((value, i) => {
              const q = obj(value);
              return (
                <div
                  key={i}
                  role="group"
                  label={`Quoted: ${str(q.text)}`}
                  style={{ ...column, ...citationChip, ...cardControl }}
                >
                  <text style={quiet}>{"❝ " + str(q.text)}</text>
                  {str(q.comment) && (
                    <text style={reading}>{str(q.comment)}</text>
                  )}
                </div>
              );
            })}
            {text && <QuoteSource id={m.rowId} text={text} userMessage />}
            {pending && (
              <text role="status" label="Steering" style={faint}>
                Steering
              </text>
            )}
            {delivery &&
              (retry ? (
                <div style={{ ...row, gap: t("metrics.gap") }}>
                  <text style={quiet}>{delivery} ·</text>
                  <button
                    role="button"
                    label="Retry"
                    tabIndex={0}
                    style={quiet}
                    onClick={retry}
                  >
                    Retry
                  </button>
                </div>
              ) : (
                <text style={quiet}>{delivery}</text>
              ))}
          </div>
        )}
      </div>
    </div>
  );
}
const UserMessage = memo(
  UserMessageView,
  (a, b) =>
    a.pending === b.pending && a.retry === b.retry &&
    JSON.stringify(a.m) === JSON.stringify(b.m),
);
const MEDIA_KIND: Record<string, string> = {
  image: "Image",
  video: "Video",
  audio: "Sound",
};
function AssistantMessageView({
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
    return (
      <Result s={s} send={send} text={text} result={obj(v.nativeResult)} />
    );
  return (
    <div
      role="listitem"
      label={`Agent: ${plain(text)}`}
      metadata={{ rowId: m.rowId }}
      style={{ ...column, gap: t("metrics.text_padding") }}
    >
      {text && <QuoteSource id={m.rowId} text={text} />}
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
      <ReplyCards value={v} rowId={m.rowId} send={send} />
    </div>
  );
}
const AssistantMessage = memo(AssistantMessageView, (a, b) => {
  // Only inert prose can ignore changing command callbacks and run facts.
  // Interactive replies always render, keeping retry/media actions current.
  const inert = ({ value: v }: Message) =>
    !v.error &&
    !v.nativeResult &&
    v.kind !== "result" &&
    !v.requiresProject &&
    !Array.isArray(v.visualReplies) &&
    !Array.isArray(v.media);
  return (
    inert(a.m) && inert(b.m) && JSON.stringify(a.m) === JSON.stringify(b.m)
  );
});
/** What a reply names besides its words: interactive visual replies (each
 * opens in its own viewer) and the images, videos and sounds it mentions
 * (each opens in its own app; a web address only when asked). */
function ReplyCards({
  value: v,
  rowId,
  send,
}: {
  value: Obj;
  rowId: string;
  send: Send;
}) {
  const visuals = Array.isArray(v.visualReplies) ? v.visualReplies : [];
  const media = Array.isArray(v.media) ? v.media : [];
  if (!visuals.length && !media.length) return null;
  const m = { rowId };
  return (
    <>
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
              id={`${m.rowId}-${str(r.id)}`}
              style={cardControl}
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
                id={`${m.rowId}-media-${index}`}
                style={cardControl}
                label={remote ? "Open in Browser" : "Open"}
                onClick={() => send({ type: "openMedia", path, reveal: false })}
              />
              {!remote && (
                <UI.PushButton
                  id={`${m.rowId}-media-${index}-reveal`}
                  style={cardControl}
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
    </>
  );
}
function PlanPreview({ s, send }: { s: Snapshot; send: Send }) {
  const plan = obj(s.plan);
  if (!s.plan) return null;
  const typed =
    typeof plan.revision === "number" &&
    Number.isFinite(plan.revision) &&
    Array.isArray(plan.commands) &&
    !plan.sceneEdit &&
    (!plan.kind || plan.kind === "native");
  const allowed =
    typed &&
    s.phase === "preview" &&
    s.runId === null &&
    !s.composerDisabled &&
    s.resultActions?.apply === true;
  return (
    <div
      role="group"
      label="Proposed change"
      style={{ ...card, margin: t("metrics.text_padding") }}
    >
      <text style={strong}>Proposed change</text>
      <Prose
        text={
          str(plan.summary) ||
          str(plan.text) ||
          str(plan.title) ||
          str(obj(plan.sceneEdit).label) ||
          str(s.plan) ||
          "Review the proposed change."
        }
      />
      {allowed && (
        <UI.PushButton
          label="Apply change"
          style={cardControl}
          onClick={() => send({ type: "applyPlan" })}
        />
      )}
    </div>
  );
}
const generatedResultSummaries = new Set([
  "Agent result",
  "Agent changes",
  "Applied agent plan",
  "This plan made no changes.",
]);

function Result({
  s,
  send,
  text,
  result,
}: {
  s: Snapshot;
  send: Send;
  text: string;
  result: Obj;
}) {
  const current = !!s.result && str(result.runId) === s.result.runId;
  const ready =
    current && s.phase === "result" && s.runId === null && !s.composerDisabled;
  const extensions = Array.isArray(result.extensions) ? result.extensions : [];
  const warning = str(result.verificationError);
  const summary = str(result.summary);
  // Earlier receipts combined the summary and the warning into saved prose.
  const textWithoutWarning =
    warning && text.endsWith(`\n\n${warning}`)
      ? text.slice(0, -(warning.length + 2))
      : text === warning
        ? ""
        : text;
  const prose = textWithoutWarning || summary;
  return (
    <div role="listitem" label="Rendered result" style={card}>
      <text style={{ ...faint, fontWeight: t("typography.semibold_weight") }}>
        Rendered result
      </text>
      <text style={strong}>Review the actual result</text>
      {prose && !generatedResultSummaries.has(prose) && <Prose text={prose} />}
      {warning && (
        <text role="status" style={quiet}>
          {warning}
        </text>
      )}
      {extensions.map((entry, index) => {
        const e = obj(entry);
        const canOpen =
          e.status === "ready" &&
          e.action !== "removed" &&
          !!str(e.extensionId) &&
          !!str(e.panelId);
        return (
          <div key={index} style={{ ...row, gap: t("metrics.gap") }}>
            <text style={{ ...quiet, ...shrink, flexGrow: 1 }}>
              {str(e.name)}
              {e.action === "removed"
                ? " · Removed"
                : e.action === "created"
                  ? " · Created"
                  : " · Updated"}
            </text>
            {canOpen && (
              <UI.PushButton
                label={`Open panel, ${str(e.name)}`}
                style={cardControl}
                onClick={() =>
                  send({
                    type: "openResultPanel",
                    extensionId: str(e.extensionId),
                    panelId: str(e.panelId),
                  })
                }
              />
            )}
          </div>
        );
      })}
      {ready && (
        <div
          style={{
            ...row,
            justifyContent: "end",
            gap: t("metrics.text_padding"),
          }}
        >
          {s.resultActions?.undo === true && (
            <UI.PushButton
              label="Undo Change"
              style={cardControl}
              onClick={() => send({ type: "undoResult" })}
            />
          )}
          {s.resultActions?.keep === true && (
            <UI.PushButton
              label="Keep Change"
              style={cardControl}
              onClick={() => send({ type: "keepResult" })}
            />
          )}
        </div>
      )}
    </div>
  );
}
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
          <div style={{ ...column, gap: t("metrics.text_padding") }}>
            <QuoteSource id={r.row.id} text={r.text} />
            <ReplyCards
              value={r.row.value as Obj}
              rowId={r.row.id}
              send={send}
            />
          </div>
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
          <QuoteSource id={r.key} text={r.text} style={faint} />
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
            <Chip key={f}>{f}</Chip>
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
          id={`${r.key}-work`}
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
      {running && !open ? (
        // A small spinner while the call runs (the kit's indeterminate row).
        <UI.ProgressRow label={`${d.label} is running`}>{[]}</UI.ProgressRow>
      ) : (
        <div
          metadata={{ symbol: open ? "chevron.down" : symbol }}
          style={{
            color: t(d.status === "error" ? "colors.danger" : "colors.muted"),
            symbolSize: t("metrics.symbol_inline"),
            flexShrink: 0,
          }}
        />
      )}
      <text style={{ ...quiet, color: t("colors.text"), flexShrink: 0 }}>
        {d.label}
      </text>
      {d.detail &&
        !(open && d.kind === "thought") &&
        (d.kind === "thought" ? (
          <text
            style={{
              ...faint,
              ...shrink,
              whiteSpace: "nowrap",
              textOverflow: "ellipsis",
            }}
          >
            {d.detail}
          </text>
        ) : (
          <Chip>{d.detail}</Chip>
        ))}
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
          <QuoteSource
            id={d.sourceId}
            text={
              d.output.length > 6000
                ? `${d.output.slice(0, 6000)}\n…`
                : d.output
            }
            code={d.kind !== "thought" && !d.task}
            style={
              d.kind === "thought" || d.task
                ? faint
                : {
                    fontSize: t("typography.minimum"),
                    color: t("colors.secondary"),
                    lineHeight: t("typography.dense_leading"),
                  }
            }
          />
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
                            borderRadius: t("radius.xs"),
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
                    style={cardControl}
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
              style={cardControl}
              label="Skip"
              onClick={() =>
                send({ type: "answer", step: r.commandId, skip: true })
              }
            />
          )}
          <UI.PushButton
            id={`${r.id}-answer`}
            style={cardControl}
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
  starting,
  retryLocal,
}: {
  s: Snapshot;
  send: Send;
  local: Local;
  starting: "waiting" | "delayed" | null;
  retryLocal?: { id: string; run: () => void };
}) {
  const list = entries(s);
  const render = (e: Entry): ComponentChildren => {
    if (e.kind === "working") {
      if (starting === "waiting") return null;
      const label = starting === "delayed" ? "Starting…" : e.label;
      return (
        <div
          key={e.key}
          role="status"
          label={label}
          style={{ ...hairline, paddingBottom: t("metrics.inset") }}
        >
          <text style={{ ...faint, fontVariantNumeric: "tabular-nums" }}>
            {label}
          </text>
        </div>
      );
    }
    if (e.kind === "work") {
      const open = local.open[e.key] ?? false;
      return (
        <div key={e.key} style={{ ...column, gap: t("metrics.agent_gap") }}>
          <button
            id={e.key}
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
          {open && e.items.map((m) => item(m))}
        </div>
      );
    }
    return item(e.message, e.pending === true);
  };
  const item = (m: Message, pending = false) =>
    m.role === "user" ? (
      <UserMessage key={m.key} m={m} send={send} pending={pending}
        retry={m.value.delivery !== "failed" || s.runId !== null
          ? undefined
          : retryLocal?.id === m.rowId
            ? retryLocal.run
            : () => send({ type: "retry", id: m.rowId })}
      />
    ) : m.role === "assistant" ? (
      <AssistantMessage key={m.key} m={m} s={s} send={send} />
    ) : (
      <Trail key={m.key} m={m} s={s} send={send} local={local} />
    );
  return <>{list.map(render)}</>;
}

// ── Composer ─────────────────────────────────────────────────────────────

/** The catalog's name for a value (the value itself when unlisted). The
 *  model already names the provider's default model for "no choice"; only
 *  before a provider lists its models is there none, and then the picker
 *  names the provider (`fallback`), never a "Default" that is no model. */
function named(entries: [string, string][], current: string, fallback = "") {
  return entries.find(([v]) => v === current)?.[1] || current || fallback;
}
/** Names a catalogue listed this session, so a thread keeps its model's name
 *  while the catalogue is away (signed out, or a 401). */
const seenModelNames = new Map<string, string>();
/** A model's name: the catalogue's, else `modelDisplayName`, else `fallback`
 *  when no model is chosen. Never the raw id. */
function modelNamed(entries: [string, string][], id: string, fallback = "") {
  const label = entries.find(([v]) => v === id)?.[1];
  if (label && label !== id) {
    seenModelNames.set(id, label);
    return label;
  }
  return id ? (seenModelNames.get(id) ?? modelDisplayName(id)) : fallback;
}
/** A saved model the catalog no longer lists stays visible, by name, rather
 *  than missing; no choice is never a row of its own. */
function pairs(entries: [string, string][], current: string) {
  const options = entries.map(([v, label]) => ({ v, label: label && label !== v ? label : modelNamed(entries, v) }));
  return !current || entries.some(([v]) => v === current)
    ? options
    : [...options, { v: current, label: modelNamed(entries, current) }];
}
function Composer({
  s,
  n,
  send,
  submit,
  stop,
  submitting,
  optimistic,
  menu,
  shown,
}: {
  key?: string;
  s: Snapshot;
  n: NativeFacts;
  send: Send;
  submit: (action: Action) => void;
  stop: () => void;
  submitting: boolean;
  optimistic?: string;
  menu?: Menu;
  /** Counts the panel's showings (each change of presentation). */
  shown: number;
}) {
  const [dragging, setDragging] = useState(false);
  const working = s.runId !== null;
  const harness = harnessMode(s);
  const available = n.connection === "connected" || working;
  const disabled = !available || s.composerDisabled;
  // With the Powermove harness, signed out keeps the composer as it is: a
  // draft can be written; the chooser and one line under it lead to
  // Settings › Providers.
  const signedOut = harnessSignedOut(s, n);
  const fieldDisabled = harness ? s.composerDisabled : disabled;
  const acceptsFiles = !disabled && s.attachments.length < 6;
  const dropTarget = acceptsFiles && (dragging || n.dropTarget);
  const placeholder = !available && !harness
    ? "Connect an agent to start"
    : working
      ? "Add direction…"
      : "Ask, edit, build anything.";
  const hasDraft = s.draft.text.trim().length > 0 || s.attachments.length > 0;
  // The composer's one surface: lifted off the panel; in the popover, whose
  // ground is the floating surface, a step more. Drops add only the
  // selected-fill wash. The draft is written on it (the field takes the
  // same fill, so no well shows).
  const surface = t(
    dropTarget
      ? "colors.selected_fill"
      : n.presentation === "popover"
        ? "colors.raised"
        : "colors.floating",
  );
  const openModelMenu = useRef<{ anchor: number; rows: string } | null>(null);
  const requestModelMenu: Menu = async (request) => {
    const open = {
      anchor: request.anchor,
      rows: JSON.stringify(request.options),
    };
    openModelMenu.current = open;
    try {
      return await menu?.(request);
    } finally {
      if (openModelMenu.current === open) openModelMenu.current = null;
    }
  };
  const modelName = modelNamed(
    s.catalog.models,
    s.settings.model,
    named(s.catalog.providers, s.settings.provider),
  );
  const shownModel = harnessModel(s, n);
  // The model's reasoning effort is its own menu beside the model's; a
  // model with no effort levels lists none, and the pill has no effort part.
  const efforts = s.catalog.efforts.filter((e) => e && e !== "default");
  // A level just picked from this model's own list shows in the pill at
  // once, before the kernel's next publication carries it (nothing flickers
  // back). Only a level: a picked model's levels and default come with the
  // kernel's publication, so the pill never pairs a model with a level it
  // can't run. A pick the kernel refused falls back when it answers, or
  // after a beat.
  const [picked, setPicked] = useState<string | null>(null);
  useLayoutEffect(
    () => setPicked(null),
    [s.settings.provider, s.settings.model, s.settings.effort, efforts.join(" ")],
  );
  useLayoutEffect(() => {
    if (picked === null) return;
    const timer = setTimeout(() => setPicked(null), PICK_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [picked]);
  const effortNow =
    picked !== null && efforts.includes(picked) ? picked : currentEffort(s, efforts);
  const effortName = effortNow ? effortLabel(effortNow) : "";
  const choicesLocked = working || s.composerDisabled;
  const [openPicker, setOpenPicker] = useState<string | null>(null);
  const menuEpoch = useRef(0);
  // Provider and model share one menu. A provider row keeps it open: the
  // Model section follows that provider, its default ticked, in the same
  // menu; it closes on a model, Escape or a click outside.
  const latest = useRef(s);
  latest.current = s;
  const native = useRef(n);
  native.current = n;
  const awaiting = useRef<{
    anchor: number;
    provider: string;
    epoch: number;
  } | null>(null);
  // With the harnesses, the pill's model part opens the model picker
  // (PM-727, Capy's layout): the harnesses as a rail attached to the card of
  // the models the chosen one runs, under their account headings, with
  // Aero's usage monitor at its foot. Signed out, the card leads to
  // Settings › Providers.
  const chooserRequest = (s: Snapshot): { options: MenuOption[]; usage?: [string, string][] } => {
    const n = native.current;
    const harness = s.settings.provider;
    const harnessName = named(s.catalog.providers, harness);
    const options: MenuOption[] = s.catalog.providers.map(([id, label]) => ({
      kind: "harness",
      label,
      value: `provider:${id}`,
      checked: id === harness,
      icon: `provider.${harnessMark(id)}`,
      stays: true,
    }));
    const listed = pairs(s.catalog.models, s.settings.model);
    // Nothing to choose: one compact pane with its one action (Try Again
    // for a harness that couldn't start), and no card. The composer's own
    // line says where to sign in.
    // Aero on Free: one quiet line and Subscribe…, which opens Settings ›
    // Account on its plans.
    if (aeroNeedsPlan(s, n) && s.runId === null) {
      options.push({ kind: "status", label: "Aero needs Aerolite.", value: "" });
      options.push({ kind: "action", label: "Subscribe…", value: "subscribe:" });
      return { options };
    }
    const fault = !harnessSignedOut(s, n) && n.connection === "unavailable" && s.runId === null;
    if (harnessSignedOut(s, n) || !listed.length || fault) {
      // The compact card says it once: what's wrong, then its one button.
      const status = fault
        ? `${harnessName} couldn’t start.`
        : n.findingModels && !harnessSignedOut(s, n)
          ? "Finding models…"
          : signInCopy(harness);
      options.push({ kind: "status", label: status, value: "" });
      options.push(
        fault
          ? { kind: "action", label: "Try Again", value: "retry:" }
          : harness === "aero"
            ? { kind: "action", label: "Sign In to Powermove…", value: "providers:" }
            : { kind: "action", label: "Connect a provider…", value: "providers:" },
      );
      return { options };
    }
    const shown = harnessModel(s, n);
    const groups = new Map<string, MenuOption[]>();
    for (const { v, label } of listed) {
      const heading = accountHeading(harness, v, n);
      const rows = groups.get(heading) ?? [];
      rows.push({
        kind: "model",
        label: chooserName(s.catalog.models, v, label),
        value: `model:${v}`,
        checked: v === shown,
        icon: `provider.${harness === "aero" ? aeroModelMark(v) : providerMark(harness, providerGroup(harness, v))}`,
      });
      groups.set(heading, rows);
    }
    const order = ["Powermove", "ChatGPT", "OpenAI API key", "Claude", "Anthropic API key", "Development"];
    const rank = (heading: string) => order.indexOf(heading) >>> 0;
    for (const heading of [...groups.keys()].sort((a, b) => rank(a) - rank(b)))
      options.push({ kind: "heading", label: heading, value: "", icon: headingIcon(heading) }, ...groups.get(heading)!);
    // Out of credits: until when, and the bigger plan (Settings › Account).
    const spent = harness === "aero" ? n.aero?.outOfCredits : null;
    if (spent) {
      options.push({
        kind: "status",
        label: spent.resets ? `Out of credits until ${spent.resets}.` : "Out of credits this month.",
        value: "",
      });
      if (spent.upgrade) options.push({ kind: "action", label: `Upgrade to ${spent.upgrade}…`, value: "providers:" });
    }
    options.push({ kind: "add", label: "Providers…", value: "providers:" });
    // Aero's usage monitor (credits, plan, reset day) once the backend has
    // answered; Claude Code and Codex report none.
    const usage = harness === "aero" && n.aero?.usage.length ? n.aero.usage : undefined;
    return { options, ...(usage ? { usage } : {}) };
  };
  const modelRows = (s: Snapshot): MenuOption[] =>
    harnessMode(s) ? chooserRequest(s).options : s.catalog.providers.flatMap(([provider, providerName]) => {
      const active = provider === s.settings.provider;
      const group = native.current.modelGroups?.[provider];
      const category = provider === "compatible"
        ? (native.current.connectedModelName ?? providerName)
        : providerName;
      const models = active
        ? pairs(s.catalog.models, s.settings.model)
        : (group ?? []).map(([v, label]) => ({ v, label }));
      const defaults = new Set(group?.filter(([, , on]) => on).map(([id]) => id));
      const rows: MenuOption[] = models.map(({ v, label }) => ({
        label: label.replace(/\s*\([^)]*context[^)]*\)/i, ""),
        detail: label.match(/\(([^)]*context[^)]*)\)/i)?.[1],
        value: active ? `model:${v}` : `other:${provider}:${v}`,
        checked: active && v === s.settings.model,
        default: defaults.has(v) || undefined,
        category,
        provider,
      }));
      if (!rows.length && !active)
        rows.push({ label: `Use ${category}`, category, value: `provider:${provider}`, stays: true });
      if (active && native.current.findingModels && !s.catalog.models.some(([id]) => id !== s.settings.model))
        rows.push({ label: "Finding models…", category, value: "", informational: true });
      return rows;
    });
  const modelSearch = (s: Snapshot) => s.catalog.models.length > 12;
  /** The model menu's request, or the chooser's with the Powermove harness. */
  const modelRequest = (s: Snapshot) => {
    if (!harnessMode(s))
      return { picker: "model" as const, search: modelSearch(s), placeholder: "Search models", options: modelRows(s) };
    return { picker: "chooser" as const, placeholder: "Search models", ...chooserRequest(s) };
  };
  const modelMenu = (
    anchor: number,
    update: boolean,
    epoch = menuEpoch.current,
  ) =>
    void requestModelMenu({
      anchor,
      ...modelRequest(latest.current),
      ...(update ? { update } : {}),
    }).then((choice) => {
      if (menuEpoch.current !== epoch) return;
      if (typeof choice !== "string") {
        setOpenPicker(null);
        return;
      }
      const at = choice.indexOf(":");
      const [kind, value] = [choice.slice(0, at), choice.slice(at + 1)];
      const now = latest.current;
      if (kind === "subscribe") {
        setOpenPicker(null);
        send({ type: "subscribe" });
      } else if (kind === "providers") {
        setOpenPicker(null);
        send({ type: "openSettings" });
      } else if (kind === "retry") {
        setOpenPicker(null);
        send({ type: "retryConnection" });
      } else if (kind === "provider") {
        if (value === now.settings.provider) return modelMenu(anchor, true);
        awaiting.current = { anchor, provider: value, epoch };
        send({ type: "setProvider", provider: value });
        // A switch the model refused: the open menu keeps working.
        setTimeout(() => {
          const pending = awaiting.current;
          if (
            pending?.provider !== value ||
            pending.epoch !== epoch ||
            menuEpoch.current !== epoch
          )
            return;
          awaiting.current = null;
          modelMenu(pending.anchor, true);
        }, 1000);
      }
      // The effort shown goes along: the new model keeps it when it
      // takes it, else runs at its own default.
      else if (kind === "model") {
        setOpenPicker(null);
        send({
          type: "setModel",
          model: value,
          effort: now.settings.effort,
        });
      } else if (kind === "other") {
        const colon = value.indexOf(":");
        setOpenPicker(null);
        send({ type: "setProvider", provider: value.slice(0, colon), model: value.slice(colon + 1) });
      }
    });
  const openModels = (anchor: number) => {
    menuEpoch.current++;
    awaiting.current = null;
    setOpenPicker("model");
    modelMenu(anchor, false);
  };
  useLayoutEffect(() => {
    const pending = awaiting.current;
    if (!pending || s.settings.provider !== pending.provider) return;
    awaiting.current = null;
    modelMenu(pending.anchor, true);
  }, [s.settings.provider, s.catalog.models]);
  useLayoutEffect(() => {
    const open = openModelMenu.current;
    if (!open) return;
    const options = modelRows(s);
    const rows = JSON.stringify(options);
    if (rows === open.rows) return;
    open.rows = rows;
    void menu?.({
      anchor: open.anchor,
      ...modelRequest(s),
      update: true,
      refresh: true,
    });
  }, [
    s.settings.provider,
    s.settings.model,
    s.settings.effort,
    s.catalog.efforts,
    s.catalog.providers,
    s.catalog.models,
    n.connection,
    n.signedOut,
    n.findingModels,
    n.modelGroups,
    n.connectedModelName,
  ]);
  const sendLabel = working ? "Send direction" : "Send";
  // The shortcuts menu hangs from the composer, once per draft that asks
  // for it (the window's searchable menu); closing it unchosen leaves the
  // draft as typed.
  const composerNode = useRef<number | null>(null);
  const edit = (event: SurfaceEvent) => {
    const text = event.value;
    if (optimistic !== undefined || event.composing || typeof text !== "string") return;
    if (event.type === "change" && text === s.draft.text) return;
    const composerEpoch = event.composerEpoch ?? s.draft.sent;
    send({
      type: "draft",
      text,
      selection: event.selection ?? [text.length, text.length],
      base: s.draft.text,
      ...(composerEpoch === undefined ? {} : { composerEpoch }),
    });
  };
  const focused = useRef(false);
  const opened = useRef<string | null>(null);
  const slashDraft =
    s.slash.visible && s.slash.choices.length ? s.draft.text : null;
  useLayoutEffect(() => {
    if (slashDraft === null) {
      opened.current = null;
      return;
    }
    if (!menu || opened.current === slashDraft || composerNode.current === null)
      return;
    opened.current = slashDraft;
    void menu({
      anchor: composerNode.current,
      placeholder: "Search commands",
      options: s.slash.choices.map((c) => {
        const copy = n.slashCopy[c.id] ?? slashCopy(c.id, c.value, s);
        return { label: `${copy.label} — ${copy.description}`, value: c.id };
      }),
    }).then((id) => {
      if (typeof id === "string") send({ type: "chooseSlash", id });
    });
  }, [slashDraft]);
  // The "+" menu: what can go with the draft, then how far the agent may go
  // on its own (Approvals). Its rows are the window's composer menu.
  const full = s.attachments.length >= 6;
  const canAttach = !disabled;
  const canApprove = !!menu && !working && !s.composerDisabled;
  const openAdd = (anchor: number) => {
    awaiting.current = null;
    const epoch = ++menuEpoch.current;
    setOpenPicker("add");
    const options: MenuOption[] = [];
    if (canAttach && full)
      options.push({ label: "A message can carry six attachments", value: "", informational: true });
    else if (canAttach)
      options.push(
        { label: "Add Files…", value: "attach" },
        // v1's: drag a box anywhere over the editor; the agent gets what
        // was under it (the composition there, or the window).
        { label: "Select Part of the Screen", value: "pickRegion" },
      );
    if (canApprove)
      options.push(
        ...(Object.keys(ACCESS) as Access[]).map((value) => ({
          label: ACCESS[value].label,
          detail: ACCESS[value].detail,
          value,
          checked: value === s.settings.access,
          category: "Approvals",
        })),
      );
    void menu?.({ anchor, picker: "add", search: false, placeholder: "", options }).then((choice) => {
      if (menuEpoch.current !== epoch) return;
      setOpenPicker(null);
      if (choice === "attach") send({ type: "attach" });
      else if (choice === "pickRegion") send({ type: "pickRegion" });
      else if (typeof choice === "string" && choice in ACCESS && choice !== s.settings.access)
        send({ type: "setAccess", access: choice as Access });
    });
  };
  // The pill's effort part opens the reasoning menu: the model's levels in
  // order, ✓ on the one in use, on the model picker's card and rows.
  // Choosing one closes it and the pill reads the new level at once.
  const openEffort = (anchor: number) => {
    awaiting.current = null;
    const epoch = ++menuEpoch.current;
    setOpenPicker("effort");
    void menu?.({
      anchor,
      picker: "chooser",
      menu: "effort",
      placeholder: "",
      options: efforts.map((value) => ({
        kind: "effort",
        label: effortLabel(value),
        value,
        checked: value === effortNow,
      })),
    }).then((choice) => {
      if (menuEpoch.current !== epoch) return;
      setOpenPicker(null);
      if (typeof choice !== "string" || choice === effortNow) return;
      setPicked(choice);
      send({ type: "setModel", model: latest.current.settings.model, effort: choice });
    });
  };
  const ready = !disabled && !submitting && s.sendable;
  // Send and Stop are one circle at the row's end: a dim disc while there
  // is nothing to send, the text ink once there is (or while a run can be
  // stopped). With a draft during a run, Stop steps back beside Send.
  // Send and Stop are rounded rectangles of the pill's highlight box (its
  // height and corner), so the row reads as one family (no circles).
  function sendFace(lit: boolean): Style {
    return {
    ...row,
    justifyContent: "center",
    width: t("metrics.hit"),
    height: t("metrics.hit"),
    borderRadius: t("metrics.inner_radius"),
    background: t(lit ? "colors.text" : "colors.composer_send_idle"),
    color: t(lit ? (n.presentation === "popover" ? "colors.raised" : "colors.floating") : "colors.muted"),
    symbolSize: t("metrics.composer_symbol"),
    };
  }
  const hitBox: Style = {
    ...target,
    ...row,
    justifyContent: "center",
    flexShrink: 0,
    width: t("metrics.hit"),
    height: t("metrics.hit"),
    borderRadius: t("metrics.hit"),
  };
  const composer = (
    <div
      id="agent-composer"
      role="group"
      label="Message composer"
      // The chooser stands clear of the composer: its bounds are measured.
      metadata={{ menuAnchor: true }}
      state={{ dropTarget }}
      data-pm-drop="files"
      onDragOver={() => {
        if (acceptsFiles) setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        setDragging(false);
        if (acceptsFiles)
          send({
            type: "attachInput",
            paths: (e.drag?.data["public.file-url"] ?? []) as string[],
          });
      }}
      // One surface: the draft written straight on it, then under it one
      // row: "+", the model chooser, and Send at the end. Its corner follows
      // the panel's (or the popover's) corner it sits `text_padding` inside.
      style={{
        ...column,
        flexShrink: 0,
        background: surface,
        borderRadius: t(
          n.presentation === "popover"
            ? "metrics.composer_popover_radius"
            : "metrics.composer_radius",
        ),
        borderWidth: t("metrics.border"),
        borderColor: t("colors.hairline"),
        marginLeft: t("metrics.text_padding"),
        marginRight: t("metrics.text_padding"),
        marginBottom: t("metrics.text_padding"),
      }}
    >
      {(s.attachments.length > 0 || dropTarget || (s.draft.citations?.length ?? 0) > 0) && (
        <div
          style={{
            ...column,
            gap: t("metrics.gap"),
            paddingTop: t("space.md"),
            paddingLeft: t("space.md"),
            paddingRight: t("space.md"),
          }}
        >
          {s.attachments.length > 0 && (
            <div
              style={{
                ...row,
                alignItems: "start",
                flexWrap: "wrap",
                gap: t("metrics.gap"),
              }}
            >
              {s.attachments.map((a) => (
                <AttachmentTile
                  key={a.id}
                  a={a}
                  armed={s.draft.armed === a.id}
                  send={send}
                />
              ))}
            </div>
          )}
          <CitationChips key={s.threadId} s={s} send={send} />
          {dropTarget && (
            <text style={{ ...quiet, color: t("colors.accent_text") }}>
              Drop to attach
            </text>
          )}
        </div>
      )}
      <Field
        // Keep the same editor throughout a send. A thread change or a
        // new showing starts with that thread’s own draft.
        key={`${s.threadId}:composer-${shown}`}
        multiline
        id="agent-composer-text"
        label="Message Powermove agent"
        title="Return to send · Shift-Return for a new line · / for shortcuts"
        value={s.draft.text}
        disabled={fieldDisabled}
        metadata={{
          placeholder,
          submitOnReturn: true,
          ...(s.draft.sent === undefined ? {} : { composerEpoch: s.draft.sent }),
          ...(optimistic === undefined ? {} : { composerSending: optimistic }),
          attachmentProject: s.projectId,
          attachmentThread: s.threadId,
        }}
        state={{ keyboardFocus: n.keyboardFocus === "composer" }}
        tabIndex={0}
        onInput={(event) => {
          edit(event);
          composerNode.current = Number(event.currentTarget);
          if (!focused.current) {
            focused.current = true;
            send({ type: "focus", focused: true });
          }
        }}
        onChange={edit}
        onKeyDown={(event) => {
          event.stopPropagation();
          const action = composerKey(event, s.draft.sent);
          if (event.key === "Enter" && !event.shiftKey && !event.composing) {
            if (!event.repeat && !disabled) submit(action);
          } else {
            send(action);
          }
        }}
        style={{
          fontSize: t("typography.body"),
          color: t("colors.text"),
          background: surface,
          minWidth: t("metrics.hit"),
        }}
      />
      <div
        id="agent-options"
        // The row stands `space.xl` under the field (about 24 pt under the
        // draft's last line). Send's circle, centred in its hit target,
        // keeps 10 pt to the composer's trailing and bottom edges; "+"
        // lines up under the draft.
        style={{
          ...row,
          height: t("metrics.hit"),
          marginTop: t("space.xl"),
          marginBottom: t("space.md"),
          paddingLeft: t("space.sm"),
          paddingRight: t("space.md"),
          gap: t("space.xs"),
        }}
      >
        <button
          id="agent-add"
          role="button"
          tabIndex={0}
          label="Add"
          title="Add files or part of the screen, or choose approvals"
          disabled={!menu || !(canAttach || canApprove)}
          open={openPicker === "add"}
          aria-haspopup="menu"
          aria-expanded={openPicker === "add"}
          metadata={{ menuAnchor: true, symbol: "composer.add" }}
          preventDefault={{ keydown: ["Enter", " "] }}
          onClick={(e) => openAdd(Number(e.currentTarget))}
          onKeyDown={(e) => {
            if (e.repeat || e.metaKey || e.ctrlKey || e.altKey) return;
            if (e.key === "Enter" || e.key === " ") openAdd(Number(e.currentTarget));
          }}
          style={{
            ...hitBox,
            ...(openPicker === "add" ? { background: t("colors.selected_fill") } : {}),
            color: t("colors.secondary"),
            symbolSize: t("metrics.composer_add_symbol"),
          }}
        />
        <ModelChooser
          mark={
            signedOut
              ? s.settings.provider === "aero"
                ? "powermove"
                : null
              : harness
                ? providerMark(s.settings.provider, providerGroup(s.settings.provider, shownModel))
                : s.settings.provider
          }
          model={
            signedOut
              ? s.settings.provider === "aero"
                ? "Sign in to Powermove"
                : "Connect a provider"
              : harness
                ? chooserName(
                    s.catalog.models,
                    shownModel,
                    modelNamed(s.catalog.models, shownModel, named(s.catalog.providers, s.settings.provider)),
                  )
                : modelName
          }
          effort={!signedOut && efforts.length > 0 ? effortName : null}
          disabled={!menu || choicesLocked}
          opened={openPicker}
          openModels={openModels}
          openEffort={openEffort}
          signedOut={signedOut}
        />
        {aeroNeedsPlan(s, n) && (
          // Aero on Free: Checkout straight from the composer, a compact
          // accent pill beside the model's (the plan the backend offers).
          <button
            id="agent-subscribe"
            role="button"
            tabIndex={0}
            label="Subscribe"
            title="Aero needs Aerolite: subscribe in your browser"
            onClick={() => send({ type: "subscribe" })}
            style={{
              ...target,
              ...row,
              flexShrink: 0,
              height: t("metrics.hit"),
              paddingLeft: t("space.md"),
              paddingRight: t("space.md"),
              borderRadius: t("metrics.inner_radius"),
              background: t("colors.accent_text"),
              color: t("colors.floating"),
            }}
          >
            <text style={{ ...strong, color: t("colors.floating") }}>Subscribe</text>
          </button>
        )}
        <div style={{ flexGrow: 1, minWidth: t("space.xs") }} />
        {working && (
          <button
            id="agent-stop"
            role="button"
            label="Stop current run"
            title="Stop current run"
            tabIndex={0}
            disabled={s.stopping}
            onClick={stop}
            style={hitBox}
          >
            <div metadata={{ symbol: "composer.stop" }} style={sendFace(!hasDraft && !s.stopping)} />
          </button>
        )}
        {(!working || hasDraft) && (
          <button
            id="agent-send"
            role="button"
            label={sendLabel}
            title={sendLabel}
            tabIndex={0}
            disabled={disabled || submitting || !s.sendable}
            onClick={(event) => {
              const text = event.value;
              const composerEpoch = event.composerEpoch ?? s.draft.sent;
              if (!event.composing && !disabled && s.sendable)
                submit({
                  type: "submit",
                  ...(typeof text === "string" ? { text } : {}),
                  ...(composerEpoch === undefined ? {} : { composerEpoch }),
                });
            }}
            style={hitBox}
          >
            <div metadata={{ symbol: "composer.send" }} style={sendFace(ready)} />
          </button>
        )}
      </div>
    </div>
  );
  return composer;
}

// ── The panel ────────────────────────────────────────────────────────────

type LocalSend = { thread: string; row: Row; baseDraft: string; failed: boolean };

export function AgentView({
  publication: { snapshot: published, native: n },
  send,
  menu,
}: {
  publication: Publication;
  send: Send;
  /** The window's field menu (threads, run choices, access, shortcuts). */
  menu?: Menu;
}) {
  const pending = useRef(false);
  const requestedStop = useRef<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [submission, setSubmission] = useState<LocalSend>();
  const [stopPending, setStopPending] = useState(false);
  const localSend =
    submission?.thread === published.threadId &&
    published.runId === null &&
    !published.rows.items.some((row) => row.id === submission.row.id)
      ? submission
      : undefined;
  const optimistic = localSend !== undefined && !localSend.failed;
  let s = published;
  if (localSend) {
    s = {
      ...published,
      rows: {
        count: published.rows.count + 1,
        range: [published.rows.range[0], published.rows.range[1] + 1],
        items: [...published.rows.items, localSend.row],
      },
    };
    if (optimistic) {
      s.runId = localSend.row.id;
      s.draft = { ...published.draft, text: "", selection: [0, 0], armed: null, citations: [] };
      s.attachments = [];
      s.sendable = false;
      s.stopping = stopPending;
    } else if (localSend.baseDraft === published.draft.text) {
      const text = str(localSend.row.value.text);
      s.draft = { ...published.draft, text, selection: [text.length, text.length] };
    }
  }
  const waiting = optimistic || published.starting === true;
  const [delayed, setDelayed] = useState(false);
  useLayoutEffect(() => {
    setDelayed(false);
    if (waiting) {
      const timer = setTimeout(() => setDelayed(true), 300);
      return () => clearTimeout(timer);
    }
  }, [waiting, published.threadId]);
  useLayoutEffect(() => {
    if (requestedStop.current === published.threadId && published.runId !== null) {
      requestedStop.current = null;
      setStopPending(false);
      send({ type: "stop" });
    }
  }, [published.runId, published.threadId, send]);
  const stop = () => {
    if (s.stopping) return;
    if (optimistic) {
      requestedStop.current = published.threadId;
      setStopPending(true);
    } else {
      send({ type: "stop" });
    }
  };
  const submit = (action: Action) => {
    if (pending.current) return;
    const authored = action.type === "submit" ? action.text
      : action.type === "key" ? action.key.value : undefined;
    const text = typeof authored === "string" ? authored : published.draft.text;
    if (!text.trim() && published.attachments.length === 0) return;
    pending.current = true;
    requestedStop.current = null;
    setStopPending(false);
    setSubmitting(true);
    const id = published.nextMessageId ?? `history:message-${published.rows.count}`;
    const start = published.runId === null && !n.needsAnswer ? {
      thread: published.threadId,
      baseDraft: published.draft.text,
      row: { id, commandId: id, value: obj(json({
        role: "user", text, attachments: published.attachments,
        citations: published.draft.citations ?? [],
      })), chosen: {}, open: false },
      failed: false,
    } : undefined;
    setSubmission(start);
    const settle = (failed: boolean) => {
      pending.current = false;
      setSubmitting(false);
      if (failed) {
        requestedStop.current = null;
        setStopPending(false);
      }
      setSubmission(failed && start ? {
        ...start,
        failed: true,
        row: { ...start.row, value: { ...start.row.value, delivery: "failed" } },
      } : undefined);
    };
    try {
      void Promise.resolve(send(action)).then(() => settle(false), () => settle(true));
    } catch {
      settle(true);
    }
  };
  const [jump, setJump] = useState<
    (Citation & { nonce: number }) | undefined
  >();
  const jumpSerial = useRef(0);
  const jumpTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  useLayoutEffect(() => {
    setJump(undefined);
    return () => {
      if (jumpTimer.current !== undefined) clearTimeout(jumpTimer.current);
    };
  }, [s.threadId]);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [away, setAway] = useState(false);
  const [compactWelcome, setCompactWelcome] = useState(false);
  useLayoutEffect(() => setAway(false), [s.threadId]);
  const [jumps, setJumps] = useState(0);
  const newestUser = s.rows.items.reduce<string | undefined>(
    (last, item) => (item.value.role === "user" ? item.id : last),
    undefined,
  );
  const followed = useRef({ thread: s.threadId, user: newestUser, sends: 0 });
  if (followed.current.thread !== s.threadId) {
    followed.current = { thread: s.threadId, user: newestUser, sends: 0 };
  } else if (
    s.draft.sent === undefined &&
    s.rows.range[1] === s.rows.count &&
    newestUser &&
    followed.current.user !== newestUser
  ) {
    followed.current.user = newestUser;
    followed.current.sends++;
  }
  // Each change of where the panel is shown (the popover opening, a dock)
  // counts as a new showing.
  const presentation = useRef({ at: n.presentation, count: 0 });
  if (presentation.current.at !== n.presentation)
    presentation.current = {
      at: n.presentation,
      count: presentation.current.count + 1,
    };
  // Stream updates must not republish historical prose through context.
  // Stable callbacks read current draft/rows at the moment of the action.
  const latestCitationState = useRef({ s, send, open });
  latestCitationState.current = { s, send, open };
  const citationContext = useMemo(() => ({
    threadId: s.threadId,
    cite: (q: Citation) => {
      const { s, send } = latestCitationState.current;
      send({ type: "citations", citations: [...(s.draft.citations ?? []), q] });
    },
    jump,
    reveal: (q: Citation) => {
      const { s, open } = latestCitationState.current;
      const expanded = { ...open };
      for (const entry of entries(s)) {
        const items =
          entry.kind === "work"
            ? entry.items
            : entry.kind === "message"
              ? [entry.message]
              : [];
        for (const message of items) {
          if (
            entry.kind === "work" &&
            (message.rowId === q.messageId ||
              message.steps.some((row) => row.id === q.messageId))
          )
            expanded[entry.key] = true;
          for (const part of activity(message.steps))
            if (part.kind === "tools") {
              part.details.forEach((detail, i) => {
                if (detail.sourceId === q.messageId) {
                  expanded[part.key] = true;
                  expanded[`${part.key}:${detail.id}-${i}`] = true;
                  if (entry.kind === "work") expanded[entry.key] = true;
                }
              });
            }
        }
      }
      setOpen(expanded);
      setJump({ ...q, nonce: ++jumpSerial.current });
      if (jumpTimer.current !== undefined) clearTimeout(jumpTimer.current);
      jumpTimer.current = setTimeout(() => setJump(undefined), 1200);
    },
  }), [s.threadId, jump]);
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
  // While the connection is still being checked, what was shown stays
  // (the original keeps setup and composer from flashing).
  const gate = !connected && !working && n.connection !== "checking";
  // With the Powermove harness the panel keeps its normal empty state and
  // composer: signing in is the chooser's (Settings › Providers); only a
  // fault is reported here.
  const harness = harnessMode(s);
  const fault =
    gate && harness && n.connection === "unavailable" && !n.signedOut && !aeroNeedsPlan(s, n);
  // Nothing to keep yet: the connection is the whole panel.
  const setup = gate && !harness && empty && !s.draft.text && !s.attachments.length;
  const popover = n.presentation === "popover";
  const welcome = empty && !setup;
  const content = (
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
      <Header s={s} n={n} send={send} menu={menu} />
      <div
        style={{
          ...column,
          overflow: "hidden",
          flexGrow: 1,
          flexShrink: 1,
          position: "relative",
        }}
      >
        <div
          id="agent-conversation"
          role="group"
          label={setup ? "Agent setup" : "Agent conversation"}
          tabIndex={0}
          onScroll={(e) => {
            const { away, compact } = obj(e.value);
            if (typeof away === "boolean") setAway(away);
            if (typeof compact === "boolean") setCompactWelcome(compact);
          }}
          metadata={{
            count: s.rows.count,
            range: s.rows.range as unknown as Json,
            ...(welcome
              ? {
                  scrollAnchor: "start",
                  scrollToken: s.threadId,
                  scrollCompactBelow: 120,
                }
              : { scrollAnchor: "end", scrollToken: `${s.threadId}:${jumps + (s.draft.sent === undefined ? followed.current.sends : s.draft.sent + Number(optimistic))}` }),
          }}
          style={{
            ...column,
            ...target,
            overflow: "scroll",
            gap: t("metrics.agent_gap"),
            paddingLeft: t("metrics.agent_gap"),
            paddingRight: t("metrics.agent_gap"),
          }}
        >
          <div
            id="agent-conversation-content"
            style={{
              ...column,
              flexShrink: 0,
              gap: t("metrics.agent_gap"),
              ...(welcome
                ? {}
                : {
                    paddingTop: t("metrics.gap"),
                    paddingBottom: t("metrics.inset"),
                  }),
            }}
          >
            <SubthreadBar s={s} send={send} />
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
                <text
                  style={{ ...quiet, ...shrink, color: t("colors.warning") }}
                >
                  Thread history couldn’t be saved. Keep this window open.
                </text>
              </div>
            )}
            {gate && !harness && <Gate s={s} n={n} send={send} compact={!setup} />}
            {fault && (
              <ErrorNotice
                raw={n.reason || `${named(s.catalog.providers, s.settings.provider)} couldn’t start.`}
                fallbackTitle={`${named(s.catalog.providers, s.settings.provider)} couldn’t start`}
                retry={() => send({ type: "retryConnection" })}
                id="agent-connect-error"
              />
            )}
            {!setup &&
              (n.loadingHistory ? (
                <UI.ProgressRow label="Loading earlier messages…" />
              ) : null)}
            {!setup &&
              (empty ? (
                <Welcome s={s} n={n} send={send} compact={compactWelcome} />
              ) : (
                <Conversation
                  s={s}
                  send={send}
                  local={local}
                  starting={waiting ? delayed ? "delayed" : "waiting" : null}
                  retryLocal={localSend?.failed ? {
                    id: localSend.row.id,
                    run: () => submit({
                      type: "submit",
                      text: published.draft.text || str(localSend.row.value.text),
                      ...(published.draft.sent === undefined ? {} : { composerEpoch: published.draft.sent }),
                    }),
                  } : undefined}
                />
              ))}
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
                <text
                  style={{ ...quiet, ...shrink, color: t("colors.danger") }}
                >
                  Couldn’t stop this task.
                </text>
              </div>
            )}
            {!setup && <PlanPreview s={s} send={send} />}
          </div>
        </div>
        {!empty && !setup && away && (
          <div
            style={{
              ...row,
              justifyContent: "center",
              position: "absolute",
              left: 0,
              right: 0,
              bottom: 0,
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
      </div>
      {!setup &&
        harness &&
        s.settings.provider === "aero" &&
        !aeroNeedsPlan(s, n) &&
        !n.aero?.outOfCredits &&
        n.aero?.lowCredits && (
          <div
            id="agent-low-credits"
            role="status"
            label={aeroLowCreditsCopy(n.aero.lowCredits)}
            style={{
              ...row,
              flexShrink: 0,
              gap: t("metrics.gap"),
              paddingLeft: t("metrics.agent_gap"),
              paddingRight: t("metrics.agent_gap"),
              paddingBottom: t("space.xs"),
            }}
          >
            <div
              metadata={{ symbol: "exclamationmark.triangle.fill" }}
              style={{
                color: t(n.aero.lowCredits.level === "critical" ? "colors.danger" : "colors.warning"),
                symbolSize: t("metrics.symbol_inline"),
                flexShrink: 0,
              }}
            />
            <text
              style={{
                ...quiet,
                ...shrink,
                color: t(n.aero.lowCredits.level === "critical" ? "colors.danger" : "colors.warning"),
              }}
            >
              {aeroLowCreditsCopy(n.aero.lowCredits)}
            </text>
          </div>
        )}
      {!setup && (
        <Composer
          key={s.threadId}
          s={s}
          n={n}
          send={send}
          submit={submit}
          stop={stop}
          submitting={submitting}
          optimistic={optimistic && localSend ? str(localSend.row.value.text) : undefined}
          menu={menu}
          shown={presentation.current.count}
        />
      )}
    </div>
  );
  return (
    <CitationContext.Provider value={citationContext}>
      {content}
    </CitationContext.Provider>
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
        const dispatch = model.dispatch(action);
        void dispatch.catch(onError);
        return dispatch;
      }}
    />
  );
}
const namedKeys: Record<string, string> = {
  backspace: "Backspace",
  delete: "Delete",
  tab: "Tab",
};
export function composerKey(event: SurfaceEvent, currentEpoch?: number): Action {
  const { key, metaKey, ctrlKey, shiftKey, altKey, repeat, composing, value } = event;
  const composerEpoch = event.composerEpoch ?? currentEpoch;
  return {
    type: "key",
    ...(composerEpoch === undefined ? {} : { composerEpoch }),
    key: {
      ...(typeof value === "string" ? { value } : {}),
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
