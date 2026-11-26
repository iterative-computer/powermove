import { useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import {
  PANEL_ID,
  PANEL_TITLE,
  presetGroups,
  selectionKey,
  status,
  type Curve,
  type EasingAction,
  type EasingModel,
  type Interpolation,
  type PresetTile,
  type Side,
} from "./model";

const t = UI.token;
const stop = (e: SurfaceEvent) => e.stopPropagation();
const reset: Curve = [0, 0, 1, 1];
const sameCurve = (a: Curve, b: Curve, tolerance = 0) =>
  a.every((v, i) => Math.abs(v - b[i]!) <= tolerance);
const row = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
  gap: t("metrics.gap"),
};
const toolbar = { ...row, gap: t("space.hair") };
const options: { v: Interpolation; label: string }[] = [
  { v: "linear", label: "Linear" },
  { v: "bezier", label: "Bezier" },
  { v: "hold", label: "Hold" },
  { v: "auto", label: "Auto" },
  { v: "continuous", label: "Continuous" },
];

interface EasingProps {
  model: EasingModel;
  bindings: {
    curve: string;
    outX: string;
    inX: string;
    interpolation?: string;
    name?: string;
    groupName?: string;
  };
}

function Tool({
  id,
  label,
  symbol,
  action,
  active,
  disabled,
  expand = true,
}: {
  id: string;
  label: string;
  symbol: string;
  action: () => void;
  active?: boolean;
  disabled?: boolean;
  expand?: boolean;
}) {
  return (
    <UI.ToolbarItem
      id={`easing:${id}`}
      label={label}
      title={label}
      active={active}
      disabled={disabled}
      metadata={{ symbol }}
      onClick={() => {
        if (!disabled) action();
      }}
      style={{
        height: t("metrics.control"),
        flexGrow: expand ? 1 : 0,
        flexBasis: t("space.hair"),
        borderRadius: t("metrics.control_radius"),
      }}
    />
  );
}

