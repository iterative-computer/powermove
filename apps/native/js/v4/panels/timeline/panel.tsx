import { useEffect, useState } from "preact/hooks";
import { memo } from "../../src/memo";
import type { ComponentChildren } from "preact";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import type {
  Intent,
  LayerRow,
  Modifiers,
  PropertyRow,
  Row,
  TimelineFrame,
} from "./model";
import { announcementText, type Announcement } from "./model";
import { nativeEvent, type TimelineSession } from "./session";

type Send = (intent: Intent) => void;
const t = UI.token;
const reading = {
  color: t("colors.secondary"),
  fontSize: t("typography.reading"),
};
const line = { display: "flex", flexDirection: "row", alignItems: "center" };
/** Leading cluster + trailing cluster: replaces the flexible-spacer placeholder
 * (design review 2026-11-01; the validator accepts space-between). */
const split = { ...line, justifyContent: "space-between" };
const text = { color: t("colors.text"), fontSize: t("typography.body") };
const modifiers = (e: SurfaceEvent): Modifiers => ({
  shiftKey: !!e.shiftKey,
  altKey: !!e.altKey,
  metaKey: !!e.metaKey,
  ctrlKey: !!e.ctrlKey,
});
function action(send: Send, intent: Intent) {
  return (event: SurfaceEvent) => {
    event.stopPropagation();
    send(intent);
  };
}
function Icon(props: {
  id: string;
  label: string;
  title?: string;
  symbol: string;
  send: Send;
  intent: Intent;
  disabled?: boolean;
  active?: boolean;
  primary?: boolean;
  /** A row's small mark (its disclosure): the inline glyph in the muted ink. */
  quiet?: boolean;
  /** The glyph's own ink (a token name), for a mark that carries state in
   * its colour instead of a lit ground. */
  tint?: string;
  tabIndex?: number;
  key?: string;
  children?: ComponentChildren;
}) {
  // DESIGN-TODO(opus) T1: kit has no symbol slot. This is a neutral, labelled
  // control with inspectable required symbol, not a text/emoji icon substitute.
  return (
    <UI.ToolbarItem
      id={props.id}
      label={props.label}
      title={props.title ?? props.label}
      metadata={{ requiredSymbol: props.symbol, designTodo: "T1" }}
      disabled={props.disabled}
      active={props.active}
      tabIndex={props.tabIndex}
      style={{
        symbolSize: t(
          props.quiet ? "metrics.symbol_inline" : "metrics.symbol_panel",
        ),
        ...(props.primary ? { color: t("colors.text") } : {}),
        ...(props.quiet ? { color: t("colors.muted") } : {}),
        ...(props.tint ? { color: t(props.tint) } : {}),
      }}
      onClick={(event) => {
        event.stopPropagation();
        if (!props.disabled) props.send(props.intent);
      }}
    >
      {props.children}
    </UI.ToolbarItem>
  );
}
/** Refused edits need visible feedback; confirmations do not. */
function refusal(value: Announcement): boolean {
  return (
    value.kind === "keyCollision" ||
    value.kind === "outsideComposition" ||
    value.kind === "locked"
  );
}
/** The graph view the switch returns to: the last one shown (v1 keeps "Show
 * Layer Bars with Graph" apart from the switch itself). */
let lastGraphView: "Graph" | "Combined" = "Graph";
/** v1's tab row: one tab per open composition (the active one named in the
 * text ink over an accent rule), and the Layers / Tracks switch at its end. */
