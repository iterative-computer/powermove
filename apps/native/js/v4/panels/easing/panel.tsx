import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import {
  PANEL_ID,
  PANEL_TITLE,
  presetName,
  status,
  type Curve,
  type EasingAction,
  type EasingModel,
} from "./model";

const t = UI.token;
const stop = (e: SurfaceEvent) => e.stopPropagation();
const row = {
  display: "flex",
  flexDirection: "row",
  flexWrap: "wrap",
  alignItems: "center",
  gap: t("metrics.gap"),
  paddingLeft: t("metrics.inset"),
  paddingRight: t("metrics.inset"),
};

const pair = { ...row, paddingLeft: 0, paddingRight: 0, flexWrap: "nowrap" };

export interface EasingProps {
  model: EasingModel;
  /** Host-owned local fields: the drawn curve and its four numbers. */
  bindings: {
    curve: string;
    outX: string;
    outY: string;
    inX: string;
    inY: string;
  };
}

/** v1's Easing panel, on the native keyframe model: a curve you see and
 * drag (its two handles, as CSS writes `cubic-bezier`: the outgoing handle,
 * then the incoming, with the numbers beside it), the preset curves as
 * thumbnails, Easy Ease and the interpolations. Every action eases the
 * keyframes selected in the timeline as one Undo step; the Graph Editor
 * shows the result. */
