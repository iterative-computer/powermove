import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import {
  PANEL_ID,
  PANEL_TITLE,
  presetName,
  status,
  customPresets,
  type Curve,
  type EasingAction,
  type EasingModel,
  type Interpolation,
} from "./model";
const t = UI.token;
const stop = (e: SurfaceEvent) => e.stopPropagation();
const row = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
  gap: t("metrics.gap"),
  paddingLeft: t("metrics.inset"),
  paddingRight: t("metrics.inset"),
};
export interface EasingProps {
  model: EasingModel;
  bindings: {
    curve: string;
    outX: string;
    outY: string;
    inX: string;
    inY: string;
    interpolation?: string;
    name?: string;
  };
}
// Preserve preset order within families, including native's Quart and named curves.
const groups = [
  "Ease",
  "Quad",
  "Cubic",
  "Quart",
  "Expo",
  "Circ",
  "Back",
  "Custom",
];
function family(id: string) {
  return (
    groups.find((g) => id.toLowerCase().startsWith(g.toLowerCase())) ??
    (id === "linear" ? "Ease" : "Custom")
  );
}
export function EasingPanel({ model, bindings }: EasingProps) {
  const [s, update] = useState(model.snapshot());
  const [message, setMessage] = useState(model.message);
  const [draft, setCurve] = useState<Curve>(s.curve ?? [0, 0, 1, 1]);
  const [hover, preview] = useState<Curve | undefined>();
  const [saveName, setSaveName] = useState("Custom easing");
  const [interpolation, setInterpolation] = useState(
    s.interpolation ?? "linear",
  );
  useLayoutEffect(
    () =>
      model.subscribe(() => {
        const next = model.snapshot();
        update(next);
        setMessage(model.message);
        if (next.curve !== undefined) setCurve(next.curve ?? [0, 0, 1, 1]);
        if (next.interpolation !== undefined)
          setInterpolation(next.interpolation ?? "linear");
        preview(undefined);
      }),
    [model],
  );
  const ready = s.open && s.count > 0;
  const curve: Curve = ready ? draft : [0, 0, 1, 1];
  const caption = !ready
    ? "Select keyframes in the timeline to ease them."
    : s.mixed
      ? "Mixed"
      : status(s, message);
  const act = (a: EasingAction) => {
    if (ready) model.send(a);
  };
  const edit = (next: Curve) => {
    if (ready) {
      setCurve(next);
      act({ type: "curve", curve: next });
    }
  };
  const influenceValue = (outgoing: boolean, raw: unknown, commit: boolean) => {
    if (!ready) return;
    const value = Number(raw) / 100;
    if (!Number.isFinite(value) || value < 0 || value > 1) return;
    const fraction = outgoing ? curve[0] : 1 - curve[2];
    const next = [...curve] as Curve;
    const speed = fraction
      ? (outgoing ? curve[1] : 1 - curve[3]) / fraction
      : 0;
    if (outgoing) {
      next[0] = value;
      next[1] = speed * value;
    } else {
      next[2] = 1 - value;
      next[3] = 1 - speed * value;
    }
    if (commit) edit(next);
    else setCurve(next);
  };
  const influence = (outgoing: boolean) => {
    const fraction = outgoing ? curve[0] : 1 - curve[2];
    const label = outgoing ? "Outgoing" : "Incoming";
    return (
      <div style={{ display: "flex", flexDirection: "column", flexGrow: 1 }}>
        <text
          style={{
            color: t("colors.secondary"),
            fontSize: t("typography.minimum"),
          }}
        >
          {label}
        </text>
        <UI.NumberField
          id={`easing:influence:${label.toLowerCase()}`}
          label={`${label} influence (%)`}
          binding={outgoing ? bindings.outX : bindings.inX}
          value={fraction * 100}
          disabled={!ready}
          step={1}
          precision={1}
          unit="%"
          min={0.1}
          max={100}
          onKeyDown={stop}
          metadata={{ commitOnRelease: true }}
          onInput={(e) => influenceValue(outgoing, e.value, false)}
          onChange={(e) => influenceValue(outgoing, e.value, true)}
        />
      </div>
    );
  };
  return (
    <div
      id={PANEL_ID}
      role="group"
      label={PANEL_TITLE}
      metadata={{ panel: PANEL_ID }}
      style={{
        background: t("colors.panel"),
        borderRadius: t("metrics.panel_radius"),
        display: "flex",
        flexDirection: "column",
        flexShrink: 0,
      }}
    >
      <div
        id="easing:header"
        data-pm-slot="header"
        style={{ ...row, height: t("metrics.header") }}
      >
        <text
          style={{
            color: t("colors.text"),
            fontSize: t("typography.title"),
            fontWeight: t("typography.semibold_weight"),
          }}
        >
          {PANEL_TITLE}
        </text>
      </div>
      <div
        id="easing:content"
        role="group"
        label="Easing"
        style={{ display: "flex", flexDirection: "column" }}
      >
        <UI.EaseCurve
          id="easing:curve"
          label="Easing curve"
          binding={bindings.curve}
          value={curve}
          disabled={!ready}
          metadata={{
            commitOnRelease: true,
            pmKit: hover ? { preview: hover } : {},
          }}
          onKeyDown={stop}
          onInput={(e) => {
            const next = Array.isArray(e.value) ? e.value.map(Number) : [];
            if (ready && next.length === 4 && next.every(Number.isFinite))
              setCurve(next as Curve);
          }}
          onChange={(e) => {
            const next = Array.isArray(e.value) ? e.value.map(Number) : [];
            if (next.length === 4 && next.every(Number.isFinite))
              edit(next as Curve);
          }}
          style={{
            height: t("metrics.curve_editor"),
            minHeight: t("metrics.curve_editor"),
            maxHeight: t("metrics.curve_editor"),
            flexShrink: 0,
            marginLeft: t("metrics.inset"),
            marginRight: t("metrics.inset"),
          }}
        />
        <text
          id="easing:status"
          role="status"
          label={caption}
          style={{
            ...row,
            height: t("metrics.row"),
            color: t("colors.secondary"),
            fontSize: t("typography.minimum"),
            overflow: "hidden",
            whiteSpace: "nowrap",
          }}
        >
          {caption}
        </text>
        <div role="group" label="Influence" style={row}>
          {influence(true)}
          {influence(false)}
        </div>
        <div
          role="group"
          label="Quick easing"
          style={{ ...row, marginTop: t("metrics.gap") }}
        >
          {(
            [
              ["both", "Easy Ease"],
              ["in", "Ease In"],
              ["out", "Ease Out"],
            ] as const
          ).map(([side, label]) => (
            <UI.PushButton
              key={side}
              id={`easing:ease:${side}`}
              label={label}
              style={{
                background: t("colors.fill"),
                borderColor: t("colors.separator"),
                borderWidth: t("metrics.border"),
                color: t(ready ? "colors.text" : "colors.muted"),
                height: t("metrics.control"),
                borderRadius: t("metrics.control_radius"),
                paddingLeft: t("metrics.gap"),
                paddingRight: t("metrics.gap"),
                flexShrink: 0,
              }}
              disabled={!ready}
              onKeyDown={stop}
              onClick={(e) => {
                stop(e);
                act({ type: "easyEase", side });
              }}
            />
          ))}
          <UI.PopUpButton
            id="easing:interpolation"
            label="Interpolation"
            widthSample="Bezier"
            binding={bindings.interpolation ?? "interpolation"}
            value={interpolation}
            disabled={!ready}
            options={[
              { v: "linear", label: "Linear" },
              { v: "bezier", label: "Bezier" },
              { v: "hold", label: "Hold" },
              { v: "auto", label: "Auto" },
            ]}
            onChange={(e) => {
              const v = String(e.value) as Interpolation;
              if (!["linear", "bezier", "hold", "auto"].includes(v)) return;
              setInterpolation(v);
              act({
                type: "interpolation",
                interpolation: v,
                side: "both",
              });
            }}
          />
        </div>
        {groups.map((group) => (
          <div
            key={group}
            role="group"
            label={group}
            style={{ display: "flex", flexDirection: "column" }}
          >
            <text
              role="heading"
              style={{
                ...row,
                height: t("metrics.row"),
                color: t("colors.secondary"),
                fontSize: t("typography.minimum"),
              }}
            >
              {group}
            </text>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(3, 1fr)",
                gap: t("metrics.gap"),
                paddingLeft: t("metrics.inset"),
                paddingRight: t("metrics.inset"),
              }}
            >
              {[
                ...s.presets.filter((p) => family(p.id) === group),
                ...(group === "Custom" ? customPresets(s.library?.source) : []),
              ].map((p) => {
                const selected =
                  ready &&
                  p.curve.every((v, i) => Math.abs(v - curve[i]!) < 0.0011);
                return (
                  <div
                    key={p.id}
                    id={`easing:preset:${p.id}`}
                    role="button"
                    label={"name" in p ? String(p.name) : presetName(p.id)}
                    title={
                      ready
                        ? "name" in p
                          ? String(p.name)
                          : presetName(p.id)
                        : "Select keyframes to ease"
                    }
                    disabled={!ready}
                    aria-selected={selected}
                    tabIndex={ready ? 0 : -1}
                    preventDefault={{ keydown: ["Enter"] }}
                    onPointerEnter={() => {
                      if (ready) preview(p.curve);
                    }}
                    onPointerLeave={() => preview(undefined)}
                    onFocus={() => {
                      if (ready) preview(p.curve);
                    }}
                    onBlur={() => preview(undefined)}
                    onKeyDown={(e) => {
                      stop(e);
                      if (e.key === "Enter") {
                        if (ready) {
                          setCurve(p.curve);
                          act(
                            p.id.startsWith("saved:")
                              ? {
                                  type: "curve",
                                  curve: p.curve,
                                  name:
                                    "name" in p
                                      ? String(p.name)
                                      : presetName(p.id),
                                }
                              : { type: "preset", preset: p.id },
                          );
                        }
                      }
                    }}
                    onClick={(e) => {
                      stop(e);
                      if (ready) {
                        setCurve(p.curve);
                        act(
                          p.id.startsWith("saved:")
                            ? {
                                type: "curve",
                                curve: p.curve,
                                name:
                                  "name" in p
                                    ? String(p.name)
                                    : presetName(p.id),
                              }
                            : { type: "preset", preset: p.id },
                        );
                      }
                    }}
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      alignItems: "center",
                      paddingTop: t("metrics.gap"),
                      paddingBottom: t("metrics.gap"),
                      borderRadius: t("metrics.control_radius"),
                      background: t("colors.panel"),
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "center",
                        width: "100%",
                        background: selected
                          ? t("colors.selected_fill")
                          : t("colors.panel"),
                      }}
                    >
                      <UI.EaseThumbnail
                        id={`easing:thumb:${p.id}`}
                        label={"name" in p ? String(p.name) : presetName(p.id)}
                        value={p.curve}
                        disabled={!ready}
                        style={{
                          width: t("metrics.header"),
                          height: t("metrics.row"),
                        }}
                      />
                      <text
                        style={{
                          color: t(ready ? "colors.secondary" : "colors.muted"),
                          fontSize: t("typography.minimum"),
                          whiteSpace: "nowrap",
                        }}
                      >
                        {"name" in p ? String(p.name) : presetName(p.id)}
                      </text>
                    </div>
                  </div>
                );
              })}
            </div>
            {group === "Custom" && (
              <UI.InspectorRow label="Save Current…">
                <div style={row}>
                  <UI.TextField
                    id="easing:save-name"
                    binding={bindings.name ?? "name"}
                    value={saveName}
                    disabled={!ready}
                    onChange={(e) => setSaveName(String(e.value))}
                  />
                  <UI.PushButton
                    id="easing:save"
                    label="Save"
                    disabled={!ready || !saveName.trim()}
                    onClick={() =>
                      act({ type: "save", name: saveName.trim(), curve })
                    }
                  />
                </div>
              </UI.InspectorRow>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