function Header({ frame: f, send }: { frame: TimelineFrame; send: Send }) {
  return (
    <div
      id="timeline:header"
      data-pm-slot="header"
      style={{
        ...split,
        alignItems: "stretch",
        height: t("metrics.header"),
        paddingLeft: t("metrics.gap"),
        paddingRight: t("metrics.gap"),
        borderBottom: t("metrics.border"),
        borderColor: t("colors.separator"),
      }}
    >
      <div
        id="timeline:tabs"
        label="Open compositions"
        role="tablist"
        style={{ ...line, alignItems: "stretch" }}
      >
        {f.hasComposition &&
          f.tabs.map((tab) => (
            <div
              key={tab.id}
              style={{
                ...line,
                paddingLeft: t("metrics.gap"),
                paddingRight: t("metrics.gap"),
                // The active tab stands on v1's 2 pt accent rule (the
                // hairline step, as a pill's inset); the others keep the
                // same height without one.
                ...(tab.active
                  ? {
                      borderBottom: t("metrics.pill_inset"),
                      borderColor: t("colors.accent"),
                    }
                  : { paddingBottom: t("metrics.pill_inset") }),
              }}
            >
              <UI.ToolbarItem
                id={`timeline:tab:${tab.id}`}
                label={tab.name}
                title={tab.name}
                role="tab"
                tabIndex={tab.active ? 0 : -1}
                aria-selected={tab.active}
                metadata={{ selectionMark: "rule" }}
                onClick={action(send, {
                  type: "tab",
                  id: tab.id,
                  close: false,
                })}
                style={{
                  color: t(tab.active ? "colors.text" : "colors.muted"),
                  fontSize: t("typography.reading"),
                  ...(tab.active
                    ? { fontWeight: t("typography.medium_weight") }
                    : {}),
                }}
              >
                {tab.name}
              </UI.ToolbarItem>
              {f.tabs.length > 1 && tab.showClose && (
                <Icon
                  id={`timeline:tab-close:${tab.id}`}
                  label={`Close ${tab.name}`}
                  title={`Close ${tab.name} (⌘W)`}
                  symbol="xmark"
                  quiet
                  tabIndex={-1}
                  send={send}
                  intent={{ type: "tab", id: tab.id, close: true }}
                />
              )}
            </div>
          ))}
      </div>
      {f.hasComposition && (
        <div style={line}>
          <UI.SegmentedControl
            id="timeline:mode"
            label="Timeline mode"
            value={f.mode}
            options={[
              { id: "Layers", label: "Layers" },
              { id: "Tracks", label: "Tracks" },
            ]}
            onChange={(mode) =>
              send({ type: "mode", mode: mode as "Layers" | "Tracks" })
            }
            style={{ fontSize: t("typography.body") }}
          />
        </div>
      )}
    </div>
  );
}
/** v1's head: the corner over the layer column, beside the ruler. Play and
 * the time lead; the graph switch (its options while shown, and Tracks'
 * snapping) sits at the column's edge. */
