import type { ComponentChildren } from "preact";
import { useLayoutEffect, useReducer } from "preact/hooks";
import type { SurfaceEvent } from "../../src";
import * as UI from "../../src/components";
import { RenderQueue } from "./queue";
import { ExportModel, progress, type Options } from "./model";
import {
  audioCodings,
  codecs,
  colorNote,
  colorNoteRec709,
  colorTags,
  depths,
  descriptions,
  fieldDefaults,
  formats,
  kinds,
  kindOf,
  profiles,
  dynamicRanges,
  qualities,
  listedRate,
  rateOptions,
  rows,
  shows,
  sidecars,
  studioQualities,
  studioPromises,
  guidedDenoiseNote,
  estimateNotes,
  extensions,
} from "./spec";
const { token } = UI;
const text = {
  color: token("colors.text"),
  fontSize: token("typography.body"),
};
/** Leading content and trailing controls on one line (design review 2026-11-01). */
const line = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
  justifyContent: "space-between",
} as const;
const cluster = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
  gap: token("metrics.gap"),
} as const;
const stack = { display: "flex", flexDirection: "column" } as const;
const actions = { ...cluster, flexShrink: 0 } as const;
const detail = {
  color: token("colors.muted"),
  fontSize: token("typography.minimum"),
};

/** Use the approved kit's sheet form controls without changing the v4 API. */
function FormRow(props: {
  name: keyof typeof rows;
  help?: string;
  children: ComponentChildren;
}) {
  const [label, help] = rows[props.name];
  return (
    <UI.FormRow
      label={label}
      detail={props.help ?? help}
      id={`export.row.${props.name}`}
      control={props.children}
    />
  );
}
function Group(props: { label: string; children: ComponentChildren }) {
  return <UI.FormGroup title={props.label}>{props.children}</UI.FormGroup>;
}
/** Native NSButton role metadata is owned by the approved kit. */
function Action(props: {
  label: string;
  id: string;
  disabled?: boolean;
  primary?: boolean;
  onClick: () => void;
}) {
  const { primary, ...rest } = props;
  return <UI.PushButton {...rest} role={primary ? "default" : "normal"} />;
}
export interface ExportSheetProps {
  model: ExportModel;
  anchor: number;
}
export function ExportSheet({ model, anchor }: ExportSheetProps) {
  const [, update] = useReducer((n) => n + 1, 0);
  useLayoutEffect(() => model.subscribe(() => update(null)), [model]);
  const s = model.state;
  const p = s.project;
  if (!p || !model.open || s.run.phase === "cancelled") return null;
  const o = model.options;
  const busy = model.locked;
  const watch = (job: Promise<unknown>) => {
    void job.catch((error) => {
      // A command that failed is said (an ambiguous host acknowledgement
      // fences every later one, Cancel included).
      console.error(
        `export: ${error instanceof Error ? error.message : String(error)}`,
      );
      update(null);
    });
  };
  const edit = <K extends keyof Options>(key: K, value: Options[K]) => {
    model.set(key, value);
    watch(model.refreshPlan());
  };
  // The 3D preset this export renders with: the sheet's choice, or the
  // composition's own.
  const studioPreset =
    o.studioQuality && o.studioQuality !== "composition"
      ? o.studioQuality
      : (p.studio?.preset ?? p.studio?.saved.toLowerCase() ?? "production");
  const help = (key: keyof typeof rows) =>
    key === "audio" && !p.hasAudio
      ? "No audio layers in this project."
      : key === "captionsSidecar" && p.captionLayers > 1
        ? "Also write one subtitle file per captions layer beside the video."
        : key === "studioQuality"
          ? (studioPromises[studioPreset] ?? rows[key][1]) +
            (p.studio?.denoiser === "guided" && studioPreset !== "draft"
              ? ` ${guidedDenoiseNote}`
              : "")
          : key === "studioTime"
            ? estimateNotes[p.studio?.state ?? "guess"]
            : rows[key][1];
  const select = (
    key: keyof Options,
    options: UI.Option[],
    row: keyof typeof rows = key as keyof typeof rows,
  ) => (
    <FormRow name={row} help={help(row)}>
      <UI.PopUpButton
        id={`export.${key}`}
        label={rows[row][0]}
        title={help(row)}
        metadata={
          key === "format"
            ? {
                export: {
                  optionSubtitles: Object.fromEntries(
                    formats
                      .filter((f) => !s.capabilities[f.v].available)
                      .map((f) => [f.v, s.capabilities[f.v].reason ?? ""]),
                  ),
                },
              }
            : undefined
        }
        binding={s.bindings[key]}
        value={
          key === "dynamicRange"
            ? (o[key] ?? "sdr")
            : key === "studioQuality"
              ? (o[key] ?? "composition")
              : key === "colorTag" || key === "audioCoding"
                ? (o[key] ?? fieldDefaults[key])
                : key === "fps"
                  ? listedRate(o.fps)
                  : (o[key] ?? null)
        }
        options={options}
        disabled={busy}
        onChange={(e) => edit(key, e.value as Options[typeof key])}
      />
    </FormRow>
  );
  const toggle = (
    key:
      | "mblur"
      | "audio"
      | "alpha"
      | "captionsBurn"
      | "useProxies"
      | "studioReducedSize"
      | "dolbyVision",
  ) => (
    <FormRow name={key} help={help(key)}>
      <UI.Switch
        id={`export.${key}`}
        label={rows[key][0]}
        title={help(key)}
        binding={s.bindings[key]}
        value={o[key] ?? key === "dolbyVision"}
        disabled={busy || (key === "audio" && !p.hasAudio)}
        onChange={(e) => edit(key, e.value as boolean)}
      />
    </FormRow>
  );
  const keyboard = (e: SurfaceEvent) => {
    if (model.queueOpen && e.key !== "Escape" && !(e.metaKey && e.key === "."))
      return;
    if (
      e.key === "Escape" ||
      e.key === "Enter" ||
      (e.metaKey && e.key === ".")
    ) {
      e.stopPropagation();
      watch(model.key(e.key!, !!e.metaKey, !!e.shiftKey));
    }
  };
  // Why the chosen format can't export (offline originals, no encoder): said
  // in the footer instead of a silently disabled button.
  const blocked =
    s.capabilities[o.format]?.available === false
      ? (s.capabilities[o.format].reason ?? undefined)
      : undefined;
  const summary = model.planningError
    ? `Can’t export with these settings: ${model.planningError}`
    : (blocked ?? p.summary);
  const active = [
    "choosingDestination",
    "preparing",
    "rendering",
    "checking",
    "finishing",
    "cancelling",
  ].includes(s.run.phase);
  const jobs = s.queue ?? [];
  const rendering = jobs.find((job) => job.status === "Rendering");
  const running = active || !!rendering;
  const fraction =
    s.run.phase === "rendering" || s.run.phase === "checking"
      ? s.run.frame / s.run.total
      : rendering?.progress;
  const currentName = rendering?.name ?? p.name;
  const currentProgress =
    (active && s.run.phase !== "choosingDestination") || !!rendering;
  const queueIndicator = running ? (
    <UI.ProgressRow
      id="export.queue.indicator"
      label="Export in progress"
      value={fraction}
      children={[]}
      metadata={
        fraction === undefined ? {} : { pmKit: { queueProgress: "ring" } }
      }
      style={{
        width: token("metrics.symbol_panel"),
        height: token("metrics.symbol_panel"),
      }}
    />
  ) : null;
  return (
    <UI.SheetAnchor
      id="export.sheet"
      label={model.queueOpen ? "Queue" : "Export"}
      anchor={anchor}
      open
      style={{
        ...stack,
        padding: token("space.xl"),
        width: token("metrics.sheet_width_wide"),
        maxHeight: token("metrics.settings_height"),
        gap: token("space.lg"),
        overflow: "hidden",
      }}
      metadata={{
        export: {
          width: "metrics.sheet_width_wide",
          inset: "space.xl",
          dismissOutside: false,
          dismissEscape: true,
          initialFocus: model.queueOpen
            ? "export.queue.back"
            : active
              ? null
              : "export.kind",
        },
      }}
      focusRequest={
        model.queueOpen
          ? "export.queue.back"
          : active
            ? undefined
            : "export.kind"
      }
      onKeyDown={keyboard}
      preventDefault={{
        keydown: model.queueOpen ? ["Escape"] : ["Enter", "Escape"],
      }}
    >
      <div id="export.header" style={{ ...line, flexShrink: 0 }}>
        <div style={cluster}>
          {model.queueOpen ? (
            <UI.ToolbarItem
              id="export.queue.back"
              label="Back to export settings"
              title="Back"
              attr={{ symbol: "chevron.left" }}
              onClick={() => model.backToSettings()}
            />
          ) : null}
          <text
            role="heading"
            style={{ ...text, fontWeight: token("typography.display_weight") }}
          >
            {model.queueOpen ? "Queue" : "Export"}
          </text>
        </div>
        <UI.ToolbarItem
          id="export.dismiss"
          label="Close export"
          title="Close"
          attr={{ symbol: "xmark" }}
          disabled={
            !!s.destinationChoosing || s.run.phase === "choosingDestination"
          }
          onClick={() => watch(model.dismiss())}
        />
      </div>
      {model.queueOpen ? (
        <scroll
          id="export.queue.rows"
          label="Render queue and history"
          style={{
            ...stack,
            flexShrink: 1,
            minHeight: token("metrics.row"),
            maxHeight: token("metrics.sheet_body_max"),
            overflow: "hidden",
          }}
        >
          <RenderQueue model={model} watch={watch} />
        </scroll>
      ) : (
        <>
          <UI.SegmentedControl
            id="export.kind"
            label="Export type"
            value={model.kind}
            options={kinds}
            disabled={busy}
            style={{ flexShrink: 0 }}
            onChange={(kind) => {
              model.chooseKind(kind as "video" | "images" | "project");
              setTimeout(() => watch(model.refreshPlan()), 0);
            }}
          />
          <text
            id="export.description"
            style={{
              ...detail,
              color: token("colors.secondary"),
              flexShrink: 0,
            }}
          >
            {descriptions[model.kind]}
          </text>
          <scroll
            id="export.options"
            role="group"
            label="Export options"
            style={{
              ...stack,
              gap: token("space.lg"),
              flexShrink: 1,
              minHeight: token("metrics.row"),
              maxHeight: token("metrics.sheet_body_max"),
              overflow: "hidden",
            }}
          >
            <Group label="Output">
              {select(
                "format",
                formats
                  .filter((f) => kindOf(f.v) === model.kind)
                  .map((f) => ({
                    ...f,
                    disabled: !s.capabilities[f.v].available,
                  })),
              )}
              {shows(o, "quality") ? select("quality", qualities) : null}
              {shows(o, "profile") ? select("profile", profiles) : null}
              {shows(o, "depth") ? select("depth", depths) : null}
              {shows(o, "range")
                ? select("range", [
                    {
                      v: "work",
                      label: p.hasWorkArea
                        ? "Work area"
                        : "Work area · not set",
                    },
                    { v: "all", label: "Full composition" },
                  ])
                : null}
              {shows(o, "scale")
                ? select("scale", [
                    ...([0.5, 1, 2] as const).map((scale, i) => ({
                      v: scale,
                      label: `${["Half", "Full", "2×"][i]} · ${Math.round(p.width * scale)}×${Math.round(p.height * scale)}`,
                    })),
                    { v: "custom", label: "Custom size…" },
                  ])
                : null}
              {shows(o, "scale") && o.scale === "custom" ? (
                <FormRow name="size">
                  {(["width", "height"] as const).map((axis, i) => (
                    <div key={axis}>
                      {i ? <text style={text}>×</text> : null}
                      <UI.NumberField
                        id={`export.${axis}`}
                        label={axis === "width" ? "Width" : "Height"}
                        title={rows.size[1]}
                        value={model.size[axis]}
                        binding={s.bindings[axis]}
                        min={16}
                        max={8192}
                        step={1}
                        unit="px"
                        disabled={busy}
                        metadata={{ export: { axis } }}
                        onChange={(e) => {
                          model.customSize(axis, Number(e.value));
                          if (!model.error) watch(model.refreshPlan());
                        }}
                      />
                    </div>
                  ))}
                  <text metadata={{ export: { symbol: "link" } }} />
                  {model.error ? (
                    <text style={{ ...detail, color: token("colors.danger") }}>
                      {model.error}
                    </text>
                  ) : null}
                </FormRow>
              ) : null}
              {shows(o, "fps") ? select("fps", rateOptions(o.fps)) : null}
              <UI.FormRow
                id="export.row.destination"
                label="Destination"
                control={
                  <div style={{ ...cluster, flexGrow: 1, overflow: "hidden" }}>
                    <text
                      id="export.destination"
                      title={model.destination?.path}
                      style={{
                        ...detail,
                        flexGrow: 1,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {model.destination?.path.split(/[\\/]/).at(-1) ??
                        `${p.name}.${extensions[o.format]}`}
                    </text>
                    <Action
                      id="export.destination.choose"
                      label={s.destinationChoosing ? "Choosing…" : "Choose…"}
                      disabled={busy}
                      onClick={() => watch(model.chooseDestination())}
                    />
                  </div>
                }
              />
            </Group>
            {model.kind !== "project" ? (
              <Group label="Rendering">
                {/* HDR MP4 is always HEVC 10-bit: H.264 carries no HDR. */}
                {shows(o, "codec") &&
                !(p.hdr && (o.dynamicRange ?? "sdr") !== "sdr")
                  ? select("codec", codecs)
                  : null}
                {p.hdr && (o.format === "prores" || o.format === "mp4")
                  ? select("dynamicRange", dynamicRanges)
                  : null}
                {/* An MP4 in HDR is HEVC; HLG HEVC carries the metadata. */}
                {p.hdr &&
                o.format === "mp4" &&
                (o.dynamicRange ?? "sdr") === "hlg"
                  ? toggle("dolbyVision")
                  : null}
                {/* HDR delivery carries its own label. */}
                {shows(o, "colorTag") &&
                !(p.hdr && (o.dynamicRange ?? "sdr") !== "sdr")
                  ? select("colorTag", colorTags)
                  : null}
                {shows(o, "timecode") ? (
                  <FormRow name="timecode" help={help("timecode")}>
                    <UI.TextField
                      id="export.timecode"
                      label={rows.timecode[0]}
                      title={help("timecode")}
                      binding={s.bindings.timecode}
                      value={o.timecode ?? ""}
                      disabled={busy}
                      // A field has no width of its own while it is empty.
                      style={{ width: token("metrics.value_column") }}
                      metadata={{ export: { placeholder: "00:00:00:00" } }}
                      onInput={(e) =>
                        edit("timecode", String(e.value ?? "").trim())
                      }
                    />
                  </FormRow>
                ) : null}

                {p.studio && o.format !== "json"
                  ? select("studioQuality", studioQualities(p.studio.saved))
                  : null}
                {p.studio?.estimate && o.format !== "json" ? (
                  // The estimate is the decision this group exists for: its
                  // own row, in the text colour, beside the preset it prices.
                  <FormRow name="studioTime" help={help("studioTime")}>
                    <text
                      id="export.studioTime"
                      role="status"
                      label={`Render time ${p.studio.estimate}`}
                      style={{
                        ...text,
                        fontWeight: token("typography.display_weight"),
                        fontVariantNumeric: "tabular-nums",
                        color: token(
                          p.studio.state === "measuring"
                            ? "colors.secondary"
                            : "colors.text",
                        ),
                      }}
                    >
                      {p.studio.estimate}
                    </text>
                  </FormRow>
                ) : null}
                {p.studio && o.format !== "json"
                  ? toggle("studioReducedSize")
                  : null}
                {shows(o, "mblur") ? toggle("mblur") : null}
                {shows(o, "audio") ? toggle("audio") : null}
                {shows(o, "audioCoding") && p.hasAudio && o.audio
                  ? select("audioCoding", audioCodings)
                  : null}
                {shows(o, "alpha") ? toggle("alpha") : null}
                {shows(o, "useProxies") ? toggle("useProxies") : null}
              </Group>
            ) : null}
            {p.captionLayers && model.kind !== "project" ? (
              <Group label="Captions">
                {toggle("captionsBurn")}
                {model.kind === "video"
                  ? select("captionsSidecar", sidecars)
                  : null}
              </Group>
            ) : null}
            {model.kind !== "project" ? (
              <Group label="Presets">
                <FormRow name="preset">
                  <UI.PopUpButton
                    id="export.preset"
                    label="Saved Preset"
                    title={rows.preset[1]}
                    binding={s.bindings.preset}
                    value={model.chosenPreset}
                    disabled={busy}
                    options={[
                      { v: "", label: "Choose Preset" },
                      ...s.presets.map((p) => ({ v: p.id, label: p.name })),
                    ]}
                    onChange={(e) => watch(model.choosePreset(String(e.value)))}
                  />
                </FormRow>

                <FormRow name="presetSave">
                  <UI.TextField
                    id="export.presetName"
                    label="Save as Preset"
                    title={rows.presetSave[1]}
                    binding={s.bindings.presetName}
                    value={model.presetName}
                    disabled={busy}
                    style={{ width: token("metrics.value_column") }}
                    metadata={{ export: { placeholder: "Preset name" } }}
                    onInput={(e) => model.namePreset(String(e.value ?? ""))}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.stopPropagation();
                        watch(model.savePreset());
                      }
                    }}
                    preventDefault={{ keydown: ["Enter"] }}
                  />
                  <Action
                    id="export.savePreset"
                    label="Save"
                    disabled={busy || !model.presetName.trim() || !!model.error}
                    onClick={() => watch(model.savePreset())}
                  />
                </FormRow>
              </Group>
            ) : null}
            {/* Notes stand under their card, on the text column. */}
            {model.presetError ? (
              <text
                role="status"
                style={{
                  color: token("colors.danger"),
                  fontSize: token("typography.minimum"),
                }}
              >
                Couldn’t save the preset. Try again.
              </text>
            ) : null}
            {model.kind !== "project" ? (
              <text style={detail}>
                {shows(o, "colorTag") &&
                o.colorTag === "rec709" &&
                !(p.hdr && (o.dynamicRange ?? "sdr") !== "sdr")
                  ? colorNoteRec709
                  : colorNote}
              </text>
            ) : null}
          </scroll>
          <text
            id="export.summary"
            role="status"
            label={summary}
            title={blocked ? s.capabilities[o.format].detail : p.note}
            metadata={{ export: { announce: "polite" } }}
            style={{
              ...detail,
              color: token(
                model.planningError || blocked
                  ? "colors.warning"
                  : "colors.secondary",
              ),
              flexShrink: 0,
            }}
          >
            {summary}
          </text>
          {s.run.phase === "done" && !rendering ? (
            <div
              id="export.completed"
              role="status"
              style={{ ...line, gap: token("space.md"), flexShrink: 0 }}
            >
              <text style={text}>Export complete</text>
              <div style={cluster}>
                <Action
                  id="export.showInFinder"
                  label="Show in Finder"
                  onClick={() => watch(model.showInFinder())}
                />
                <Action
                  id="export.done"
                  label="Done"
                  onClick={() => watch(model.dismissCompleted())}
                />
              </div>
            </div>
          ) : s.run.phase === "failed" && !rendering ? (
            <div
              id="export.failure"
              role="status"
              style={{ ...line, gap: token("space.md"), flexShrink: 0 }}
            >
              <text
                style={{
                  ...detail,
                  color: token("colors.warning"),
                  flexGrow: 1,
                }}
              >
                {s.run.message}
              </text>
              <Action
                id="export.close"
                label="Close"
                disabled={model.pending}
                onClick={() => watch(model.closeFailure())}
              />
              {s.run.offline ? (
                <Action
                  id="export.without"
                  label="Export Without Them"
                  disabled={model.pending}
                  onClick={() => watch(model.exportWithout())}
                />
              ) : null}
              <Action
                id="export.retry"
                label="Try Again"
                disabled={model.pending}
                primary
                onClick={() => watch(model.retry())}
              />
            </div>
          ) : currentProgress ? (
            <div
              id="export.current"
              role="group"
              label="Current export"
              title={[progress(s.run).leading, progress(s.run).eta]
                .filter(Boolean)
                .join(" · ")}
              style={{
                ...cluster,
                flexShrink: 0,
                minHeight: token("metrics.control"),
              }}
            >
              <text
                id="export.current.name"
                style={{
                  ...text,
                  flexGrow: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {currentName}
              </text>
              <UI.ProgressRow
                id="export.progress"
                label="Export progress"
                value={fraction}
                children={[]}
                metadata={{
                  pmKit: { queueProgress: "bar" },
                  export: {
                    valueDescription:
                      progress(s.run).ax ?? progress(s.run).leading,
                  },
                }}
                style={{
                  width: token("metrics.export_job_progress_width"),
                  height: token("metrics.export_job_progress_height"),
                }}
              />
              <text
                id="export.current.percent"
                style={{ ...detail, fontVariantNumeric: "tabular-nums" }}
              >
                {fraction === undefined
                  ? progress(s.run).leading
                  : `${Math.round(fraction * 100)} %`}
              </text>
              <UI.ToolbarItem
                id="export.cancel"
                label="Cancel export"
                title="Cancel"
                attr={{ symbol: "xmark" }}
                disabled={s.run.phase === "cancelling"}
                onClick={() =>
                  watch(
                    !active && rendering
                      ? model.queueAction(rendering.id, "cancel")
                      : model.cancel(),
                  )
                }
              />
            </div>
          ) : null}
        </>
      )}
      <div
        id="export.footer"
        role="group"
        label="Export actions"
        style={{
          ...line,
          gap: token("space.md"),
          flexShrink: 0,
          paddingTop: token("space.lg"),
          borderTopWidth: token("metrics.border"),
          borderColor: token("colors.separator"),
        }}
      >
        <UI.ToolbarItem
          id="export.queue.open"
          label={`Queue${jobs.length ? `, ${jobs.length} exports` : ""}`}
          title={
            model.queueOpen
              ? "Return to export settings"
              : "Show queue and history"
          }
          active={model.queueOpen}
          onClick={() =>
            model.queueOpen ? model.backToSettings() : model.showQueue()
          }
        >
          <div style={cluster}>
            {queueIndicator}
            <text style={text}>Queue</text>
            {jobs.length ? (
              <text
                id="export.queue.badge"
                label={`${jobs.length} exports`}
                style={{
                  ...detail,
                  paddingLeft: token("space.xs"),
                  paddingRight: token("space.xs"),
                  borderRadius: token("metrics.pill_radius"),
                  background: token("colors.fill"),
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {jobs.length}
              </text>
            ) : null}
          </div>
        </UI.ToolbarItem>
        <div style={actions}>
          <Action
            id="export.queue.add"
            label="Add to Queue"
            disabled={!model.canExport}
            onClick={() => watch(model.addToQueue())}
          />
          <Action
            id="export.start"
            label={model.startWanted ? "Preparing…" : "Export"}
            primary
            disabled={!model.canExport && !model.canPress}
            onClick={() => {
              model.backToSettings();
              watch(model.start());
            }}
          />
        </div>
      </div>
    </UI.SheetAnchor>
  );
}
