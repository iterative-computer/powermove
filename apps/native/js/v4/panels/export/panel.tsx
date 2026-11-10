import type { ComponentChildren } from "preact";
import { useLayoutEffect, useReducer } from "preact/hooks";
import type { SurfaceEvent } from "../../src";
import * as UI from "../../src/components";
import { ExportModel, isProgress, progress, type Options } from "./model";
import {
  codecs,
  colorNote,
  depths,
  descriptions,
  formats,
  kinds,
  kindOf,
  profiles,
  dynamicRanges,
  qualities,
  rates,
  rows,
  shows,
  sidecars,
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
/** Summary and actions under a hairline, like v1's sheet footer. */
const footer = {
  ...line,
  gap: token("metrics.gap"),
  borderTopWidth: token("metrics.border"),
  borderColor: token("colors.hairline"),
  paddingTop: token("metrics.inset"),
  marginTop: token("metrics.gap"),
} as const;
/** The footer's notice wraps beside the buttons; the buttons never give way. */
const notice = { ...stack, flexGrow: 1, flexShrink: 1 } as const;
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
  if (
    !p ||
    !model.open ||
    s.run.phase === "done" ||
    s.run.phase === "cancelled"
  )
    return null;
  const o = model.options;
  const busy = model.locked;
  const watch = (job: Promise<unknown>) => {
    void job.catch(() => update(null));
  };
  const edit = <K extends keyof Options>(key: K, value: Options[K]) => {
    model.set(key, value);
    watch(model.refreshPlan());
  };
  const help = (key: keyof typeof rows) =>
    key === "audio" && !p.hasAudio
      ? "No audio layers in this project."
      : key === "captionsSidecar" && p.captionLayers > 1
        ? "Also write one subtitle file per captions layer beside the video."
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
        value={key === "dynamicRange" ? (o[key] ?? "sdr") : o[key]}
        options={options}
        disabled={busy}
        onChange={(e) => edit(key, e.value as Options[typeof key])}
      />
    </FormRow>
  );
  const toggle = (
    key: "mblur" | "audio" | "alpha" | "captionsBurn" | "useProxies",
  ) => (
    <FormRow name={key} help={help(key)}>
      <UI.Checkbox
        id={`export.${key}`}
        label={rows[key][0]}
        title={help(key)}
        binding={s.bindings[key]}
        value={o[key] ?? false}
        disabled={busy || (key === "audio" && !p.hasAudio)}
        onChange={(e) => edit(key, e.value as boolean)}
      />
    </FormRow>
  );
  const keyboard = (e: SurfaceEvent) => {
    if (
      e.key === "Escape" ||
      e.key === "Enter" ||
      (e.metaKey && e.key === ".")
    ) {
      e.stopPropagation();
      watch(model.key(e.key!, !!e.metaKey, !!e.shiftKey));
    }
  };
  const progressSheet = isProgress(s.run);
  // Why the chosen format can't export (offline originals, no encoder): said
  // in the footer instead of a silently disabled button.
  const blocked =
    s.capabilities[o.format]?.available === false
      ? (s.capabilities[o.format].reason ?? undefined)
      : undefined;
  const summary = model.planningError
    ? `Can’t export with these settings: ${model.planningError}`
    : (blocked ?? p.summary);
  return (
    <UI.SheetAnchor
      id="export.sheet"
      label="Export"
      anchor={anchor}
      open
      style={{
        padding: token("metrics.sheet_inset"),
        width: token("metrics.sheet_width_wide"),
        gap: token("metrics.inset"),
      }}
      // No substitute for missing wide-sheet/inset tokens. Metadata is a request,
      // not a claim the current kit interprets these properties.
      metadata={{
        export: {
          width: "metrics.sheet_width_wide",
          inset: "metrics.sheet_inset",
          dismissOutside: false,
          dismissEscape: !progressSheet,
          initialFocus: progressSheet ? null : "export.kind",
        },
      }}
      focusRequest={progressSheet ? undefined : "export.kind"}
      onKeyDown={keyboard}
      preventDefault={{ keydown: ["Enter", "Escape"] }}
    >
      <text
        role="heading"
        style={{
          color: token("colors.text"),
          fontSize: token("typography.heading"),
          fontWeight: token("typography.semibold_weight"),
        }}
      >
        {progressSheet ? `Exporting “${p.name}”` : "Export"}
      </text>
      {progressSheet ? (
        <>
          {s.run.phase === "rendering" && s.run.preview ? (
            <frame
              resource={s.run.preview}
              metadata={{ export: { aspect: [p.width, p.height] } }}
              style={{
                background: token("colors.viewer"),
                borderRadius: token("metrics.control_radius"),
              }}
            />
          ) : null}
          {s.run.phase === "failed" ? (
            <text
              label={`Export failed: ${s.run.message}`}
              metadata={{
                export: {
                  symbol: "exclamationmark.triangle.fill",
                  announce: "polite",
                },
              }}
              style={{ ...text, color: token("colors.danger") }}
            >{`Export failed: ${s.run.message}`}</text>
          ) : (
            <>
              <UI.ProgressRow
                id="export.progress"
                label="Export progress"
                // The bar speaks its label; the lines below show the words.
                children={[]}
                value={progress(s.run).value}
                title={progress(s.run).eta}
                metadata={{
                  export: {
                    valueDescription:
                      progress(s.run).ax ?? progress(s.run).leading,
                  },
                }}
              />
              <div style={line}>
                <text style={{ ...text, fontVariantNumeric: "tabular-nums" }}>
                  {progress(s.run).leading}
                </text>
                {progress(s.run).eta ? (
                  <text
                    style={{
                      color: token("colors.secondary"),
                      fontSize: token("typography.body"),
                    }}
                  >
                    {progress(s.run).eta}
                  </text>
                ) : null}
              </div>
            </>
          )}
          <div
            role="group"
            label="Export"
            metadata={{ export: { recipe: "sheet-footer" } }}
            style={{
              ...footer,
              justifyContent: "end",
              gap: token("metrics.gap"),
            }}
          >
            {s.run.phase === "failed" ? (
              <>
                <Action
                  id="export.close"
                  label="Close"
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
                  primary
                  disabled={model.pending}
                  onClick={() => watch(model.retry())}
                />
              </>
            ) : (
              <Action
                id="export.cancel"
                label={s.run.phase === "cancelling" ? "Cancelling…" : "Cancel"}
                disabled={s.run.phase === "cancelling"}
                onClick={() => watch(model.cancel())}
              />
            )}
          </div>
        </>
      ) : (
        <>
          <UI.SegmentedControl
            id="export.kind"
            label="Export type"
            value={model.kind}
            options={kinds}
            disabled={busy}
            onChange={(kind) => {
              model.chooseKind(kind as "video" | "images" | "project");
              watch(model.refreshPlan());
            }}
          />
          <text
            style={{
              color: token("colors.secondary"),
              fontSize: token("typography.reading"),
            }}
          >
            {descriptions[model.kind]}
          </text>
          {model.kind === "project" ? (
            <UI.FormGroup label="Keep Every Layer Editable">
              <div
                role="group"
                label="Keep Every Layer Editable"
                style={{
                  ...stack,
                  gap: token("metrics.border"),
                  padding: token("metrics.inset"),
                }}
              >
                <text style={{ ...text, fontSize: token("typography.title") }}>
                  Keep every layer editable
                </text>
                <text
                  style={{ ...detail, fontSize: token("typography.label") }}
                >
                  Save a portable Powermove project with its media, effects,
                  keyframes and composition settings.
                </text>
                <text
                  style={{
                    color: token("colors.secondary"),
                    fontSize: token("typography.label"),
                    marginTop: token("metrics.gap"),
                  }}
                >
                  Editable layers · Original media · Animation & effects
                </text>
              </div>
            </UI.FormGroup>
          ) : (
            <scroll
              id="export.options"
              role="group"
              label="Export options"
              style={{
                // A long option list scrolls; the sheet stays on screen.
                ...stack,
                gap: token("metrics.inset"),
                maxHeight: token("metrics.sheet_body_max"),
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
                {shows(o, "profile") ? select("profile", profiles) : null}
                {/* HDR MP4 is always HEVC 10-bit: H.264 carries no HDR. */}
                {shows(o, "codec") &&
                !(p.hdr && (o.dynamicRange ?? "sdr") !== "sdr")
                  ? select("codec", codecs)
                  : null}
                {shows(o, "depth") ? select("depth", depths) : null}
                {shows(o, "scale")
                  ? select("scale", [
                      ...([0.5, 1, 2] as const).map((scale, i) => ({
                        v: scale,
                        label: `${["Half", "Full", "2×"][i]} · ${Math.round(p.width * scale)}×${Math.round(p.height * scale)}`,
                      })),
                      { v: "custom", label: "Custom size…" },
                    ])
                  : null}
                {o.scale === "custom" ? (
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
                      <text
                        style={{ ...detail, color: token("colors.danger") }}
                      >
                        {model.error}
                      </text>
                    ) : null}
                  </FormRow>
                ) : null}
                {shows(o, "fps") ? select("fps", rates) : null}
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
                {shows(o, "quality") ? select("quality", qualities) : null}
                {p.hdr && (o.format === "prores" || o.format === "mp4")
                  ? select("dynamicRange", dynamicRanges)
                  : null}
              </Group>
              <Group label="Rendering">
                {shows(o, "mblur") ? toggle("mblur") : null}
                {shows(o, "audio") ? toggle("audio") : null}
                {shows(o, "alpha") ? toggle("alpha") : null}
                {shows(o, "useProxies") ? toggle("useProxies") : null}
              </Group>
              {p.captionLayers ? (
                <Group label="Captions">
                  {toggle("captionsBurn")}
                  {model.kind === "video"
                    ? select("captionsSidecar", sidecars)
                    : null}
                </Group>
              ) : null}
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
                <text style={detail}>{colorNote}</text>
              </Group>
            </scroll>
          )}
          <div
            role="group"
            label="Export"
            metadata={{
              export: {
                recipe: "sheet-footer",
                height: ["metrics.control", "metrics.inset", "metrics.inset"],
              },
            }}
            style={footer}
          >
            <div style={notice}>
              <text
                id="export.summary"
                label={summary}
                title={blocked ? s.capabilities[o.format].detail : undefined}
                metadata={{
                  export: { announce: "polite" },
                  ...(model.planningError || blocked
                    ? { symbol: "exclamationmark.triangle.fill" }
                    : {}),
                }}
                style={{
                  color: token(
                    model.planningError || blocked
                      ? "colors.warning"
                      : "colors.secondary",
                  ),
                  fontSize: token("typography.body"),
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {summary}
              </text>
              {p.note ? <text style={detail}>{p.note}</text> : null}
            </div>
            <div style={actions}>
              <Action
                id="export.cancel"
                label="Cancel"
                disabled={busy}
                onClick={() => watch(model.cancel())}
              />
              <Action
                id="export.start"
                label={model.actionLabel}
                primary
                disabled={!model.canExport}
                onClick={() => watch(model.start())}
              />
            </div>
          </div>
        </>
      )}
    </UI.SheetAnchor>
  );
}