function Head({ frame: f, send }: { frame: TimelineFrame; send: Send }) {
  if (f.view !== "DopeSheet") lastGraphView = f.view;
  const graph = f.view !== "DopeSheet";
  return (
    <div
      id="timeline:head"
      style={{
        ...split,
        height: t("metrics.ruler"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.gap"),
      }}
    >
      <div style={{ ...line, gap: t("metrics.gap") }}>
        <Icon
          id="timeline:play"
          label={f.playing ? "Pause" : "Play"}
          title="Play / Pause (Space)"
          symbol={f.playing ? "pause" : "play"}
          send={send}
          intent={{ type: "play" }}
        />
        {f.timeEditor ? (
          <UI.TextField
            id="timeline:time"
            label="Current time"
            value={f.timeEditor.value}
            binding={f.timeEditor.binding}
            metadata={{ cancelChange: true }}
            onChange={(e) =>
              send({ type: "editDone", value: String(e.value ?? "") })
            }
            style={{
              color: t("colors.accent_text"),
              fontSize: t("typography.timecode"),
              fontWeight: t("typography.medium_weight"),
              fontVariantNumeric: "tabular-nums",
            }}
          />
        ) : (
          <UI.ToolbarItem
            id="timeline:time"
            label="Current time"
            title={`Current time (${f.frameRate})`}
            value={f.spokenTime}
            onClick={action(send, { type: "timeEdit" })}
            metadata={{ fixedWidthSample: "00:00:00:00" }}
            style={{
              color: t("colors.accent_text"),
              fontSize: t("typography.timecode"),
              fontWeight: t("typography.medium_weight"),
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {/* The native timecode layer draws the time in this fixed-width
                slot, paused and playing: the panel hears play start and stop
                only after the transport has moved (S3), so handing the time
                back and forth showed two readouts, or none, for a frame. */}
            {""}
          </UI.ToolbarItem>
        )}
      </div>
      <div style={line}>
        {graph && (
          <Icon
            id="timeline:graph-options"
            label="Graph Options"
            symbol="ellipsis"
            send={send}
            intent={{ type: "graphOptions" }}
          />
        )}
        <Icon
          id="timeline:graph"
          label="Graph Editor"
          title="Graph Editor"
          symbol="point.topleft.down.to.point.bottomright.curvepath"
          active={graph}
          tint={graph ? "colors.accent_text" : undefined}
          send={send}
          intent={{ type: "view", view: graph ? "DopeSheet" : lastGraphView }}
        />
        {(f.shyLayers || f.hideShy) && (
          <Icon
            id="timeline:hide-shy"
            label="Hide Shy Layers"
            symbol={f.hideShy ? "face.dashed.fill" : "face.dashed"}
            active={f.hideShy === true}
            title="Hide Shy Layers"
            send={send}
            intent={{ type: "hideShy" }}
          />
        )}
        {f.mode === "Tracks" && (
          <Icon
            id="timeline:snap"
            label="Snapping"
            symbol="arrow.right.and.line.vertical.and.arrow.left"
            active={f.snapping}
            tint={f.snapping ? "colors.accent_text" : undefined}
            title="Snap Clips to Edges, Markers and the Playhead"
            send={send}
            intent={{ type: "snap" }}
          />
        )}
      </div>
    </div>
  );
}
/** Exact timeline §2 glyph table; unknown kinds remain neutral until schema
 * presentation supplies a symbol. */
export function kindSymbol(kind: string): string | null {
  return (
    (
      {
        text: "textformat",
        shape: "square.on.circle",
        video: "film",
        image: "photo",
        audio: "waveform",
        adjustment: "slider.horizontal.3",
        null: "circle.dashed",
        camera: "camera",
        group: "square.stack",
        precomp: "rectangle.stack",
        captions: "captions.bubble",
        solid: "square.fill",
        shader: "wand.and.rays",
        extension: "wand.and.rays",
        model3d: "cube",
        light3d: "lightbulb",
        camera3d: "camera",
      } as Record<string, string>
    )[kind] ?? null
  );
}
function Layer({
  row,
  send,
  single,
}: {
  row: LayerRow;
  send: Send;
  single: boolean;
}) {
  // Unset switches appear under the pointer, keyboard focus or a single
  // selected layer, so a multi-selection never squeezes every name.
  const show = row.hovered || row.focused || (row.selected && single);
  // In column order, the last at the row's trailing edge.
  const switches = [
    ...(row.transformable
      ? [
          {
            name: "parent" as const,
            on: row.parent.set,
            label: `Parent: ${row.parent.name} · Drag to a layer`,
            symbol: "link",
            active: undefined as boolean | undefined,
          },
        ]
      : []),
    {
      name: "shy" as const,
      on: row.shy === true,
      label: "Toggle Shy",
      symbol: row.shy ? "face.dashed.fill" : "face.dashed",
      active: row.shy === true,
    },
    {
      name: "solo" as const,
      on: row.solo,
      label: "Toggle Solo",
      symbol: row.solo ? "s.circle.fill" : "s.circle",
      active: row.solo,
    },
    {
      name: "lock" as const,
      on: row.locked,
      label: "Toggle Lock",
      symbol: row.locked ? "lock" : "lock.open",
      active: undefined,
    },
    {
      name: "visibility" as const,
      on: !row.visible,
      label: "Toggle Visibility",
      symbol: row.visible ? "eye" : "eye.slash",
      active: undefined,
    },
  ];
  return (
    <>
      <div
        style={{
          ...line,
          gap: t("metrics.gap"),
          overflow: "hidden",
          paddingLeft: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        }}
      >
        {row.index !== null && (
          <text
            style={{
              color: t("colors.muted"),
              fontSize: t("typography.minimum"),
              fontVariantNumeric: "tabular-nums",
              flexShrink: 0,
            }}
          >
            {String(row.index).padStart(2, "0")}
          </text>
        )}
        {row.canOpen ? (
          <Icon
            id={`timeline:disclosure:${row.id}`}
            label={`${row.expanded ? "Collapse" : "Expand"} ${row.label}`}
            symbol={row.expanded ? "chevron.down" : "chevron.right"}
            quiet
            tabIndex={-1}
            send={send}
            intent={{ type: "disclosure", id: row.id, open: !row.expanded }}
          />
        ) : (
          // The disclosure column stays reserved so badges and names align.
          <div
            role="presentation"
            style={{
              width: t("metrics.hit"),
              minWidth: t("metrics.hit"),
              flexShrink: 0,
              height: t("metrics.hit"),
            }}
          />
        )}
        <div
          role="presentation"
          attr={{ symbol: row.layerKind ? kindSymbol(row.layerKind) : null }}
          metadata={{
            recipe: "kind-badge",
            designTodo: row.layerKind ? null : "native-schema-kind",
          }}
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            width: t("metrics.kind_badge"),
            minWidth: t("metrics.kind_badge"),
            flexShrink: 0,
            height: t("metrics.kind_badge"),
            borderRadius: t("metrics.inner_radius"),
            background: t(
              row.labelColor
                ? `colors.labels.${row.labelColor}`
                : "colors.field",
            ),
            color: t("colors.text"),
            symbolSize: t("metrics.symbol_inline"),
          }}
        />
        {row.nameEditor ? (
          <UI.TextField
            label={row.label}
            value={row.nameEditor.value}
            binding={row.nameEditor.binding}
            tabIndex={-1}
            metadata={{ cancelChange: true }}
            onChange={(e) =>
              send({ type: "editDone", value: String(e.value ?? "") })
            }
          />
        ) : (
          <text
            title={row.label}
            style={{
              ...text,
              flexGrow: 1,
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
            onDoubleClick={action(send, { type: "rename", id: row.id })}
          >
            {row.label}
          </text>
        )}
      </div>
      <div style={line}>
        {switches.map((item, i) =>
          item.on || show ? (
            <Icon
              key={item.name}
              id={`timeline:${item.name}:${row.id}`}
              label={item.label}
              symbol={item.symbol}
              active={item.active}
              tabIndex={-1}
              send={send}
              intent={{ type: "switch", id: row.id, name: item.name }}
            />
          ) : switches.slice(0, i).some((before) => before.on) ? (
            // Each switch keeps its own column: a set one further from the
            // edge leaves the columns after it open.
            <div
              key={item.name}
              role="presentation"
              style={{
                width: t("metrics.hit"),
                minWidth: t("metrics.hit"),
                flexShrink: 0,
                height: t("metrics.hit"),
              }}
            />
          ) : null,
        )}
      </div>
    </>
  );
}
function Property({ row, send }: { row: PropertyRow; send: Send }) {
  // The key navigator stands in the columns a layer's number, disclosure
  // and badge take, so a property's name starts where its layer's does.
  const step = (direction: "previous" | "next") =>
    row.animated ? (
      <Icon
        id={`timeline:${direction}:${row.id}`}
        label={
          direction === "previous"
            ? "Previous Keyframe (⇧J)"
            : "Next Keyframe (⇧K)"
        }
        symbol={direction === "previous" ? "chevron.left" : "chevron.right"}
        quiet
        disabled={!row[direction]}
        tabIndex={-1}
        send={send}
        intent={{
          type: "adjacentKey",
          layer: row.layer,
          path: row.path,
          direction,
        }}
      />
    ) : (
      <div
        role="presentation"
        style={{
          width: t("metrics.hit"),
          minWidth: t("metrics.hit"),
          flexShrink: 0,
          height: t("metrics.hit"),
        }}
      />
    );
  return (
    <>
      <div style={{ ...line, paddingLeft: t("metrics.gap") }}>
        {step("previous")}
        <Icon
          id={`timeline:diamond:${row.id}`}
          label={row.keyAtPlayhead ? "Remove Keyframe" : "Add Keyframe"}
          symbol={
            row.keyAtPlayhead
              ? "diamond.fill"
              : row.animated
                ? "diamond.lefthalf.filled"
                : "diamond"
          }
          quiet
          tint={row.animated ? "colors.accent" : "colors.muted"}
          disabled={row.locked}
          tabIndex={-1}
          send={send}
          intent={{ type: "keyToggle", layer: row.layer, path: row.path }}
        />
        {step("next")}
        <text
          title={row.path}
          style={{
            color: t("colors.secondary"),
            fontSize: t("typography.body"),
          }}
        >
          {row.label}
        </text>
      </div>
      <div style={{ ...line, paddingRight: t("metrics.gap") }}>
        {/* The link stands before the value: values end on one edge. */}
        {row.scaleLinked !== undefined && (
          <Icon
            id={`timeline:scale-link:${row.id}`}
            label={
              row.scaleLinked ? "Unlock Aspect Ratio" : "Lock Aspect Ratio"
            }
            // One glyph (there is no slashed link in the system's set: it
            // drew nothing); the state is its ink.
            symbol="link"
            tint={row.scaleLinked ? "colors.secondary" : "colors.faint"}
            disabled={row.locked}
            tabIndex={-1}
            send={send}
            intent={{ type: "scaleLink", layer: row.layer }}
          />
        )}
        {row.numeric && row.scaleLinked === undefined ? (
          <UI.NumberField
            id={`timeline:value:${row.id}`}
            label={row.label}
            {...row.numeric}
            disabled={row.locked}
            tabIndex={-1}
          />
        ) : (
          <text
            label={row.accessibilityLabel}
            metadata={{
              designTodo: "T5",
              timelineScrub: { layer: row.layer, path: row.path },
            }}
            style={{
              color: t(row.locked ? "colors.muted" : "colors.text"),
              fontSize: t("typography.body"),
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {row.readout}
          </text>
        )}
      </div>
    </>
  );
}
const TimelineRow = memo(
  function TimelineRow({
    row: published,
    send,
    single,
  }: {
    row: Row;
    send: Send;
    single: boolean;
    key?: string;
  }) {
    let clickModifiers: Modifiers = {};
    // Hover is the pointer's, local to the row: switches show without a host trip.
    const [hover, setHover] = useState(false);
    const row =
      hover && !published.hovered ? { ...published, hovered: true } : published;
    return (
      <UI.TreeItem
        id={`timeline:row:${row.id}`}
        data-pm-variant="band"
        label={row.accessibilityLabel}
        selected={row.selected}
        tabIndex={-1}
        expanded={row.kind === "property" ? undefined : row.expanded}
        state={{
          locked: row.locked,
          focused: row.focused,
          hovered: row.hovered,
        }}
        metadata={{
          nativeRow: row.id,
          depth: row.depth,
          designTodo: "T4",
          ...(row.kind === "layer" ? { media: row.media } : {}),
        }}
        onClickCapture={(event) => {
          clickModifiers = modifiers(event);
        }}
        onPointerEnter={() => setHover(true)}
        onPointerLeave={() => setHover(false)}
        onSelect={() => {
          send({ type: "selectRow", id: row.id, modifiers: clickModifiers });
          clickModifiers = {};
        }}
        // Child controls stop click bubbling; the outline remains the sole row Tab stop.
        style={{
          ...split,
          height: t(
            row.kind === "property"
              ? "metrics.property_row"
              : "metrics.track_row",
          ),
          ...(row.hovered && !row.selected
            ? { background: t("colors.hover") }
            : {}),
        }}
      >
        {row.kind === "layer" ? (
          <Layer row={row} send={send} single={single} />
        ) : row.kind === "property" ? (
          <Property row={row} send={send} />
        ) : (
          <>
            <div
              style={{
                ...line,
                gap: t("metrics.gap"),
                overflow: "hidden",
                paddingLeft: t("metrics.inset"),
              }}
            >
              <text
                style={{
                  ...text,
                  fontWeight: t("typography.medium_weight"),
                  fontVariantNumeric: "tabular-nums",
                  flexShrink: 0,
                }}
              >
                {row.label}
              </text>
              {row.summary && (
                <text
                  title={row.summary}
                  style={{
                    color: t("colors.secondary"),
                    fontSize: t("typography.body"),
                    flexGrow: 1,
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                  }}
                >
                  {row.summary}
                </text>
              )}
            </div>
            <div style={{ ...line, paddingRight: t("metrics.gap") }}>
              <Icon
                id={`timeline:track-enable:${row.id}`}
                label={row.accessibilityLabel}
                symbol={
                  row.audio
                    ? row.enabled
                      ? "speaker.wave.2"
                      : "speaker.slash"
                    : row.enabled
                      ? "eye"
                      : "eye.slash"
                }
                tabIndex={-1}
                send={send}
                intent={{
                  type: "switch",
                  id: row.id,
                  name: row.audio ? "mute" : "visibility",
                }}
              />
              <Icon
                id={`timeline:track-lock:${row.id}`}
                label="Toggle Lock"
                symbol={row.locked ? "lock" : "lock.open"}
                tabIndex={-1}
                send={send}
                intent={{ type: "switch", id: row.id, name: "lock" }}
              />
            </div>
          </>
        )}
      </UI.TreeItem>
    );
  },
  (before, after) =>
    before.send === after.send &&
    before.single === after.single &&
    JSON.stringify(before.row) === JSON.stringify(after.row),
);

/** Provisional kit-only surface. Missing timeline recipes are explicit, neutral
 * placeholders, never substitute CSS/canvas implementations or new product UI.
 */
export function TimelinePanel({
  frame: f,
  send,
}: {
  frame: TimelineFrame;
  send: Send;
}) {
  return (
    <div
      id="timeline"
      label="Timeline"
      role="group"
      metadata={{
        document: f.document,
        revision: f.revision,
        feedback: f.feedback,
      }}
      style={{
        background: t("colors.panel"),
        borderRadius: t("metrics.panel_radius"),
      }}
    >
      <Header frame={f} send={send} />
      {!f.hasComposition ? (
        <UI.EmptyState
          label="No Composition"
          explanation="Open a project to start editing."
          metadata={{ requiredSymbol: "film.stack", designTodo: "T1" }}
          action={
            <Icon
              id="timeline:open"
              label="Open…"
              symbol=""
              send={send}
              intent={{ type: "open" }}
            >
              <text>Open…</text>
            </Icon>
          }
        />
      ) : (
        <>
          <div
            id="timeline:body"
            style={{ position: "relative", flexGrow: 1, overflow: "hidden" }}
          >
            <canvas
              id="timeline:surface"
              surface="timeline"
              label="Timeline tracks"
              style={{ position: "absolute", inset: "fill" }}
            />
            <div
              id="timeline:outline"
              metadata={{ nativeColumn: true }}
              style={{
                // The head over the ruler's band above the rows, the bottom
                // bar's below.
                paddingBottom: t("metrics.ruler"),
                width: t("metrics.layer_column"),
                flexGrow: 1,
              }}
            >
              <Head frame={f} send={send} />
              <div
                style={{
                  position: "relative",
                  flexGrow: 1,
                  overflow: "hidden",
                }}
              >
                <UI.Tree
                  id="timeline:tracks"
                  label={f.mode}
                  tabIndex={0}
                  style={{
                    minWidth: t("metrics.hit"),
                    minHeight: t("metrics.hit"),
                  }}
                  metadata={{
                    totalRows: f.window.total,
                    firstRow: f.window.first,
                    indexDigits: f.indexDigits,
                    nativeFirstRow: f.window.rows[0]?.id ?? null,
                    nativeWindow: [f.window.first, f.window.rows.length],
                  }}
                  onKeyDown={(event) => {
                    event.stopPropagation();
                    send({
                      type: "keyDown",
                      event: nativeEvent(event),
                    });
                  }}
                  onRangeChange={(event) => {
                    if (event.range && event.range[1] > event.range[0])
                      send({
                        type: "range",
                        first: event.range[0],
                        visible: event.range[1] - event.range[0],
                      });
                  }}
                >
                  {f.window.rows.map((row) => (
                    <TimelineRow
                      key={row.id}
                      row={row}
                      send={send}
                      single={!f.selection || f.selection.count <= 1}
                    />
                  ))}
                  {f.window.total === 0 && (
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "center",
                        justifyContent: "center",
                      }}
                    >
                      <text style={reading}>
                        Drag media here or import from your computer.
                      </text>
                      <Icon
                        id="timeline:import"
                        label="Import…"
                        symbol=""
                        tabIndex={-1}
                        send={send}
                        intent={{ type: "import" }}
                      >
                        <text>Import…</text>
                      </Icon>
                    </div>
                  )}
                </UI.Tree>
              </div>
            </div>
            {f.announcement && refusal(f.announcement.value) && (
              // A refused edit stays visible: a quiet capsule over the tracks,
              // also announced (role="status").
              <text
                id="timeline:announcement"
                role="status"
                label={announcementText(f.announcement.value)}
                metadata={{ sequence: f.announcement.sequence }}
                style={{
                  position: "absolute",
                  left: t("metrics.inset"),
                  bottom: t("metrics.inset"),
                  paddingLeft: t("metrics.gap"),
                  paddingRight: t("metrics.gap"),
                  height: t("metrics.field"),
                  display: "flex",
                  alignItems: "center",
                  borderRadius: t("metrics.control_radius"),
                  background: t("colors.floating"),
                  color: t("colors.text"),
                  fontSize: t("typography.minimum"),
                }}
              >
                {announcementText(f.announcement.value)}
              </text>
            )}
          </div>
          <text id="timeline:render-bar" label={f.renderSummary} />
          {f.selection && f.selection.count > 1 && (
            // Spoken, not drawn: the rows and rings already show the selection,
            // and the inspector shows the count (as v1 does).
            <text
              id="timeline:selection"
              role="status"
              label={`${f.selection.count} ${f.selection.kind === "layers" ? "layers" : "keyframes"} selected`}
            />
          )}
          {f.announcement && !refusal(f.announcement.value) && (
            // Confirmations (marker added/removed) are spoken only.
            <text
              id="timeline:announcement"
              role="status"
              label={announcementText(f.announcement.value)}
              metadata={{ sequence: f.announcement.sequence }}
            />
          )}
        </>
      )}
    </div>
  );
}

/** Session activation is explicit and awaited by the worker before mounting.
 * An unready session has no invented loading copy or fabricated empty project.
 */
export function ConnectedTimeline({ session }: { session: TimelineSession }) {
  const [frame, setFrame] = useState(session.frame);
  useEffect(() => session.subscribe(setFrame), [session]);
  return frame ? <TimelinePanel frame={frame} send={session.send} /> : null;
}
