import { useEffect, useState } from "preact/hooks";
import {
  EmptyState,
  PopUpButton,
  PushButton,
  ToolbarItem,
  token,
} from "../../src/components";
import type { SurfaceProps } from "../../src/jsx-runtime";
import type { Json } from "../../src/protocol";
import {
  channelViews,
  effectiveResolution,
  percent,
  resolutionTitle,
  resolutions,
  zoomPresets,
  zoomTitle,
  type ViewerSnapshot,
} from "./contract";

export interface ViewerActions {
  /** Pop-up writes are handled once by the native binding adapter. This callback
   * is for worker/headless change delivery; native hosts must not echo a write. */
  drive: (element: string, argument?: Json) => void;
  open: () => void;
  /** Runs one of the app's commands (`play`, `gotoStart`, `toggleLoop`…). */
  command: (name: string) => void;
}
const popupStyle: SurfaceProps["style"] = {
  background: null,
  height: token("metrics.field"),
  color: token("colors.secondary"),
  fontSize: token("typography.body"),
  borderRadius: token("metrics.control_radius"),
};
const target = {
  minWidth: token("metrics.hit"),
  minHeight: token("metrics.hit"),
};

const row = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
} as const;
/** The transport, the app's own commands: the same Play the timeline's header
 * and the Space bar run, so either shows the other's state at once. */
const transportButtons = [
  {
    id: "gotoStart",
    label: "Go to Start",
    symbol: "backward.end.fill",
    key: "Home",
  },
  {
    id: "prevFrame",
    label: "Previous Frame",
    symbol: "backward.frame.fill",
    key: "←",
  },
  { id: "play", label: "Play", symbol: "play.fill", key: "Space" },
  {
    id: "nextFrame",
    label: "Next Frame",
    symbol: "forward.frame.fill",
    key: "→",
  },
  { id: "gotoEnd", label: "Go to End", symbol: "forward.end.fill", key: "End" },
] as const;
function Transport({
  playing,
  looping,
  command,
}: {
  playing: boolean;
  looping: boolean;
  command: (name: string) => void;
}) {
  return (
    <div id="viewer.transport" role="group" label="Transport" style={row}>
      {transportButtons.map((b) => {
        const play = b.id === "play";
        const label = play && playing ? "Pause" : b.label;
        return (
          <ToolbarItem
            key={b.id}
            id={`viewer.transport.${b.id}`}
            label={label}
            title={`${label} (${b.key})`}
            metadata={{ symbol: play && playing ? "pause.fill" : b.symbol }}
            style={{
              ...target,
              symbolSize: token("metrics.symbol_panel"),
              color: token(play ? "colors.text" : "colors.secondary"),
            }}
            onClick={() => command(b.id)}
          />
        );
      })}
      <ToolbarItem
        id="viewer.transport.loop"
        label="Loop Playback"
        title="Loop Playback"
        active={looping}
        metadata={{ symbol: "repeat" }}
        style={{ ...target, symbolSize: token("metrics.symbol_panel") }}
        onClick={() => command("toggleLoop")}
      />
    </div>
  );
}

/** Built-in viewer v4 worker tree. Unsupported media/gizmo/overlay visuals stay
 * neutral placeholders; metadata is inspection data, never a rendering recipe.
 * All output is provisional until Opus review. See README for exact section gates.
 */
