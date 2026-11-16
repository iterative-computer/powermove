import { createContext } from "preact";
import { useContext, useEffect, useState } from "preact/hooks";
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

const TimelineFocused = createContext(false);

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
  square?: boolean;
  /** A row's small mark (its disclosure): the inline glyph in the muted ink. */
  quiet?: boolean;
  /** The glyph's own ink (a token name), for a mark that carries state in
   * its colour instead of a lit ground. */
  tint?: string;
  tabIndex?: number;
  key?: string;
  children?: ComponentChildren;
  parentPick?: string;
}) {
  // DESIGN-TODO(opus) T1: kit has no symbol slot. This is a neutral, labelled
  // control with inspectable required symbol, not a text/emoji icon substitute.
  return (
    <UI.ToolbarItem
      id={props.id}
      label={props.label}
      title={props.title ?? props.label}
      metadata={{
        requiredSymbol: props.symbol,
        designTodo: "T1",
        ...(props.parentPick
          ? { timelineParentPick: { layer: props.parentPick } }
          : {}),
      }}
      disabled={props.disabled}
      active={props.active}
      tabIndex={props.tabIndex}
      style={{
        symbolSize: t(
          props.quiet ? "metrics.symbol_inline" : "metrics.symbol_panel",
        ),
        ...(props.square
          ? {
              width: t("metrics.hit"),
              height: t("metrics.hit"),
            }
          : {}),
        ...(props.square && props.active
          ? { background: t("colors.selected_fill") }
          : {}),
        ...(props.primary ? { color: t("colors.text") } : {}),
        ...(props.quiet ? { color: t("colors.muted") } : {}),
        ...(props.tint ? { color: t(props.tint) } : {}),
      }}
      onClick={(event) => {
        event.stopPropagation();
        if (!props.disabled)
          props.send(
            props.intent.type === "disclosure"
              ? { ...props.intent, all: !!event.altKey }
              : props.intent,
          );
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
/** One stable pill; the reserved close slot never changes the label width. */
function CompositionTab({
  tab,
  closable,
  send,
}: {
  key?: string;
  tab: TimelineFrame["tabs"][number];
  closable: boolean;
  send: Send;
}) {
  const [hovered, setHovered] = useState(false);
  const context = (event: SurfaceEvent) => {
    event.stopPropagation();
    send({ type: "tabContext", id: tab.id });
  };
  return (
    <div
      id={`timeline:tab:${tab.id}`}
      label={tab.name}
      title={tab.name}
      metadata={{ compositionTab: true }}
      role="tab"
      tabIndex={tab.active ? 0 : -1}
      aria-selected={tab.active}
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
      onPointerDown={(event) => {
        if (event.button === 2 || (event.button === 0 && event.ctrlKey))
          context(event);
      }}
      onClick={(event) => {
        event.stopPropagation();
        if (
          !event.ctrlKey &&
          (event.button === undefined || event.button === 0)
        )
          send({ type: "tab", id: tab.id, close: false });
      }}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (event.key === "Enter" || event.key === " ") {
          event.stopPropagation();
          send({ type: "tab", id: tab.id, close: false });
        }
      }}
      style={{
        ...line,
        flexShrink: 0,
        height: t("metrics.comp_tab_height"),
        minHeight: t("metrics.comp_tab_height"),
        minWidth: t("metrics.comp_tab_height"),
        paddingLeft: t("metrics.comp_tab_leading"),
        paddingRight: t("metrics.comp_tab_trailing"),
        gap: t("metrics.comp_tab_content_gap"),
        borderRadius: t("metrics.control_radius"),
        color: t(tab.active ? "colors.text" : "colors.secondary"),
        ...(tab.active ? { background: t("colors.selected_fill") } : {}),
        fontSize: t("typography.reading"),
      }}
    >
      <text
        style={{
          maxWidth: t("metrics.toolbar_title_width"),
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {tab.name}
      </text>
      {closable && (
        <UI.ToolbarItem
          id={`timeline:tab-close:${tab.id}`}
          label={`Close ${tab.name}`}
          title={`Close ${tab.name}`}
          tabIndex={-1}
          aria-hidden={!tab.active && !hovered}
          metadata={{
            requiredSymbol: "xmark",
            compositionTabClose: {
              tab: `timeline:tab:${tab.id}`,
              active: tab.active,
              visible: tab.active || hovered,
            },
          }}
          style={{
            width: t("metrics.comp_tab_close"),
            height: t("metrics.comp_tab_close"),
            minWidth: t("metrics.comp_tab_close"),
            minHeight: t("metrics.comp_tab_close"),
            paddingLeft: t("metrics.border"),
            paddingRight: t("metrics.border"),
            symbolSize: t("metrics.symbol_inline"),
            color: t("colors.muted"),
            borderRadius: t("metrics.inner_radius"),
          }}
          onPointerDown={(event) => {
            event.stopPropagation();
            if (event.button === 2 || event.ctrlKey) context(event);
          }}
          onClick={(event) => {
            event.stopPropagation();
            if (
              !event.ctrlKey &&
              (event.button === undefined || event.button === 0)
            )
              send({ type: "tab", id: tab.id, close: true });
          }}
        />
      )}
    </div>
  );
}
/** v1's tab row: one tab per open composition (the active one in the text
 * ink on the kit's selected ground, a raised neutral cell: the accent marks
 * the playhead alone), and the Layers / Tracks switch at its end. */
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
      }}
    >
      <div
        id="timeline:tabs"
        label="Open compositions"
        role="tablist"
        // Tabs sit centred in the bar at a control's height, clear of the
        // panel's top edge and of the row below.
        style={{
          ...line,
          minWidth: t("metrics.comp_tab_gap"),
          flexShrink: 1,
          overflow: "scroll",
          gap: t("metrics.comp_tab_gap"),
        }}
      >
        {f.hasComposition &&
          f.tabs.map((tab) => (
            <CompositionTab
              key={tab.id}
              tab={tab}
              closable={f.tabs.length > 1}
              send={send}
            />
          ))}
      </div>
      {f.hasComposition && (
        <div style={line}>
          <UI.SegmentedControl
            id="timeline:mode"
            label="Timeline mode"
            data-pm-variant="plate"
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
          symbol={f.playing ? "pause.fill" : "play.fill"}
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
              color: t("colors.text"),
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
              color: t("colors.text"),
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
          title={graph ? "Hide Graph Editor (⇧F3)" : "Show Graph Editor (⇧F3)"}
          symbol="point.topleft.down.to.point.bottomright.curvepath"
          active={graph}
          primary
          square
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
  // As v1: every row keeps its switches' columns at its trailing edge, so
  // names truncate at the same place on every row and nothing moves under
  // the pointer. Visibility and the parent link always show; Lock shows
  // when set, or under the pointer, keyboard focus or a single selected
  // layer. Shy and Solo stay reachable on each layer row.
  const focusedSelection = useContext(TimelineFocused) && row.selected;
  const show = row.hovered || row.focused || (row.selected && single);
  // In column order, the last at the row's trailing edge.
  const switches = [
    { name: "shy" as const, on: !!row.shy, label: "Toggle Shy", symbol: "face.dashed", active: row.shy, always: true },
    { name: "solo" as const, on: row.solo, label: "Toggle Solo", symbol: "s.circle", active: row.solo, always: true },
    ...(row.transformable
      ? [
          {
            name: "parent" as const,
            on: row.parent.set,
            label: `Parent: ${row.parent.name} · Drag to a layer`,
            symbol: "link",
            active: undefined as boolean | undefined,
            always: true,
          },
        ]
      : []),
    {
      name: "lock" as const,
      on: row.locked,
      label: "Toggle Lock",
      symbol: row.locked ? "lock" : "lock.open",
      active: undefined,
      always: false,
    },
    {
      name: "visibility" as const,
      on: !row.visible,
      label: "Toggle Visibility",
      symbol: row.visible ? "eye" : "eye.slash",
      active: undefined,
      always: true,
    },
  ];
  return (
    <>
      <div
        style={{
          ...line,
          // The rename field takes the room the switches leave (the name
          // itself keeps to its text, which a double-click renames).
          ...(row.nameEditor ? { flexGrow: 1 } : {}),
          gap: t("metrics.gap"),
          // The name gives way first: this side shrinks, the switches never.
          flexShrink: 1,
          overflow: "hidden",
          paddingLeft: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        }}
      >
        {row.index !== null && (
          <text
            style={{
              color: t(focusedSelection ? "colors.accent_text" : "colors.muted"),
              fontSize: t("typography.minimum"),
              fontVariantNumeric: "tabular-nums",
              flexShrink: 0,
            }}
          >
            {String(row.index).padStart(2, "0")}
          </text>
        )}
        <div
          role="presentation"
          metadata={{ timelineIndent: row.depth }}
          style={{
            display: "flex",
            flexDirection: "row",
            flexShrink: 0,
          }}
        >
          {Array.from({ length: Math.min(4, row.depth) }, (_, depth) => (
            <div
              key={depth}
              role="presentation"
              style={{ width: t("metrics.timeline_indent"), flexShrink: 0 }}
            />
          ))}
          {row.canOpen ? (
            <Icon
              id={`timeline:disclosure:${row.id}`}
              label={`${row.expanded ? "Collapse" : "Expand"} ${row.label}`}
              symbol={row.expanded ? "chevron.down" : "chevron.right"}
              quiet
              tint={focusedSelection ? "colors.accent_text" : undefined}
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
        </div>
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
            color: t(focusedSelection ? "colors.accent_text" : "colors.text"),
            symbolSize: t("metrics.symbol_inline"),
          }}
        />
        {row.nameEditor ? (
          // The field takes the name's place across the column while it
          // edits (a field in a row grows; the cluster gives it the room).
          <UI.TextField
            // Its own identity: its commit is carried to the newest tree
            // when the panel has published another on the way.
            id={`timeline:rename:${row.id}`}
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
            // A stable identity: the double-click that renames lands after
            // the first click's selection has re-rendered the row, and the
            // host carries an event to the newest tree only by its id.
            id={`timeline:name:${row.id}`}
            title={row.label}
            // v1: dragging a name reorders the layer (the native surface
            // takes the gesture past its threshold; a click still selects).
            metadata={{ timelineLayerDrag: { layer: row.layer } }}
            style={{
              ...text,
              ...(focusedSelection ? { color: t("colors.accent_text") } : {}),
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
      <div style={{ ...line, flexShrink: 0 }}>
        {switches.map((item) =>
          item.on || item.always || show ? (
            <Icon
              key={item.name}
              parentPick={item.name === "parent" ? row.layer : undefined}
              id={`timeline:${item.name}:${row.id}`}
              label={item.label}
              symbol={item.symbol}
              tint={focusedSelection ? "colors.accent_text" : undefined}
              active={item.active}
              tabIndex={-1}
              send={send}
              intent={{ type: "switch", id: row.id, name: item.name }}
            />
          ) : (
            // An unset switch keeps its column, so the name never reflows.
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
          ),
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
      <div
        style={{
          ...line,
          width: t("metrics.value_column"),
          flexShrink: 0,
          gap: t("metrics.gap"),
          paddingRight: t("metrics.gap"),
        }}
      >
        {/* v1 propertyValueColumns: equal axis columns, then the Scale link. */}

        {row.numeric && row.scaleLinked === undefined ? (
          <UI.NumberField
            id={`timeline:value:${row.id}`}
            label={row.label}
            {...row.numeric}
            binding={
              typeof row.numeric.binding === "string"
                ? row.numeric.binding
                : row.numeric.binding.handle
            }
            disabled={row.locked}
            tabIndex={-1}
          />
        ) : (
          (row.axes ?? [{ path: row.path, readout: row.readout }]).map(
            (axis) =>
              axis.numeric ? (
                <UI.NumberField
                  id={`timeline:value:${row.id}:${axis.path}`}
                  label={`${row.label} ${axis.path.split(".").at(-1)}`}
                  {...axis.numeric}
                  binding={
                    typeof axis.numeric.binding === "string"
                      ? axis.numeric.binding
                      : axis.numeric.binding.handle
                  }
                  disabled={row.locked}
                  tabIndex={-1}
                />
              ) : axis.editor ? (
                <UI.TextField
                  id={`timeline:value-edit:${row.id}:${axis.path}`}
                  label={`${row.label} ${axis.path.split(".").at(-1)}`}
                  value={axis.editor.value}
                  binding={axis.editor.binding}
                  style={{
                    flexGrow: 1,
                    flexBasis: t("metrics.hit"),
                    minWidth: t("metrics.hit"),
                  }}
                  tabIndex={-1}
                  metadata={{ cancelChange: true }}
                  onChange={(e) =>
                    send({ type: "editDone", value: String(e.value ?? "") })
                  }
                />
              ) : (
                <text
                  id={`timeline:value:${row.id}:${axis.path}`}
                  label={`${row.label} ${axis.path.split(".").at(-1)}, ${axis.readout}`}
                  metadata={
                    row.locked
                      ? {}
                      : {
                          timelineScrub: { layer: row.layer, path: axis.path },
                        }
                  }
                  style={{
                    color: t(row.locked ? "colors.muted" : "colors.text"),
                    fontSize: t("typography.body"),
                    fontVariantNumeric: "tabular-nums",
                    flexGrow: 1,
                    flexBasis: t("metrics.hit"),
                    minWidth: t("metrics.hit"),
                    overflow: "hidden",
                  }}
                >
                  {axis.readout}
                </text>
              ),
          )
        )}
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
    const [hover, setHover] = useState(false);
    const timelineFocused = useContext(TimelineFocused);
    if (published.kind === "heading")
      return (
        <div
          id={`timeline:heading:${published.id}`}
          role="row"
          label={published.label}
          style={{
            ...line,
            height: t("metrics.property_row"),
            paddingLeft: `${published.depth * 16 + 12}px`,
          }}
        >
          <text
            style={{ color: t("colors.muted"), fontSize: t("typography.body") }}
          >
            {published.label}
          </text>
        </div>
      );
    let clickModifiers: Modifiers = {};
    // Hover is the pointer's, local to the row: switches show without a host trip.
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
          ...(row.kind === "layer"
            ? {
                media: row.media,
                // The native host derives the drag source from nativeRow.
                // An open field explicitly opts out until rename is finished.
                ...(row.nameEditor ? { timelineLayerDrag: null } : {}),
              }
            : {}),
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
          ...(row.selected && timelineFocused
            ? {
                background: t(
                  row.hovered
                    ? "colors.timeline_selected_hover"
                    : "colors.timeline_selected",
                ),
                color: t("colors.accent_text"),
              }
            : row.selected
              ? { background: t("colors.fill_strong") }
              : row.hovered
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
                flexGrow: 1,
                flexShrink: 1,
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
            <div
              style={{
                ...line,
                flexShrink: 0,
                paddingRight: t("metrics.gap"),
              }}
            >
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
  const [focused, setFocused] = useState(false);
  return (
    <TimelineFocused.Provider value={focused}>
    <div
      id="timeline"
      onPointerDownCapture={() => setFocused(true)}
      onFocus={() => setFocused(true)}
      onBlur={(event) => setFocused(typeof event.value === "string" && event.value.startsWith("timeline"))}
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
                </UI.Tree>
                {f.window.total === 0 && (
                  // An empty composition: the whole layer area, centred,
                  // inset from its edges (the playhead never enters it).
                  // The sentence wraps rather than touch the divider; a
                  // short panel clips the block below the ruler.
                  <div
                    id="timeline:empty"
                    style={{
                      position: "absolute",
                      inset: "fill",
                      display: "flex",
                      flexDirection: "column",
                      alignItems: "center",
                      justifyContent: "center",
                      gap: t("metrics.inset"),
                      padding: t("metrics.inset"),
                      overflow: "hidden",
                    }}
                  >
                    <text
                      style={{
                        ...reading,
                        textAlign: "center",
                        whiteSpace: "normal",
                      }}
                    >
                      Drag media here or import from your computer.
                    </text>
                    <UI.PushButton
                      id="timeline:import"
                      label="Import…"
                      tabIndex={-1}
                      onClick={action(send, { type: "import" })}
                    />
                  </div>
                )}
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
    </TimelineFocused.Provider>
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
