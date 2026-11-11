import {
  EmptyState,
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
  const value = `${m.name || "Untitled Composition"}, ${percent(m.shownZoom)} percent, ${effective ? `${spoken(effective)} resolution, ` : ""}${m.selectionCount} layers selected`;
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
  // 3d-studio.md §2 item 5: Fast | Quality, shown only with 3D layers.
  // Fast is the live path; Quality refines toward what the export renders.
  const studio = m.studio ? (
    <SegmentedControl
      id="viewer.studio.tier"
      label="Preview"
      value={m.studio.tier}
      options={[
        { id: "live", label: "Fast" },
        { id: "refine", label: "Quality" },
      ]}
      onChange={(value) => actions.drive("viewer.studio.tier", value)}
    />
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
