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
  tabIndex?: number;
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
        symbolSize: t("metrics.symbol_panel"),
        ...(props.primary ? { color: t("colors.text") } : {}),
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
function Header({ frame: f, send }: { frame: TimelineFrame; send: Send }) {
  return (
    <div
      id="timeline:header"
      style={{
        ...split,
        height: t("metrics.header"),
        paddingLeft: t("metrics.inset"),
        paddingRight: t("metrics.inset"),
      }}
    >
      <div style={{ ...line, gap: t("metrics.gap") }}>
        <Icon
          id="timeline:play"
          label={f.playing ? "Pause" : "Play"}
          title="Play / Pause (Space)"
          symbol={f.playing ? "pause.fill" : "play.fill"}
          disabled={!f.hasComposition}
          primary
          send={send}
          intent={{ type: "play" }}
        />
        {f.hasComposition && (
          <>
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
                value={f.spokenTime}
                onClick={action(send, { type: "timeEdit" })}
                metadata={{ designTodo: "T2", fixedWidthSample: "00:00:00:00" }}
                style={{
                  color: t("colors.text"),
                  fontSize: t("typography.timecode"),
                  fontWeight: t("typography.medium_weight"),
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {f.timecode}
              </UI.ToolbarItem>
            )}
            <text
              style={{
                color: t("colors.muted"),
                fontSize: t("typography.minimum"),
              }}
            >
              {f.frameRate}
            </text>
            {f.tabs.length > 0 && (
              <div
                id="timeline:header-separator"
                role="presentation"
                style={{
                  width: t("metrics.border"),
                  height: t("metrics.symbol_panel"),
                  background: t("colors.separator"),
                }}
              />
            )}
            {f.tabs.length > 0 && (
              <div
                id="timeline:tabs"
                label="Open compositions"
                role="tablist"
                metadata={{ designTodo: "T3" }}
                style={line}
              >
                {f.tabs.map((tab) => (
                  <div key={tab.id} style={line}>
                    {f.tabs.length > 1 && tab.showClose && (
                      <Icon
                        id={`timeline:tab-close:${tab.id}`}
                        label={`Close ${tab.name}`}
                        title={`Close ${tab.name} (⌘W)`}
                        symbol="xmark"
                        tabIndex={-1}
                        send={send}
                        intent={{ type: "tab", id: tab.id, close: true }}
                      />
                    )}
                    <UI.ToolbarItem
                      id={`timeline:tab:${tab.id}`}
                      label={tab.name}
                      title={tab.name}
                      role="tab"
                      tabIndex={tab.active ? 0 : -1}
                      aria-selected={tab.active}
                      onClick={action(send, {
                        type: "tab",
                        id: tab.id,
                        close: false,
                      })}
                      style={{
                        color: t(
                          tab.active ? "colors.text" : "colors.secondary",
                        ),
                        fontSize: t("typography.body"),
                      }}
                    >
                      {tab.name}
                    </UI.ToolbarItem>
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </div>
      {f.hasComposition && (
        <div style={{ ...line, gap: t("metrics.gap") }}>
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
            style={{
              fontSize: t("typography.minimum"),
              fontWeight: t("typography.medium_weight"),
            }}
          />
          <UI.SegmentedControl
            id="timeline:view"
            label="Timeline view"
            value={f.view}
            options={[
              {
                id: "DopeSheet",
                label: "Dope Sheet",
                symbol: "chart.bar.doc.horizontal",
              },
              {
                id: "Combined",
                label: "Dope Sheet and Graph",
                symbol: "rectangle.split.1x2",
              },
              {
                id: "Graph",
                label: "Graph Editor",
                symbol: "point.topleft.down.to.point.bottomright.curvepath",
              },
            ]}
            onChange={(view) =>
              send({
                type: "view",
                view: view as "DopeSheet" | "Combined" | "Graph",
              })
            }
          />
          {f.view !== "DopeSheet" && (
            <Icon
              id="timeline:graph-options"
              label="Graph Options"
              symbol="ellipsis.circle"
              send={send}
              intent={{ type: "graphOptions" }}
            />
          )}
          <Icon
            id="timeline:snap"
            label="Snapping"
            symbol="magnet"
            active={f.snapping}
            title="Snap Clips to Edges, Markers and the Playhead"
            send={send}
            intent={{ type: "snap" }}
          />
        </div>
      )}
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
      } as Record<string, string>
    )[kind] ?? null
  );
}
function Layer({ row, send }: { row: LayerRow; send: Send }) {
  // Unset switches appear under the pointer or keyboard focus only, so a
  // selection never squeezes every layer name behind four icons.
  const show = row.hovered || row.focused;
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
        {row.transformable && (row.parent.set || show) && (
          <Icon
            id={`timeline:parent:${row.id}`}
            label={`Parent: ${row.parent.name} · Drag to a layer`}
            symbol="link"
            tabIndex={-1}
            send={send}
            intent={{ type: "switch", id: row.id, name: "parent" }}
          />
        )}
        {(row.solo || show) && (
          <Icon
            id={`timeline:solo:${row.id}`}
            label="Toggle Solo"
            symbol={row.solo ? "s.circle.fill" : "s.circle"}
            active={row.solo}
            tabIndex={-1}
            send={send}
            intent={{ type: "switch", id: row.id, name: "solo" }}
          />
        )}
        {(row.locked || show) && (
          <Icon
            id={`timeline:lock:${row.id}`}
            label="Toggle Lock"
            symbol={row.locked ? "lock" : "lock.open"}
            tabIndex={-1}
            send={send}
            intent={{ type: "switch", id: row.id, name: "lock" }}
          />
        )}
        {(!row.visible || show) && (
          <Icon
            id={`timeline:visibility:${row.id}`}
            label="Toggle Visibility"
            symbol={row.visible ? "eye" : "eye.slash"}
            tabIndex={-1}
            send={send}
            intent={{ type: "switch", id: row.id, name: "visibility" }}
          />
        )}
      </div>
    </>
  );
}
function Property({ row, send }: { row: PropertyRow; send: Send }) {
  return (
    <>
      <div style={line}>
        {row.animated && (
          <Icon
            id={`timeline:previous:${row.id}`}
            label="Previous Keyframe (⇧J)"
            symbol="chevron.left"
            disabled={!row.previous}
            tabIndex={-1}
            send={send}
            intent={{
              type: "adjacentKey",
              layer: row.layer,
              path: row.path,
              direction: "previous",
            }}
          />
        )}
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
          active={row.animated}
          disabled={row.locked}
          tabIndex={-1}
          send={send}
          intent={{ type: "keyToggle", layer: row.layer, path: row.path }}
        />
        {row.animated && (
          <Icon
            id={`timeline:next:${row.id}`}
            label="Next Keyframe (⇧K)"
            symbol="chevron.right"
            disabled={!row.next}
            tabIndex={-1}
            send={send}
            intent={{
              type: "adjacentKey",
              layer: row.layer,
              path: row.path,
              direction: "next",
            }}
          />
        )}
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
      <div style={line}>
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
        {row.scaleLinked !== undefined && (
          <Icon
            id={`timeline:scale-link:${row.id}`}
            label={
              row.scaleLinked ? "Unlock Aspect Ratio" : "Lock Aspect Ratio"
            }
            symbol={row.scaleLinked ? "link" : "link.slash"}
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
  }: {
    row: Row;
    send: Send;
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
          <Layer row={row} send={send} />
        ) : row.kind === "property" ? (
          <Property row={row} send={send} />
        ) : (
          <>
            <text style={text}>{row.label}</text>
            <div style={line}>
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
                paddingTop: t("metrics.ruler"),
                width: t("metrics.layer_column"),
                flexGrow: 1,
              }}
            >
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
                    <TimelineRow key={row.id} row={row} send={send} />
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
            {f.view !== "DopeSheet" && f.graphEmpty && (
              <text
                id="timeline:graph"
                style={{
                  ...reading,
                  position: "absolute",
                  right: t("metrics.inset"),
                  bottom: t("metrics.inset"),
                }}
              >
                Select an animated property to see its curve.
              </text>
            )}
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
