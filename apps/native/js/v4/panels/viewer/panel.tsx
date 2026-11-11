import { useEffect, useState } from "preact/hooks";
import {
  EmptyState,
  NumberField,
  PopUpButton,
  SegmentedControl,
  ToolbarItem,
  token,
} from "../../src/components";
import type { SurfaceProps } from "../../src/jsx-runtime";
import type { Json } from "../../src/protocol";
import {
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
  if (!m)
    return (
      <div
        id="viewer"
        role="group"
        label="Viewer"
      >
        <EmptyState
          label="No Composition"
          explanation="Open a project to start editing."
          action={
            <ToolbarItem id="viewer.open" label="Open…" onClick={actions.open}>
              Open…
            </ToolbarItem>
          }
        />
      </div>
    );
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
  const popups = (
    <>
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
  // 3d-studio.md §2: Look Through, Tool, Axes, Frame, Views and Preview,
  // leading the bar, only while the composition has 3D layers. Tooltips are
  // two lines: the name (with its shortcut), then what it does.
  const looks = [
    { id: "camera", label: "Camera", help: "See exactly what the shot renders." },
    { id: "free", label: "Free View", help: "Look around without changing the shot." },
  ];
  const tools = [
    { id: "move", label: "Move", symbol: "move.3d", help: "Drag the selection, or one arrow for one axis." },
    { id: "rotate", label: "Rotate", symbol: "rotate.3d", help: "Turn the selection by its rings." },
    { id: "scale", label: "Scale", symbol: "scale.3d", help: "Resize the selection." },
    { id: "orbit", label: "Orbit", symbol: "view.3d", help: "Drag to circle the view. Anytime: ⌥-drag." },
    { id: "pan", label: "Pan", symbol: "hand.raised", help: "Drag to slide the view. Anytime: ⇧⌥-drag." },
    { id: "zoom", label: "Zoom", symbol: "plus.magnifyingglass", help: "Drag to move closer. Anytime: ⌥-scroll." },
    { id: "light", label: "Light", symbol: "lightbulb", help: "Click a model to put light there." },
  ];
  const views = [
    { id: "1", label: "One View", short: "1 View", help: "One view of the scene." },
    { id: "2", label: "Two Views", short: "2 Views", help: "Side by side, each with its own camera." },
    { id: "4", label: "Four Views", short: "4 Views", help: "The shot, top, front and side." },
  ];
  const tiers = [
    { id: "live", label: "Fast", help: "Real-time preview. Exports stay full quality." },
    { id: "refine", label: "Quality", help: "Path-traced, as it will export." },
  ];
  const object = m.studio?.axes === "object";
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
        ? "Refining · Denoising"
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
  // 3d-studio.md §6: the Light panel, top-leading over the stage while Light
  // is the tool. Values are the model's; a pull is one Undo ("Light Size").
  const light = m.studio?.tool === "light" ? m.studio.light : undefined;
  const kinds = [
    { v: "area", label: "Softbox" },
    { v: "spot", label: "Spotlight" },
    { v: "point", label: "Bulb" },
    { v: "sun", label: "Sun" },
  ];
  const lightPanel = light ? (
    <div
      id="viewer.studio.lightPanel"
      role="group"
      label="Light Tool"
      style={{
        position: "absolute",
        left: token("metrics.gap"),
        top: token("metrics.gap"),
        width: token("metrics.popover_width"),
        display: "flex",
        flexDirection: "column",
        alignItems: "stretch",
        gap: token("metrics.gap"),
        padding: token("metrics.inset"),
        background: token("colors.floating"),
        borderRadius: token("metrics.control_radius"),
      }}
    >
      <SegmentedControl
        id="viewer.studio.lightAim"
        label="Aim"
        value={m.studio?.lightAim ?? "reflect"}
        options={[
          { id: "reflect", label: "Reflect", help: "The highlight lands where you click." },
          { id: "direct", label: "Direct", help: "The light faces where you click." },
          { id: "orbit", label: "Orbit", help: "Drag to circle the light around its target." },
        ]}
        onChange={(value) => actions.drive("viewer.studio.lightAim", value)}
      />
      <div style={{ display: "flex", flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: token("metrics.gap") }}>
        <ToolbarItem
          id="viewer.studio.lightAdd"
          label="Add"
          title={"Add\nThen click a model to put it there."}
          active={light.settings.add}
          onClick={() => actions.drive("viewer.studio.lightAdd", light.settings.add ? "off" : "on")}
        >
          Add
        </ToolbarItem>
        <PopUpButton
          id="viewer.studio.lightKind"
          data-pm-variant="inline"
          binding="viewer.studio.lightKind"
          label="Light to Add"
          value={m.studio?.lightKind ?? "area"}
          style={popupStyle}
          options={kinds}
          onChange={(event) => actions.drive("viewer.studio.lightKind", event.value)}
        />
      </div>
      {light.hint ? (
        <text style={{ color: token("colors.muted"), fontSize: token("typography.body") }}>
          {light.hint}
        </text>
      ) : (
        <>
          <div style={{ display: "flex", flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", gap: token("metrics.gap") }}>
            <text style={{ color: token("colors.text"), fontSize: token("typography.title"), fontWeight: token("typography.semibold_weight") }}>
              {light.name ?? ""}
            </text>
            <text style={{ color: token("colors.muted"), fontSize: token("typography.minimum") }}>
              {light.kind ?? ""}
            </text>
          </div>
          {light.rows.map((row) => (
            <div
              key={row.id}
              style={{ display: "flex", flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: token("metrics.gap"), height: token("metrics.row") }}
            >
              <text
                title={`${row.label}\n${row.tooltip}`}
                style={{ color: token("colors.secondary"), fontSize: token("typography.body"), whiteSpace: "nowrap" }}
              >
                {row.label}
              </text>
              <NumberField
                id={`viewer.studio.light.${row.id}`}
                label={row.label}
                title={`${row.label}\n${row.tooltip}`}
                binding={`viewer.studio.light.${row.id}`}
                value={Math.round(row.value * 100) / 100}
                min={row.min}
                max={row.max}
                step={row.max > 100 ? 0.5 : 0.05}
                precision={row.value < 10 ? 2 : row.value < 100 ? 1 : 0}
                unit={row.id === "spread" ? "°" : ""}
                disabled={light.locked}
                style={{ width: token("metrics.value_column") }}
                onChange={(e) => actions.drive(`viewer.studio.light.${row.id}`, Number(e.value))}
              />
            </div>
          ))}
        </>
      )}
    </div>
  ) : null;
  const studio = !m.studio ? null : m.studio.compact ? (
    // Narrower than the group: one 3D pull-down, same items and order.
    <PopUpButton
      id="viewer.studio.menu"
      data-pm-variant="inline"
      binding="viewer.studio.menu"
      title={"3D\nCamera, tools, axes, views and preview."}
      label="3D"
      display="3D"
      value={null}
      style={popupStyle}
      options={[
        ...looks.map((o) => ({ v: `look:${o.id}`, label: o.label, checked: (m.studio?.look ?? "camera") === o.id })),
        { separator: true },
        ...tools.map((o) => ({ v: `tool:${o.id}`, label: o.label, checked: (m.studio?.tool ?? "move") === o.id })),
        { separator: true },
        { v: "axes:scene", label: "Scene Axes", checked: !object },
        { v: "axes:object", label: "Object Axes", checked: object },
        { v: "frame", label: "Frame" },
        { separator: true },
        ...views.map((o) => ({ v: `views:${o.id}`, label: o.label, checked: (m.studio?.views ?? "1") === o.id })),
        { separator: true },
        ...tiers.map((o) => ({ v: `tier:${o.id}`, label: o.label, checked: m.studio?.tier === o.id })),
      ]}
      onChange={(event) => actions.drive("viewer.studio.menu", event.value)}
    />
  ) : (
    <div
      id="viewer.studio"
      role="group"
      label="3D"
      style={{
        display: "flex",
        flexDirection: "row",
        alignItems: "center",
        gap: token("metrics.gap"),
      }}
    >
      <SegmentedControl
        id="viewer.studio.look"
        label="Look Through"
        value={m.studio.look ?? "camera"}
        options={looks}
        onChange={(value) => actions.drive("viewer.studio.look", value)}
      />
      <SegmentedControl
        id="viewer.studio.tool"
        label="Tool"
        value={m.studio.tool ?? "move"}
        options={tools}
        onChange={(value) => actions.drive("viewer.studio.tool", value)}
      />
      <ToolbarItem
        id="viewer.studio.axes"
        label={object ? "Object Axes" : "Scene Axes"}
        title={
          object
            ? "Object Axes\nHandles line up with the object. Click for the scene’s."
            : "Scene Axes\nHandles line up with the scene. Click for the object’s."
        }
        metadata={{ symbol: object ? "cube" : "globe" }}
        onClick={() => actions.drive("viewer.studio.axes", object ? "scene" : "object")}
      />
      <ToolbarItem
        id="viewer.studio.frame"
        label="Frame"
        title={"Frame (F)\nFit the selection, or everything."}
        metadata={{ symbol: "viewfinder" }}
        onClick={() => actions.drive("viewer.studio.frame")}
      />
      <PopUpButton
        id="viewer.studio.views"
        data-pm-variant="inline"
        binding="viewer.studio.views"
        title={"Views\nOne view, two side by side, or four."}
        label="Views"
        value={m.studio.views ?? "1"}
        style={popupStyle}
        options={views.map((o) => ({ v: o.id, label: o.short, detail: o.help }))}
        onChange={(event) => actions.drive("viewer.studio.views", event.value)}
      />
      <SegmentedControl
        id="viewer.studio.tier"
        label="Preview"
        value={m.studio.tier}
        options={tiers}
        onChange={(value) => actions.drive("viewer.studio.tier", value)}
      />
    </div>
  );
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
        <div
          id="viewer.bar"
          data-pm-slot="header"
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            height: token("metrics.header"),
            gap: token("metrics.gap"),
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.gap"),
            justifyContent: source || studio ? "space-between" : "end",
          }}
        >
          {!source && studio}
          {source && (
            <div
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: token("metrics.gap"),
              }}
            >
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
            </div>
          )}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
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
        {lightPanel}
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