export function ViewerPanel({
  state,
  actions,
}: {
  state: ViewerSnapshot;
  actions: ViewerActions;
}) {
  const m = state.model;
  const source = state.host.source;
  const transport = state.host.transport;
  if (!m)
    return (
      <div id="viewer" role="group" label="Viewer">
        <EmptyState
          label="No Composition"
          explanation="Open a project to start editing."
          symbol="film"
          action={
            // The state's one action: the real push button.
            <PushButton id="viewer.open" label="Open…" onClick={actions.open} />
          }
        />
      </div>
    );
  // 3D compositions add no toolbar (3d-studio.md §2): looking around is the
  // viewer's own gesture. While away from the shot the stage is the editor
  // view: the zoom slot names it, and one control, top-left, returns.
  const free = !source && m.studio?.look === "free";
  const zoomValue = m.fit ? "fit" : m.shownZoom;
  // The native pop-up shows its selected row, so Fit names its zoom.
  const zoomOptions = [
    { v: "fit", label: m.fit ? zoomTitle(m) : "Fit" },
    ...zoomPresets.map((v) => ({ v, label: `${percent(v)} %` })),
    ...[
      ["viewer.zoom.in", "Zoom In"],
      ["viewer.zoom.out", "Zoom Out"],
      ["actual", "Actual Size"],
      ["viewer.fit", "Fit Composition"],
    ].map(([v, label]) => ({ v, label })),
  ];
  const effective = effectiveResolution(m);
  const spoken = (text: string) => text.replace(/ %$/, " percent");
  // D3: an unnamed composition is spoken as "Untitled Composition"; the authored
  // empty name is never rewritten.
  const value = `${m.name || "Untitled Composition"}, ${percent(m.shownZoom)} percent, ${effective ? `${spoken(effective)} resolution, ` : ""}${m.selectionCount} ${m.selectionCount === 1 ? "layer" : "layers"} selected`;
  // The same two pop-ups (same ids) live in the docked bar or, in Focus and
  // full screen, in the ViewerOverlay capsule (viewer-chrome.md §2 Overlay),
  // so keyboard focus follows them across presentations (design review D5).
  const channels = state.host.channels ?? "viewer.showRGB";
  const popups = (
    <>
      <PopUpButton
        id="viewer.channels"
        data-pm-variant="inline"
        binding="viewer.channels"
        label={`Channels, ${channelViews.find((c) => c.v === channels)?.label ?? "RGB"}`}
        title="Channels"
        value={channels}
        options={channelViews}
        style={popupStyle}
        onChange={(event) => {
          if (channelViews.some((c) => c.v === event.value))
            actions.command(String(event.value));
        }}
      >
        {channelViews.find((c) => c.v === channels)?.label ?? "RGB"}
      </PopUpButton>
      {free ? (
        <text
          id="viewer.zoom"
          label="Editor View"
          style={{
            color: token("colors.text"),
            fontSize: token("typography.body"),
            whiteSpace: "nowrap",
          }}
        >
          Editor View
        </text>
      ) : (
        <PopUpButton
          id="viewer.zoom"
          data-pm-variant="inline"
          binding="viewer.zoom"
          label={`Zoom, ${m.fit ? "Fit, " : ""}${percent(m.shownZoom)} percent`}
          title="Zoom"
          value={zoomValue}
          display={zoomTitle(m)}
          widthSample="Fit (100 %)"
          options={zoomOptions}
          style={popupStyle}
          onChange={(event) => {
            if (event.value === "fit") actions.drive("viewer.fit");
            else if (event.value === "actual") actions.drive("viewer.zoom", 1);
            else if (
              typeof event.value === "number" &&
              zoomPresets.includes(event.value as (typeof zoomPresets)[number])
            )
              actions.drive("viewer.zoom", event.value);
            else if (
              ["viewer.zoom.in", "viewer.zoom.out", "viewer.fit"].includes(
                String(event.value),
              )
            )
              actions.drive(String(event.value));
          }}
        >
          {zoomTitle(m)}
        </PopUpButton>
      )}
      <PopUpButton
        id="viewer.quality"
        data-pm-variant="inline"
        binding="viewer.quality"
        label={`Preview resolution, ${m.quality === "auto" ? `Auto${effective ? `, currently ${spoken(effective)}` : ""}` : resolutionTitle(m)}`}
        title="Preview Resolution"
        value={m.quality}
        options={resolutions}
        style={popupStyle}
        onChange={(event) => {
          if (resolutions.some((o) => o.v === event.value))
            actions.drive("viewer.quality", event.value);
        }}
      >
        {resolutionTitle(m)}
      </PopUpButton>
    </>
  );
  // 3d-studio.md §3 status capsule: Quality's progress, bottom-trailing over
  // the image. Hidden in Fast and while Live; "Clean" shows briefly, then goes.
  const refine = m.studio?.tier === "refine" ? m.studio?.refine : undefined;
  const [clean, setClean] = useState(false);
  const stage = refine?.stage;
  useEffect(() => {
    if (stage !== "complete") {
      setClean(false);
      return;
    }
    setClean(true);
    const timer = setTimeout(() => setClean(false), 2000);
    return () => clearTimeout(timer);
  }, [stage, refine?.samplesTarget]);
  const percentDone =
    refine && refine.samplesTarget > 0
      ? Math.min(99, Math.floor((refine.samplesDone / refine.samplesTarget) * 100))
      : 0;
  // Before the first sample lands (a heavy scene can take a while) the
  // capsule says so, rather than leave a dark stage that looks broken.
  const pending =
    m.studio?.tier === "refine" &&
    (m.studio?.pending === true || (stage === "sampling" && refine?.samplesDone === 0));
  const capsuleText =
    stage === "sampling" && refine && refine.samplesDone > 0
      ? `Refining · ${percentDone} %`
      : stage === "denoising"
        ? m.studio?.denoiser === "guided"
          ? "Refining · Built-in denoiser"
          : "Refining · Denoising"
        : stage === "complete" && clean
          ? "Clean"
          : pending
            ? "Rendering…"
            : null;
  const capsule = capsuleText ? (
    <div
      id="viewer.studio.status"
      role="status"
      label={capsuleText}
      style={{
        position: "absolute",
        right: token("metrics.inset"),
        bottom: token("metrics.inset"),
        height: token("metrics.field"),
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        gap: token("metrics.inner_radius"),
        paddingLeft: token("metrics.inset"),
        paddingRight: token("metrics.inset"),
        background: token("colors.floating"),
        borderRadius: token("metrics.pill_radius"),
      }}
    >
      <text style={{ color: token("colors.secondary"), fontSize: token("typography.body"), fontVariantNumeric: "tabular-nums" }}>
        {capsuleText === "Clean" ? "✓ Clean" : capsuleText}
      </text>
    </div>
  ) : null;
  const viewCamera = free ? (
    <div
      id="viewer.studio.return"
      role="group"
      label="Editor View"
      style={{
        position: "absolute",
        left: token("metrics.inset"),
        top: token("metrics.inset"),
        height: token("metrics.field"),
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        paddingLeft: token("metrics.inner_radius"),
        paddingRight: token("metrics.inner_radius"),
        background: token("colors.floating"),
        borderRadius: token("metrics.pill_radius"),
      }}
    >
      <ToolbarItem
        id="viewer.studio.viewCamera"
        label="View camera"
        title={"View camera\nBack to the shot. Looking around never moves it."}
        metadata={{ symbol: "camera" }}
        style={{
          color: token("colors.text"),
          fontSize: token("typography.body"),
        }}
        onClick={() => actions.drive("viewer.studio.look", "camera")}
      >
        View camera
      </ToolbarItem>
    </div>
  ) : null;
  return (
    <div
      id="viewer"
      role="group"
      label="Viewer"
      style={{
        background: token("colors.panel"),
        borderRadius: token("metrics.panel_radius"),
      }}
    >
      {state.host.presentation === "Docked" && (
        // One bar: the time at the leading edge, the transport in the middle,
        // the view's own controls at the trailing edge. The leading and
        // trailing groups share the room equally, so the transport stays
        // centred on the panel.
        <div
          id="viewer.bar"
          data-pm-slot="header"
          style={{
            ...row,
            height: token("metrics.header"),
            gap: token("metrics.gap"),
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.gap"),
          }}
        >
          <div
            style={{
              ...row,
              flexGrow: 1,
              flexBasis: token("metrics.value_column"),
              minWidth: token("metrics.hit"),
              gap: token("metrics.gap"),
            }}
          >
            {source ? (
              <>
                <text
                  style={{
                    color: token("colors.muted"),
                    fontSize: token("typography.body"),
                  }}
                >
                  Source:
                </text>
                <text
                  style={{
                    color: token("colors.text"),
                    fontSize: token("typography.body"),
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {source.name}
                </text>
                <ToolbarItem
                  id="viewer.source.close"
                  label="Back to Composition"
                  metadata={{ symbol: "chevron.left" }}
                  style={{
                    color: token("colors.accent_text"),
                    fontSize: token("typography.body"),
                  }}
                  onClick={() => actions.drive("viewer.source.close")}
                >
                  Back to Composition
                </ToolbarItem>
              </>
            ) : (
              transport && (
                <>
                  {/* While playing, the native timecode layer draws the time in
                      this fixed-width slot every frame (panels hear only play
                      start and stop), as in the timeline's header. */}
                  <text
                    id="viewer.time"
                    label="Current time"
                    value={transport.spokenTime}
                    metadata={{ fixedWidthSample: "00:00:00:00" }}
                    style={{
                      color: token("colors.text"),
                      fontSize: token("typography.timecode"),
                      fontWeight: token("typography.medium_weight"),
                      fontVariantNumeric: "tabular-nums",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {transport.playing ? "" : transport.timecode}
                  </text>
                  <text
                    id="viewer.duration"
                    style={{
                      color: token("colors.muted"),
                      fontSize: token("typography.body"),
                      fontVariantNumeric: "tabular-nums",
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {transport.duration}
                  </text>
                </>
              )
            )}
          </div>
          {transport && !source && (
            <Transport
              playing={transport.playing}
              looping={transport.looping}
              command={actions.command}
            />
          )}
          <div
            style={{
              ...row,
              flexGrow: 1,
              flexBasis: token("metrics.value_column"),
              minWidth: token("metrics.hit"),
              justifyContent: "end",
              gap: token("metrics.gap"),
            }}
          >
            {popups}
          </div>
        </div>
      )}
      <div
        id="viewer.canvas"
        role="group"
        label="Viewer"
        value={value}
        tabIndex={0}
        style={{
          ...target,
          position: "relative",
          flexGrow: 1,
        }}
        state={{ interaction: state.host.interaction }}
        metadata={{
          // DESIGN-TODO(opus): map spec geometry/keylines/opacity to named
          // pm-design tokens; the kit has no viewer canvas/overlay recipe yet.
          frame: state.host.frame,
          source,
          presentation: state.host.presentation,
          empty: m.empty,
          selectionCount: m.selectionCount,
          showControls: m.showControls,
          canvasLost: !source && m.canvasLost,
          pendingRecipe: "viewer-chrome.md §§2–3,6–7",
        }}
      >
        <canvas
          id="viewer.overlays"
          surface="viewer.overlays"
          label="Extension overlays"
          style={{ position: "absolute", inset: "fill" }}
        />
        <canvas
          id="viewer.gizmos"
          surface="viewer.gizmos"
          label="Transform gizmos"
          style={{ position: "absolute", inset: "fill" }}
        />
        {viewCamera}
        {capsule}
      </div>
      {!source && m.canvasLost && (
        <ToolbarItem
          id="viewer.canvas.return"
          label="Return to Composition"
          metadata={{ symbol: "viewfinder", surface: "ViewerOverlay" }}
          onClick={() => actions.drive("viewer.fit")}
        >
          Return to Composition
        </ToolbarItem>
      )}
      {state.host.presentation !== "Docked" && (
        <div
          id="viewer.overlay"
          role="group"
          label="Viewer controls"
          // DESIGN-TODO(kit): ViewerOverlay glass capsule recipe (bottom-centre,
          // m.inset above the edge, dark scheme, idle fade). Structure only.
          metadata={{ surface: "ViewerOverlay", appearance: "dark" }}
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "center",
            gap: token("metrics.gap"),
          }}
        >
          {popups}
        </div>
      )}
    </div>
  );
}