export function EasingPanel({ model, bindings }: EasingProps) {
  const [s, update] = useState(model.snapshot());
  const [message, setMessage] = useState(model.message);
  useLayoutEffect(
    () =>
      model.subscribe(() => {
        update(model.snapshot());
        setMessage(model.message);
      }),
    [model],
  );
  const [curve, setCurve] = useState<Curve>([0.42, 0, 0.58, 1]);
  const [chosen, choose] = useState("easeInOut");
  const ready = s.open && s.count > 0;
  const act = (action: EasingAction) => {
    if (ready) model.send(action);
  };
  const number = (index: number, label: string, binding: string) => (
    <UI.NumberField
      id={`${PANEL_ID}:curve:${index}`}
      label={label}
      binding={binding}
      value={curve[index]!}
      step={0.01}
      precision={2}
      min={index % 2 === 0 ? 0 : -4}
      max={index % 2 === 0 ? 1 : 4}
      onKeyDown={stop}
      onChange={(e: SurfaceEvent) => {
        const value = Number(e.value);
        if (!Number.isFinite(value)) return;
        const next = [...curve] as Curve;
        next[index] = value;
        setCurve(next);
        choose("");
      }}
      style={{ flexGrow: 1 }}
    />
  );
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
      }}
    >
      <div
        id={`${PANEL_ID}:header`}
        data-pm-slot="header"
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          height: t("metrics.header"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
        }}
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
      <scroll
        id={`${PANEL_ID}:content`}
        role="group"
        label={PANEL_TITLE}
        style={{ display: "flex", flexDirection: "column" }}
      >
        <text
          id={`${PANEL_ID}:status`}
          role="status"
          label={status(s, message)}
          style={{
            paddingLeft: t("metrics.inset"),
            paddingRight: t("metrics.inset"),
            paddingBottom: t("metrics.gap"),
            color: t(message ? "colors.text" : "colors.muted"),
            fontSize: t("typography.minimum"),
          }}
        >
          {status(s, message)}
        </text>
        <UI.EaseCurve
          id={`${PANEL_ID}:curve`}
          label="Easing curve"
          title="Drag a handle to shape the curve (⇧ snaps, ⌥ fine)"
          binding={bindings.curve}
          value={curve}
          onKeyDown={stop}
          onChange={(e: SurfaceEvent) => {
            const next = Array.isArray(e.value) ? e.value.map(Number) : [];
            if (next.length !== 4 || !next.every(Number.isFinite)) return;
            setCurve(next as Curve);
            choose("");
          }}
          style={{
            marginLeft: t("metrics.inset"),
            marginRight: t("metrics.inset"),
            marginTop: t("metrics.gap"),
            marginBottom: t("metrics.gap"),
          }}
        />
        <UI.InspectorRow label="Outgoing">
          <div style={pair}>
            {number(0, "Outgoing X", bindings.outX)}
            {number(1, "Outgoing Y", bindings.outY)}
          </div>
        </UI.InspectorRow>
        <UI.InspectorRow label="Incoming">
          <div style={pair}>
            {number(2, "Incoming X", bindings.inX)}
            {number(3, "Incoming Y", bindings.inY)}
          </div>
        </UI.InspectorRow>
        <div
          role="group"
          label="Apply Curve"
          style={{ ...row, paddingTop: t("metrics.gap") }}
        >
          <UI.PushButton
            id={`${PANEL_ID}:apply`}
            label="Apply Curve"
            title={`Apply cubic-bezier(${curve.join(", ")}) to the selected keyframes`}
            role="default"
            disabled={!ready}
            onKeyDown={stop}
            onClick={(e: SurfaceEvent) => {
              stop(e);
              act({ type: "curve", curve });
            }}
          />
        </div>
        <Subheading title="Presets" />
        <div role="group" label="Presets" style={row}>
          {s.presets.map((p) => (
            <div
              key={p.id}
              id={`${PANEL_ID}:preset:${p.id}`}
              role="button"
              label={presetName(p.id)}
              title={`${presetName(p.id)}: cubic-bezier(${p.curve.join(", ")})`}
              aria-selected={chosen === p.id}
              onKeyDown={stop}
              onClick={(e: SurfaceEvent) => {
                stop(e);
                // The preset fills the editor, to start from, and eases the
                // selected keyframes.
                setCurve([...p.curve] as Curve);
                choose(p.id);
                act({ type: "preset", preset: p.id });
              }}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                width: t("metrics.value_column"),
                paddingTop: t("metrics.gap"),
                paddingBottom: t("metrics.gap"),
                borderRadius: t("metrics.control_radius"),
                ...(chosen === p.id
                  ? { background: t("colors.selected_fill") }
                  : {}),
              }}
            >
              <UI.EaseThumbnail
                id={`${PANEL_ID}:thumb:${p.id}`}
                label={presetName(p.id)}
                value={p.curve}
                style={{ width: 44, height: 32 }}
              />
              <text
                style={{
                  color: t(
                    chosen === p.id ? "colors.text" : "colors.secondary",
                  ),
                  fontSize: t("typography.minimum"),
                  textAlign: "center",
                }}
              >
                {presetName(p.id)}
              </text>
            </div>
          ))}
        </div>
        <Subheading title="Ease" />
        <div role="group" label="Ease" style={row}>
          {(
            [
              ["both", "Easy Ease", "F9"],
              ["in", "Ease In", "⇧F9"],
              ["out", "Ease Out", "⌘⇧F9"],
            ] as const
          ).map(([side, label, keys]) => (
            <UI.PushButton
              key={side}
              id={`${PANEL_ID}:ease:${side}`}
              label={label}
              title={`${label} (${keys})`}
              disabled={!ready}
              onKeyDown={stop}
              onClick={(e: SurfaceEvent) => {
                stop(e);
                act({ type: "easyEase", side });
              }}
            />
          ))}
        </div>
        <Subheading title="Interpolation" />
        <div
          role="group"
          label="Interpolation"
          style={{ ...row, paddingBottom: t("metrics.inset") }}
        >
          {(
            [
              ["linear", "Linear"],
              ["bezier", "Bézier"],
              ["auto", "Auto Bézier"],
              ["hold", "Hold"],
            ] as const
          ).map(([interpolation, label]) => (
            <UI.PushButton
              key={interpolation}
              id={`${PANEL_ID}:interpolation:${interpolation}`}
              label={label}
              title={`${label} interpolation on both sides`}
              disabled={!ready}
              onKeyDown={stop}
              onClick={(e: SurfaceEvent) => {
                stop(e);
                act({ type: "interpolation", interpolation, side: "both" });
              }}
            />
          ))}
        </div>
      </scroll>
    </div>
  );
}

function Subheading({ title }: { title: string }) {
  return (
    <text
      role="heading"
      style={{
        display: "flex",
        alignItems: "center",
        height: t("metrics.row"),
        paddingLeft: t("metrics.inset"),
        color: t("colors.muted"),
        fontSize: t("typography.minimum"),
        fontWeight: t("typography.semibold_weight"),
      }}
    >
      {title}
    </text>
  );
}