export function EasingPanel({ model, bindings }: EasingProps) {
  const [s, update] = useState(model.snapshot());
  const [curve, setCurve] = useState<Curve>(s.curve ?? reset);
  const [undo, setUndo] = useState<Curve[]>([]);
  const [redo, setRedo] = useState<Curve[]>([]);
  const [clipboard, copy] = useState<Curve>();
  const [autoApply, setAutoApply] = useState(true);
  const [more, showMore] = useState(false);
  const [coordinates, showCoordinates] = useState(false);
  const [editor, showEditor] = useState<"preset" | "group">();
  const [name, setName] = useState("");
  const [activeGroup, setActiveGroup] = useState<string>();
  const [hover, preview] = useState<PresetTile>();
  const [hint, setHint] = useState("");
  const [message, setMessage] = useState(model.message);
  const previous = useRef(s);
  const observedCurve = useRef(s.curve ?? reset);
  const draftOnly = useRef(false);
  const gesture = useRef<Curve>();

  useLayoutEffect(
    () =>
      model.subscribe(() => {
        const next = model.snapshot();
        const before = previous.current;
        previous.current = next;
        const selectionChanged = selectionKey(next) !== selectionKey(before);
        update(next);
        setMessage(model.message);
        if (selectionChanged) {
          gesture.current = undefined;
          preview(undefined);
          setHint("");
          if (next.count) {
            observedCurve.current = next.curve ?? reset;
            // A curve drafted before choosing any keyframes can be applied
            // as soon as the user picks a target.
            if (before.count || !draftOnly.current) {
              draftOnly.current = false;
              setUndo([]);
              setRedo([]);
              setCurve(observedCurve.current);
            }
          }
        } else if (
          next.count &&
          !model.applying &&
          !draftOnly.current &&
          !gesture.current
        ) {
          const published = next.curve ?? reset;
          if (!sameCurve(published, observedCurve.current)) setCurve(published);
          observedCurve.current = published;
        }
      }),
    [model],
  );

  const ready = s.open && s.count > 0;
  const { groups, active } = useMemo(
    () => presetGroups(s),
    [s.library?.source, s.presets],
  );
  const group =
    groups.find((g) => g.id === (activeGroup ?? active)) ?? groups[0]!;
  const selected = group.presets.find((p) => sameCurve(p.curve, curve, 0.0011));
  const caption = hover
    ? `${hover.name} · ${Math.round(hover.curve[0] * 100)}% / ${Math.round((1 - hover.curve[2]) * 100)}%`
    : status(s, message || hint);
  const send = (action: EasingAction) => {
    setHint("");
    model.send(action);
  };
  const apply = (
    next: Curve,
    action: EasingAction = { type: "curve", curve: next },
  ) => {
    if (!ready) return;
    draftOnly.current = false;
    send(action);
  };
  const edit = (next: Curve, action?: EasingAction) => {
    const before = gesture.current ?? curve;
    gesture.current = undefined;
    setCurve(next);
    setHint("");
    const changed = !sameCurve(before, next);
    if (changed) {
      setUndo((history) => [...history.slice(-99), before]);
      setRedo([]);
    }
    if (ready && (action || (changed && autoApply))) apply(next, action);
    else if (changed) draftOnly.current = true;
  };
  const sample = (next: Curve) => {
    const before = gesture.current ?? curve;
    gesture.current = sameCurve(before, next) ? undefined : before;
    setCurve(next);
  };
  const historyStep = (forward: boolean) => {
    const history = forward ? redo : undo;
    const next = history.at(-1);
    if (!next) return;
    (forward ? setRedo : setUndo)(history.slice(0, -1));
    (forward ? setUndo : setRedo)((stack) => [...stack, curve]);
    gesture.current = undefined;
    setCurve(next);
    if (autoApply && ready) apply(next);
    else draftOnly.current = true;
  };
  const easyEase = (side: Side) => {
    const next: Curve = [
      side === "in" ? curve[0] : 1 / 3,
      side === "in" ? curve[1] : 0,
      side === "out" ? curve[2] : 2 / 3,
      side === "out" ? curve[3] : 1,
    ];
    edit(next, { type: "easyEase", side });
  };
  const selectPreset = (p: PresetTile) => {
    edit(
      p.curve,
      p.native
        ? { type: "preset", preset: p.id }
        : { type: "curve", curve: p.curve, name: p.name },
    );
  };
  const curveValue = (raw: unknown): Curve | undefined =>
    Array.isArray(raw) &&
    raw.length === 4 &&
    raw.every((n) => typeof n === "number" && Number.isFinite(n))
      ? (raw as Curve)
      : undefined;
  const influenceValue = (outgoing: boolean, raw: unknown, commit: boolean) => {
    if (raw === undefined || raw === null || raw === "") return;
    const value = Number(raw) / 100;
    if (!Number.isFinite(value) || value < 0 || value > 1) return;
    const before = gesture.current ?? curve;
    const index = outgoing ? 0 : 2;
    const end = outgoing ? 0 : 1;
    const fraction = before[index] - end;
    const speed = fraction ? (before[index + 1]! - end) / fraction : 0;
    const next = [...before] as Curve;
    next[index] = outgoing ? value : 1 - value;
    next[index + 1] = Math.max(
      -4,
      Math.min(4, end + speed * (next[index] - end)),
    );
    if (commit) edit(next);
    else sample(next);
  };
  const influence = (outgoing: boolean) => {
    const fraction = outgoing ? curve[0] : 1 - curve[2];
    const label = outgoing ? "Outgoing" : "Incoming";
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          flexGrow: 1,
          flexBasis: t("space.hair"),
        }}
      >
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
          step={1}
          precision={1}
          unit="%"
          min={0}
          max={100}
          onKeyDown={stop}
          metadata={{ commitOnRelease: true }}
          onInput={(e) => influenceValue(outgoing, e.value, false)}
          onChange={(e) => influenceValue(outgoing, e.value, true)}
        />
      </div>
    );
  };
  const toggleEditor = (which: "preset" | "group") => {
    if (editor === which) showEditor(undefined);
    else {
      setName(which === "preset" ? "Custom easing" : String(groups.length + 1));
      showEditor(which);
    }
  };
  const save = () => {
    const trimmed = name.trim();
    if (!s.open || !trimmed || [...trimmed].length > 100) return;
    if (editor === "preset")
      send({ type: "save", name: trimmed, group: group.id, curve });
    else {
      const id = `group-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      send({ type: "addGroup", id, name: trimmed });
      setActiveGroup(id);
    }
    showEditor(undefined);
  };

  const interpolationControl = (
    <UI.PopUpButton
      id="easing:interpolation"
      label="Interpolation"
      widthSample="Continuous"
      binding={bindings.interpolation ?? "interpolation"}
      value={s.interpolation ?? "linear"}
      disabled={!ready}
      options={options}
      onChange={(e) => {
        const interpolation = options.find(({ v }) => v === e.value)?.v;
        if (ready && interpolation)
          send({ type: "interpolation", interpolation, side: "both" });
      }}
    />
  );
  const nameEditor = (
    <div style={row}>
      <UI.TextField
        key={editor}
        id={editor === "preset" ? "easing:save-name" : "easing:group-name"}
        label={editor === "preset" ? "Preset name" : "Group name"}
        binding={
          editor === "preset"
            ? (bindings.name ?? "name")
            : (bindings.groupName ?? "groupName")
        }
        value={name}
        onKeyDown={stop}
        onInput={(e) => setName(String(e.value))}
        onChange={(e) => setName(String(e.value))}
        style={{ flexGrow: 1, flexBasis: t("space.hair") }}
      />
      <UI.PushButton
        id="easing:save"
        label={editor === "preset" ? "Save" : "Add"}
        disabled={!name.trim() || [...name.trim()].length > 100}
        onClick={save}
      />
    </div>
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
        flexShrink: 0,
      }}
    >
      <div
        id="easing:header"
        data-pm-slot="header"
        style={{
          ...row,
          height: t("metrics.header"),
          paddingLeft: t("metrics.inset"),
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
      <div
        id="easing:content"
        role="group"
        label="Easing"
        style={{
          display: "flex",
          flexDirection: "column",
          // The dock scrolls this body at its full content height.
          flexGrow: 1,
          flexShrink: 0,
          gap: t("metrics.gap"),
          paddingLeft: t("metrics.inset"),
          paddingRight: t("metrics.inset"),
          paddingBottom: t("metrics.inset"),
        }}
      >
        <div role="toolbar" label="Easing tools" style={toolbar}>
          <Tool
            id="more"
            label="More easing tools"
            symbol="ellipsis"
            active={more}
            action={() => showMore(!more)}
          />
          <Tool
            id="copy"
            label={
              clipboard ? "Copy easing (clipboard has a curve)" : "Copy easing"
            }
            symbol="square.on.square"
            active={!!clipboard}
            disabled={!ready || !s.curve}
            action={() => {
              if (s.curve) copy(s.curve);
              setHint("Easing copied");
            }}
          />
          <Tool
            id="paste"
            label="Paste easing"
            symbol="point.topleft.down.to.point.bottomright.curvepath"
            disabled={!ready || !clipboard}
            action={() => {
              if (clipboard)
                edit(clipboard, { type: "curve", curve: clipboard });
            }}
          />
          <Tool
            id="easy"
            label="Easy Ease"
            symbol="clock.fill"
            action={() => easyEase("both")}
          />
        </div>
        {more && (
          <div
            role="group"
            label="More easing tools"
            style={{
              display: "flex",
              flexDirection: "column",
              gap: t("metrics.gap"),
            }}
          >
            <UI.SegmentedControl
              id="easing:quick"
              label="Quick easing"
              value=""
              options={[
                { id: "both", label: "Easy Ease" },
                { id: "in", label: "Ease In" },
                { id: "out", label: "Ease Out" },
              ]}
              onChange={(value) => {
                if (value === "both" || value === "in" || value === "out")
                  easyEase(value);
              }}
            />
            {interpolationControl}
            <UI.PushButton
              id="easing:reset"
              label="Reset Curve"
              onClick={() => edit(reset)}
            />
          </div>
        )}
        <UI.EaseCurve
          id="easing:curve"
          label="Easing curve"
          binding={bindings.curve}
          value={curve}
          metadata={{
            commitOnRelease: true,
            pmKit: hover ? { preview: hover.curve } : {},
          }}
          onKeyDown={stop}
          onInput={(e) => {
            const next = curveValue(e.value);
            if (next) sample(next);
          }}
          onChange={(e) => {
            const next = curveValue(e.value);
            if (next) edit(next);
          }}
          style={{
            height: t("metrics.curve_editor"),
            minHeight: t("metrics.curve_editor"),
            maxHeight: t("metrics.curve_editor"),
            minWidth: t("metrics.hit"),
            flexShrink: 0,
          }}
        />
        <div role="toolbar" label="Graph tools" style={toolbar}>
          <Tool
            id="undo"
            label="Undo curve"
            symbol="arrow.uturn.backward"
            disabled={!undo.length}
            action={() => historyStep(false)}
          />
          <Tool
            id="redo"
            label="Redo curve"
            symbol="arrow.uturn.forward"
            disabled={!redo.length}
            action={() => historyStep(true)}
          />
          <Tool
            id="coordinates"
            label="Influence and interpolation"
            symbol="slider.horizontal.3"
            active={coordinates}
            action={() => showCoordinates(!coordinates)}
          />
          <Tool
            id="mirror"
            label="Mirror curve"
            symbol="arrow.clockwise"
            action={() =>
              edit([1 - curve[2], 1 - curve[3], 1 - curve[0], 1 - curve[1]])
            }
          />
          <Tool
            id="save-preset"
            label="Save easing preset"
            symbol="plus"
            active={editor === "preset"}
            disabled={!s.open}
            action={() => toggleEditor("preset")}
          />
          <Tool
            id="auto-apply"
            label="Auto apply"
            symbol="bolt.fill"
            active={autoApply}
            action={() => setAutoApply(!autoApply)}
          />
          <Tool
            id="apply"
            label="Apply curve"
            symbol="return"
            disabled={!ready}
            action={() => apply(curve)}
          />
        </div>
        {coordinates && (
          <div role="group" label="Influence" style={row}>
            {influence(true)}
            {influence(false)}
          </div>
        )}
        {coordinates && !more && interpolationControl}
        {editor === "preset" && nameEditor}
        <div
          role="tablist"
          label="Preset groups"
          style={{ ...row, flexWrap: "wrap" }}
        >
          {groups.map((g) => (
            <button
              key={g.id}
              id={`easing:group:${g.id}`}
              role="tab"
              label={g.name}
              title={g.name}
              aria-selected={g.id === group.id}
              onClick={() => {
                setActiveGroup(g.id);
                preview(undefined);
              }}
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                height: t("metrics.control"),
                minWidth: t("metrics.hit"),
                minHeight: t("metrics.hit"),
                paddingLeft: t("metrics.inset"),
                paddingRight: t("metrics.inset"),
                background: t(
                  g.id === group.id ? "colors.field" : "colors.panel",
                ),
                color: t(
                  g.id === group.id ? "colors.accent" : "colors.secondary",
                ),
                borderRadius: t("metrics.control_radius"),
              }}
            >
              {g.name}
            </button>
          ))}
          <Tool
            id="add-group"
            label="New preset group"
            symbol="plus"
            expand={false}
            active={editor === "group"}
            disabled={!s.open}
            action={() => toggleEditor("group")}
          />
        </div>
        {editor === "group" && nameEditor}
        <div
          role="group"
          label="Easing presets"
          style={{ display: "grid", gridTemplateColumns: 6, gap: t("space.xs") }}
        >
          {group.presets.map((p) => (
            <div
              key={p.id}
              id={`easing:preset:${p.id}`}
              role="button"
              label={p.name}
              title={p.name}
              aria-selected={selected?.id === p.id}
              tabIndex={0}
              preventDefault={{ keydown: ["Enter"] }}
              onPointerEnter={() => preview(p)}
              onPointerLeave={() => preview(undefined)}
              onFocus={() => preview(p)}
              onBlur={() => preview(undefined)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  stop(e);
                  selectPreset(p);
                }
              }}
              onClick={(e) => {
                stop(e);
                selectPreset(p);
              }}
              style={{
                display: "flex",
                flexDirection: "column",
                minWidth: t("metrics.hit"),
                minHeight: t("metrics.hit"),
                padding: t("space.hair"),
                borderRadius: t("metrics.control_radius"),
                background: t(
                  selected?.id === p.id
                    ? "colors.selected_fill"
                    : hover?.id === p.id
                      ? "colors.hover"
                      : "colors.panel",
                ),
              }}
            >
              <UI.EaseThumbnail
                id={`easing:thumb:${p.id}`}
                label={p.name}
                value={p.curve}
                style={{ height: t("metrics.control") }}
              />
            </div>
          ))}
        </div>
        <text
          id="easing:status"
          role="status"
          label={caption}
          style={{
            height: t("metrics.row"),
            color: t("colors.secondary"),
            fontSize: t("typography.minimum"),
            overflow: "hidden",
            whiteSpace: "nowrap",
            textOverflow: "ellipsis",
          }}
        >
          {caption}
        </text>
      </div>
    </div>
  );
}
