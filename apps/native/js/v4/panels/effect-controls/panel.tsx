import { useLayoutEffect, useState } from "preact/hooks";
import * as UI from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";
import { EffectsController } from "./controller";
import {
  PANEL_ID,
  PANEL_TITLE,
  type Projection,
  type EffectHeader,
  type ParameterRow,
  type Intent,
} from "./contract";
const t = UI.token;
const hit = { minWidth: t("metrics.hit"), minHeight: t("metrics.hit") };
const stop = (e: SurfaceEvent) => e.stopPropagation();
export interface PanelProps {
  controller: EffectsController;
  /** Must be resolved from pm-design Tokens.metrics.row by the native worker. */
  rowHeight: number;
  increaseContrast?: boolean;
  reduceMotion?: boolean;
  /** Native focus ownership, never inferred from a pointer-generated focus event. */
  keyboardFocus?: boolean;
}
interface Drag {
  id?: string;
  definition?: string;
  slot: number;
}

export function EffectControls(props: PanelProps) {
  const { controller: c } = props;
  const [p, update] = useState(c.snapshot());
  const [hover, setHover] = useState<string | null>(null);
  const [keyboard, setKeyboard] = useState(false);
  const [drag, setDrag] = useState<Drag | null>(null);
  useLayoutEffect(() => c.subscribe(() => update(c.snapshot())), [c]);
  useLayoutEffect(() => {
    setDrag(null);
  }, [p.layer, p.locked]);
  if (!Number.isFinite(props.rowHeight) || props.rowHeight <= 0)
    throw new TypeError("Native row token required");
  const editable = p.target === "Layer" && !p.locked;
  const send = (intent: Intent) => {
    if (editable) c.send(intent);
  };
  const add = () => {
    void c.add().catch((error) => c.platform.report(error));
  };
  const focused = p.rows.find(
    (r) => r.kind === "effect" && r.id === p.focus,
  ) as EffectHeader | undefined;
  const key = (e: SurfaceEvent) => {
    setKeyboard(true);
    // Nested native controls consume their own keys before this list route.
    const k = e.key?.toLowerCase();
    let intent: Intent | undefined;
    if (k === "escape") {
      e.stopPropagation();
      setDrag(null);
      send({ type: "clear" });
      c.platform.focus(`${PANEL_ID}:list`);
      return;
    }
    if (!editable || !p.focus) return;
    if (k === "arrowup" || k === "arrowdown") {
      const direction = k === "arrowup" ? -1 : 1;
      intent = e.altKey
        ? { type: "reorder", id: p.focus, direction }
        : { type: "navigate", direction, shift: !!e.shiftKey };
    } else if ((e.metaKey || e.ctrlKey) && k === "c")
      intent = { type: "copy", id: p.focus };
    else if ((e.metaKey || e.ctrlKey) && k === "v" && p.clipboardCount)
      intent = { type: "paste" };
    else if (k === "backspace" || k === "delete")
      intent = { type: "remove", id: p.focus };
    else if (k === "arrowright" || k === "arrowleft")
      intent = { type: "disclose", id: p.focus, open: k === "arrowright" };
    else if (k === " ") intent = { type: "toggle", id: p.focus };
    else if (k === "enter") {
      const parameter = p.rows.find(
        (r) => r.kind === "parameter" && r.parameter.effect === p.focus,
      );
      if (parameter) c.platform.focus(`${PANEL_ID}:field:${parameter.id}`);
      e.stopPropagation();
      return;
    } else return;
    e.stopPropagation();
    if (intent) send(intent);
  };
  const dragOver = (e: SurfaceEvent, slot: number) => {
    if (!editable) return;
    const data = e.drag?.data;
    // Stack reorders originate in this panel. External payload ids cannot
    // accidentally address a same-named effect in another project/layer.
    const id = drag?.id;
    const definition =
      typeof data?.["application/x-powermove-effect-definition"] === "string"
        ? (data["application/x-powermove-effect-definition"] as string)
        : undefined;
    if (id || definition || drag?.id) {
      e.stopPropagation();
      setDrag({ id: id ?? drag?.id, definition, slot });
    }
  };
  const drop = (e: SurfaceEvent) => {
    stop(e);
    if (editable && drag) {
      if (drag.id) send({ type: "drop", id: drag.id, slot: drag.slot });
      else if (drag.definition)
        send({ type: "add", definition: drag.definition, slot: drag.slot });
    }
    setDrag(null);
  };
  const trailingAdd = (
    <UI.PushButton
      id={`${PANEL_ID}:add-bottom`}
      label="Add Effect…"
      disabled={!editable}
      onClick={add}
      style={{ height: t("metrics.control"), marginTop: t("metrics.inset") }}
    >
      Add Effect…
    </UI.PushButton>
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
        ...(props.increaseContrast
          ? {
              borderWidth: t("metrics.border"),
              borderColor: t("colors.separator"),
            }
          : {}),
      }}
      onClick={() => send({ type: "clear" })}
      onPointerMove={() => setHover(null)}
      onBlur={() => {
        setHover(null);
        setKeyboard(false);
      }}
    >
      <div
        role="group"
        label={PANEL_TITLE}
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          height: t("metrics.header"),
          paddingLeft: t("metrics.inset"),
        }}
      >
        <UI.SegmentedControl
          label={PANEL_TITLE}
          value={PANEL_ID}
          options={[
            { id: "assets", label: "Project" },
            { id: PANEL_ID, label: PANEL_TITLE },
          ]}
          onChange={(id) => {
            if (id === "assets") c.platform.activatePanel("assets");
          }}
        />
        {p.target === "Layer" && (
          <UI.ToolbarItem
            id={`${PANEL_ID}:add`}
            label="Add Effect"
            title="Add Effect"
            disabled={!editable}
            attr={{ symbol: "plus" }}
            onClick={add}
            style={{ symbolSize: t("metrics.symbol_panel") }}
          />
        )}
      </div>
      {p.layer && (
        <div
          role="heading"
          label={`${p.name} · ${p.typeLabel}`}
          title={`${p.name} · ${p.typeLabel}`}
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: t("metrics.gap"),
            height: t("metrics.row"),
            paddingLeft: t("metrics.inset"),
            paddingRight: t("metrics.inset"),
            color: t(p.locked ? "colors.muted" : "colors.text"),
            fontSize: t("typography.title"),
            fontWeight: t("typography.semibold_weight"),
          }}
        >
          <text>{p.name}</text>
          <text
            style={{
              color: t("colors.muted"),
              fontWeight: t("typography.regular_weight"),
            }}
          >{`· ${p.typeLabel}`}</text>
          {p.locked && (
            <text
              attr={{ symbol: "lock" }}
              title="Locked layers can’t be edited."
            />
          )}
        </div>
      )}
      {p.target !== "Layer" ? (
        <UI.EmptyState
          label={
            p.target === "NoProject"
              ? "No Composition"
              : p.target === "NoSelection"
                ? "No Selection"
                : "No Effects"
          }
          explanation={
            p.target === "NoProject"
              ? "Open a project to start editing."
              : p.target === "NoSelection"
                ? "Select a layer to see its effects."
                : "This kind of layer can’t have effects."
          }
          attr={{ symbol: "wand.and.rays" }}
          action={null}
        />
      ) : p.count === 0 ? (
        <UI.EmptyState
          label="No Effects"
          explanation="Drag an effect here from Effects, or add one."
          attr={{ symbol: "wand.and.rays" }}
          action={trailingAdd}
          onDragOver={(e) => dragOver(e, 0)}
          onDrop={drop}
        />
      ) : (
        <>
          <UI.List
            id={`${PANEL_ID}:list`}
            label="Effects"
            style={hit}
            tabIndex={p.locked ? -1 : 0}
            disabled={p.locked}
            metadata={{
              multiselectable: true,
              rowCount: p.count,
              effectCount: p.effectCount,
              focus: p.focus,
            }}
            onFocus={() => setKeyboard(props.keyboardFocus === true)}
            onPointerDown={() => setKeyboard(false)}
            onKeyDown={key}
            onDragOver={(e) => dragOver(e, p.effectCount)}
            onDrop={drop}
          >
            <virtual-list
              count={p.count}
              range={p.range}
              itemSize={props.rowHeight}
              metadata={{
                tokenSource: "metrics.row",
                overscanViewports: 1,
                anchor: p.rows[0]?.id ?? null,
              }}
              onRangeChange={(e) => {
                if (e.range) {
                  const [start, end] = e.range,
                    length = end - start;
                  c.send({
                    type: "range",
                    range: [
                      Math.max(0, start - length),
                      Math.min(p.count, end + length),
                    ],
                  });
                }
              }}
            >
              {p.rows.map((row) =>
                row.kind === "action" ? (
                  <UI.PushButton
                    key={row.id}
                    label={row.label}
                    disabled={p.locked || row.readonly}
                    onClick={(e) => {
                      stop(e);
                      send({ type: "action", id: row.effect, index: row.index });
                    }}
                  />
                ) : row.kind === "parameter" ? (
                  <ParameterView
                    key={row.id}
                    row={row}
                    locked={p.locked}
                    controller={c}
                  />
                ) : (
                  <UI.ListItem
                    key={row.id}
                    id={`${PANEL_ID}:effect:${row.id}`}
                    label={`${row.label} effect, ${row.enabled ? "enabled" : "disabled"}, ${row.expanded ? "expanded" : "collapsed"}, ${row.ordinal + 1} of ${row.effectCount}`}
                    tabIndex={-1}
                    disabled={p.locked}
                    selected={row.selected}
                    onSelect={() => {}}
                    metadata={{
                      effect: row.id,
                      actions: [
                        "Press",
                        row.expanded ? "Collapse" : "Expand",
                        "Delete",
                        "Move Up",
                        "Move Down",
                        "Reset",
                      ],
                      focused: keyboard && p.focus === row.id,
                      dragging: drag?.id === row.id,
                      dragOpacity: drag?.id === row.id ? 0.5 : 1,
                      insertBefore: drag?.slot === row.ordinal,
                      reduceMotion: !!props.reduceMotion,
                    }}
                    onPointerMove={(e) => {
                      e.stopPropagation();
                      setHover(row.id);
                    }}
                    onPointerCancel={() => {
                      setHover(null);
                      setDrag(null);
                    }}
                    onClickCapture={(e) => {
                      if (e.target === e.currentTarget && editable) {
                        e.stopPropagation();
                        send({
                          type: "select",
                          id: row.id,
                          meta: !!e.metaKey,
                          ctrl: !!e.ctrlKey,
                          shift: !!e.shiftKey,
                        });
                      }
                    }}
                    onPointerDown={(e) => {
                      setKeyboard(false);
                      if (e.button === 2) {
                        e.stopPropagation();
                        void c
                          .menu(row.id)
                          .catch((error) => c.platform.report(error));
                      }
                    }}
                    onDragStart={(e) => {
                      if (editable && e.target === e.currentTarget)
                        setDrag({ id: row.id, slot: row.ordinal });
                    }}
                    onDragOver={(e) => dragOver(e, row.ordinal)}
                    onDragEnd={() => setDrag(null)}
                    style={{
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      justifyContent: "space-between",
                      color: t(
                        p.locked || !row.enabled
                          ? "colors.muted"
                          : "colors.text",
                      ),
                      fontSize: t("typography.title"),
                      fontWeight: t("typography.semibold_weight"),
                      borderRadius: t("metrics.control_radius"),
                      ...(row.ordinal ? { marginTop: t("metrics.gap") } : {}),
                      ...(hover === row.id && !row.selected && !p.locked
                        ? { background: t("colors.hover") }
                        : {}),
                      ...(props.increaseContrast && row.ordinal
                        ? {
                            borderTopWidth: t("metrics.border"),
                            borderColor: t("colors.separator"),
                          }
                        : {}),
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "row",
                        alignItems: "center",
                      }}
                    >
                      <UI.ToolbarItem
                        label={`${row.expanded ? "Collapse" : "Expand"} ${row.label}`}
                        tabIndex={-1}
                        disabled={p.locked}
                        aria-expanded={row.expanded}
                        attr={{
                          symbol: "chevron.right",
                          expanded: row.expanded,
                        }}
                        onClick={(e) => {
                          stop(e);
                          send({
                            type: "disclose",
                            id: row.id,
                            open: !row.expanded,
                          });
                        }}
                        style={{ symbolSize: t("metrics.symbol_inline") }}
                      />
                      <UI.Checkbox
                        preventDefault={{ click: true }}
                        label={`Enable ${row.label}`}
                        binding={`${PANEL_ID}:enable:${row.id}`}
                        value={row.enabled}
                        disabled={p.locked}
                        tabIndex={-1}
                        onClick={(e) => {
                          stop(e);
                          send({ type: "toggle", id: row.id });
                        }}
                        onKeyDown={stop}
                      />
                      <text
                        onClick={(e) => {
                          stop(e);
                          send({
                            type: "select",
                            id: row.id,
                            meta: !!e.metaKey,
                            ctrl: !!e.ctrlKey,
                            shift: !!e.shiftKey,
                          });
                        }}
                      >
                        {row.label}
                      </text>
                      {row.missing && (
                        <text
                          attr={{ symbol: "exclamationmark.triangle.fill" }}
                          style={{
                            color: t("colors.warning"),
                            fontSize: t("typography.minimum"),
                          }}
                        >
                          Not installed
                        </text>
                      )}
                    </div>
                    {(hover === row.id ||
                      row.selected ||
                      (keyboard && p.focus === row.id)) && (
                      <div
                        style={{
                          display: "flex",
                          flexDirection: "row",
                          alignItems: "center",
                        }}
                      >
                        <UI.ToolbarItem
                          label={`Reset ${row.label}`}
                          title="Reset Effect"
                          disabled={p.locked || row.missing}
                          tabIndex={-1}
                          attr={{ symbol: "arrow.counterclockwise" }}
                          onClick={(e) => {
                            stop(e);
                            send({ type: "reset", id: row.id });
                          }}
                          style={{ symbolSize: t("metrics.symbol_inline") }}
                        />
                        <UI.ToolbarItem
                          label={`Remove ${row.label}`}
                          title="Remove Effect"
                          disabled={p.locked}
                          tabIndex={-1}
                          attr={{ symbol: "xmark" }}
                          onClick={(e) => {
                            stop(e);
                            send({ type: "remove", id: row.id });
                          }}
                          style={{ symbolSize: t("metrics.symbol_inline") }}
                        />
                      </div>
                    )}
                  </UI.ListItem>
                ),
              )}
            </virtual-list>
          </UI.List>
          {drag && (
            <div
              metadata={{ insertionSlot: drag.slot, insertionLine: true }}
              style={{ background: t("colors.accent") }}
            />
          )}
          {trailingAdd}
        </>
      )}
    </div>
  );
}
/** Read-only text for a value without a registry control (design review T4). */
function scalar(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "boolean") return value ? "On" : "Off";
  return undefined;
}
function ParameterView({
  row,
  locked,
  controller: c,
}: {
  key?: string;
  row: ParameterRow;
  locked: boolean;
  controller: EffectsController;
}) {
  const p = row.parameter;
  const disabled = locked || p.readonly;
  const common = {
    id: `${PANEL_ID}:field:${row.id}`,
    label: p.label,
    binding: p.binding,
    disabled,
    mixed: p.mixed,
    onKeyDown: stop,
    onClick: stop,
  };
  let field;
  if (p.control === "number")
    field = (
      <UI.NumberField
        {...common}
        value={p.value as number}
        step={p.step}
        min={p.min}
        max={p.max}
        precision={p.precision}
        unit={p.unit}
      />
    );
  else if (p.control === "toggle")
    field = <UI.Checkbox {...common} value={p.value as boolean} />;
  else if (p.control === "color")
    field = <UI.ColourWell {...common} value={p.value as string} />;
  else if (p.control === "text")
    field = <UI.TextField {...common} value={p.value as string} />;
  else if (p.control === "choice")
    field = (
      <UI.PopUpButton {...common} value={p.value} options={p.options ?? []} />
    );
  else
    field = (
      <text
        label={`${p.label}, ${scalar(p.value) ?? "custom value"}, read only`}
        style={{
          color: t(
            scalar(p.value) === undefined ? "colors.muted" : "colors.secondary",
          ),
          fontSize: t("typography.body"),
        }}
      >
        {scalar(p.value) ?? "Custom value"}
      </text>
    );
  const state = p.keyAtPlayhead
    ? "keyframe at playhead"
    : p.animated
      ? "animated"
      : "not animated";
  return (
    <div role="group" label={row.effectLabel} onClick={stop} onKeyDown={stop}>
      <UI.InspectorRow
        label={p.label}
        metadata={{
          parameter: p.id,
          indentToken: "metrics.tree_indent",
          valueColumn: "panel",
        }}
        style={{
          color: t(
            locked || !row.enabled ? "colors.muted" : "colors.secondary",
          ),
        }}
        leading={
          <UI.ToolbarItem
            label={`Keyframe, ${p.label}`}
            title={state}
            disabled={disabled}
            aria-pressed={p.keyAtPlayhead}
            attr={{
              symbol: p.keyAtPlayhead
                ? "diamond.fill"
                : p.animated
                  ? "diamond.lefthalf.filled"
                  : "diamond",
              value: state,
            }}
            onClick={(e) => {
              stop(e);
              if (!disabled)
                c.send({
                  type: "parameter",
                  id: p.id,
                  action: e.altKey ? "stopwatch" : "diamond",
                });
            }}
            style={{
              ...hit,
              color: t(p.animated ? "colors.accent" : "colors.muted"),
              symbolSize: t("metrics.keyframe"),
            }}
          />
        }
      >
        {field}
      </UI.InspectorRow>
    </div>
  );
}
