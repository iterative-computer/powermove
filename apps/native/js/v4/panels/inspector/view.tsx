import { useLayoutEffect, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import {
  Checkbox,
  ColourWell,
  DisclosureGroup,
  EmptyState,
  InspectorRow,
  NumberField,
  PopUpButton,
  SegmentedControl,
  TextField,
  ToolbarItem,
  token,
} from "../../src/components";
import type { SurfaceProps } from "../../src/jsx-runtime";
import type { SurfaceEvent } from "../../src/protocol";
import type { Alignment, Binding, MenuItem, Row, RowAction } from "./contracts";
import { InspectorSession } from "./session";

const well = {
  background: token("colors.field"),
  borderRadius: token("metrics.control_radius"),
};
const column = { display: "flex", flexDirection: "column" };
const horizontal = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
};
export const alignmentButtons: {
  id: Alignment;
  label: string;
  symbol: string;
}[] = [
  { id: "left", label: "Align Left", symbol: "align.horizontal.left" },
  {
    id: "hcenter",
    label: "Align Horizontal Centres",
    symbol: "align.horizontal.center",
  },
  { id: "right", label: "Align Right", symbol: "align.horizontal.right" },
  { id: "top", label: "Align Top", symbol: "align.vertical.top" },
  {
    id: "vcenter",
    label: "Align Vertical Centres",
    symbol: "align.vertical.center",
  },
  { id: "bottom", label: "Align Bottom", symbol: "align.vertical.bottom" },
];
export const newLayerItems = [
  "Text",
  "Shape",
  "Solid",
  "Adjustment Layer",
  "Null",
  null,
  "Import Media…",
];
export function rowMenu(row: Row): MenuItem[] {
  const enabled = !row.locked && !row.disabled;
  return [
    {
      label: "Add Keyframe at Playhead",
      action: "add-key",
      enabled: enabled && !!row.animation,
    },
    {
      label: "Remove Animation",
      action: "remove-animation",
      enabled: enabled && !!row.animation && row.animation !== "static",
    },
    {
      label: row.expression ? "Edit Expression…" : "Add Expression…",
      action: "edit-expression",
      enabled: enabled && !!row.animation,
      separatorBefore: true,
    },
    {
      label: "Remove Expression",
      action: "remove-expression",
      enabled: enabled && !!row.expression,
    },
    { label: "Reset", action: "reset", enabled, separatorBefore: true },
    {
      label: "Show in Graph Editor",
      action: "show-graph",
      enabled: !!row.animation,
    },
    ...(row.linked === undefined
      ? []
      : [
          {
            label: "Link Scale Axes",
            action: "link" as RowAction,
            enabled,
            checked: row.linked,
          },
        ]),
  ];
}
function activate(event: SurfaceEvent, run: () => void) {
  if (event.key === "Enter" || event.key === " ") {
    event.stopPropagation();
    run();
  }
}
/** Keyboard callbacks are for v4 semantic buttons only. Native bound fields are
 * driven by Rust Controls/Bindings; duplicating their key routes here edits twice.
 */
function Action(props: {
  key?: string;
  id: string;
  label: string;
  disabled?: boolean;
  active?: boolean;
  onActivate: (event: SurfaceEvent) => void;
  value?: string;
  children?: ComponentChildren;
  tooltip?: string;
  symbol?: string;
  tabIndex?: number;
  style?: SurfaceProps["style"];
}) {
  const { onActivate, tooltip, symbol, ...rest } = props;
  return (
    <ToolbarItem
      {...rest}
      metadata={{ symbol: symbol ?? (props.id === "inspector:add-effect" ? "wand.and.rays" : props.id === "inspector:new-layer" ? "plus" : props.id.startsWith("key:") ? "diamond" : "") }}
      title={tooltip ?? props.label}
      onClick={(event) => {
        if (!props.disabled) onActivate(event);
      }}
      preventDefault={{ keydown: ["Enter", " "] }}
      onKeyDown={(event) => {
        if (!props.disabled) activate(event, () => onActivate(event));
      }}
    />
  );
}
function AlignmentStrip({
  session,
}: {
  session: InspectorSession;
  key?: string;
}) {
  const [focused, setFocused] = useState(0);
  const [focusRequest, setFocusRequest] = useState<string>();
  const target = session.inspect().target;
  if (target.kind !== "layers" || target.first.audio) return null;
  return (
    <div
      role="group"
      label="Align"
      focusRequest={focusRequest}
      style={{
        ...horizontal,
        gap: token("metrics.gap"),
        height: token("metrics.row"),
        paddingLeft: token("metrics.inset"),
        paddingRight: token("metrics.inset"),
      }}
      preventDefault={{ keydown: ["ArrowLeft", "ArrowRight"] }}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.stopPropagation();
        const next =
          (focused +
            (event.key === "ArrowLeft" ? -1 : 1) +
            alignmentButtons.length) %
          alignmentButtons.length;
        setFocused(next);
        setFocusRequest(`align:${alignmentButtons[next].id}`);
      }}
    >
      {[alignmentButtons.slice(0, 3), alignmentButtons.slice(3)].map(
        (group, index) => (
          <div key={index} style={horizontal}>
            {group.map((button) => (
              <Action
                key={button.id}
                id={`align:${button.id}`}
                label={button.label}
                symbol={button.symbol}
                disabled={target.first.locked}
                tabIndex={alignmentButtons[focused].id === button.id ? 0 : -1}
                onActivate={() =>
                  session.send({ type: "align", alignment: button.id })
                }
              />
            ))}
          </div>
        ),
      )}
    </div>
  );
}
function Field({
  row,
  session,
}: {
  row: Row;
  session: InspectorSession;
  key?: string;
}) {
  const control = row.control;
  const blocked = !!row.disabled || !!row.locked;
  const disabled = (binding: Binding, local?: boolean) =>
    blocked || local || !session.ready(binding.handle);
  const fields =
    control.kind === "number" || control.kind === "point" ? control.axes : null;
  const common = { id: `value:${row.id}`, label: row.label, disabled: blocked };
  const style: NonNullable<SurfaceProps["style"]> = row.expression
    ? { color: token("colors.accent_text") }
    : {};
  if (fields)
    return (
      <>
        {fields.map((axis, index) => (
          <NumberField
            key={axis.axis ?? index}
            id={`value:${row.id}:${axis.axis ?? index}`}
            label={axis.axis ? `${row.label} ${axis.axis}` : row.label}
            binding={axis.binding.handle}
            value={axis.value}
            mixed={axis.mixed}
            disabled={disabled(axis.binding, axis.disabled)}
            unit={axis.unit}
            step={axis.step}
            precision={axis.precision}
            min={axis.min}
            max={axis.max}
            style={style}
          />
        ))}
        {control.kind === "point" ? (
          <Action
            id={`pad:${row.id}`}
            label={`${row.label} Pad`}
            disabled={blocked}
            onActivate={(event) =>
              session.nativeRequest({
                type: "point-pad",
                row: row.id,
                anchor: event.currentTarget,
              })
            }
          />
        ) : null}
      </>
    );
  // Narrow the number/point union explicitly for strict TypeScript.
  if ("axes" in control) return null;
  const bound = {
    ...common,
    binding: control.binding.handle,
    mixed: control.mixed,
    disabled: disabled(control.binding, control.disabled),
    style,
  };
  switch (control.kind) {
    case "text":
      // API-GAP I1: kit has no multiline text/line-limit recipe. Keep
      // the authored text in the existing native text component pending that API.
      return <TextField {...bound} value={control.value} />;
    case "choice":
      return (
        <PopUpButton
          {...bound}
          value={control.value}
          options={control.options}
        />
      );
    case "font":
      // Font search/menu is a host request, never substitute an ordinary menu.
      return (
        <Action
          {...common}
          disabled={bound.disabled}
          style={well}
          children={
            typeof control.value === "string" && control.value
              ? control.value
              : null
          }
          onActivate={(event) =>
            session.nativeRequest({
              type: "font-menu",
              row: row.id,
              anchor: event.currentTarget,
            })
          }
        />
      );
    case "toggle":
      return <Checkbox {...bound} value={control.value} />;
    case "colour":
      return (
        <>
          <ColourWell {...bound} value={control.value} />
          {control.hexBinding && control.hex !== undefined ? (
            <TextField
              id={`hex:${row.id}`}
              label={`${row.label} Hex`}
              binding={control.hexBinding.handle}
              value={control.hex}
              disabled={disabled(control.hexBinding)}
              style={{ fontVariantNumeric: "tabular-nums" }}
            />
          ) : null}
        </>
      );
    case "fill":
      // API-GAP I2: no kit Fill recipe; preserve structured gradient
      // data in the model and use a neutral host-popover trigger, never flatten.
      return (
        <Action
          {...common}
          disabled={bound.disabled}
          style={well}
          children={control.description}
          onActivate={(event) =>
            session.nativeRequest({
              type: "fill-popover",
              row: row.id,
              anchor: event.currentTarget,
            })
          }
        />
      );
    case "segmented":
      return (
        <SegmentedControl
          {...common}
          disabled={bound.disabled}
          mixed={control.mixed}
          value={control.value}
          options={control.options}
          onChange={(value) =>
            session.send({ type: "set-value", row: row.id, value })
          }
        />
      );
  }
}
function Property({
  row,
  session,
}: {
  row: Row;
  session: InspectorSession;
  key?: string;
}) {
  const disabled = !!row.locked || !!row.disabled;
  const keyValue =
    row.animation === "key-at-playhead"
      ? "keyframe at playhead"
      : row.animation === "animated"
        ? "animated"
        : "not animated";
  const requestMenu = (event: SurfaceEvent) =>
    session.nativeRequest({
      type: "row-menu",
      anchor: event.currentTarget,
      row: row.id,
      items: rowMenu(row),
    });
  return (
    <div
      id={`row:${row.id}`}
      onPointerDown={(event) => {
        if (event.button === 2) requestMenu(event);
      }}
    >
      <InspectorRow
        label={row.label}
        title={row.locked ? "Locked layers can’t be edited." : row.label}
        leading={
          row.animation ? (
            <Action
              id={`key:${row.id}`}
              label={`Keyframe, ${row.label}`}
              value={keyValue}
              disabled={disabled}
              active={row.animation !== "static"}
              onActivate={(event) =>
                session.send({
                  type: "key",
                  row: row.id,
                  removeAnimation: !!event.altKey,
                })
              }
            />
          ) : null
        }
        trailing={
          row.linked !== undefined ? (
            <Action
              id={`link:${row.id}`}
              label="Lock aspect ratio"
              tooltip={row.linked ? "Unlock Aspect Ratio" : "Lock Aspect Ratio"}
              active={row.linked}
              disabled={disabled}
              onActivate={() =>
                session.send({
                  type: "row-action",
                  row: row.id,
                  action: "link",
                })
              }
            />
          ) : null
        }
      >
        <Field row={row} session={session} />
      </InspectorRow>
      {row.expression ? (
        <InspectorRow label="Expression">
          <TextField
            id={`expression:${row.id}`}
            label={`Expression for ${row.label}`}
            binding={row.expression.binding.handle}
            value={row.expression.source}
            disabled={disabled || !session.ready(row.expression.binding.handle)}
            title={row.expression.error ?? "Expression is valid"}
          />
        </InspectorRow>
      ) : null}
      {row.expression?.error ? (
        <text
          role="status"
          style={{
            color: token("colors.danger"),
            fontSize: token("typography.minimum"),
          }}
        >
          {row.expression.error}
        </text>
      ) : null}
      {row.error ? (
        <text
          role="status"
          style={{
            color: token("colors.danger"),
            fontSize: token("typography.minimum"),
          }}
        >
          {row.error.kind === "number"
            ? "Enter a number."
            : `Enter a value from ${row.error.min} to ${row.error.max}.`}
        </text>
      ) : null}
    </div>
  );
}
export function InspectorPanel({
  session,
}: {
  session: InspectorSession;
  key?: string;
}) {
  const [, refresh] = useState(0);
  useLayoutEffect(
    () => session.subscribe(() => refresh((value) => value + 1)),
    [session],
  );
  const snapshot = session.inspect();
  const target = snapshot.target;
  const [drop, setDrop] = useState(false);
  const canDrop =
    target.kind === "layers" &&
    target.first.acceptsEffects &&
    !target.first.locked;
  const accepts = (event: SurfaceEvent) =>
    canDrop &&
    event.drag?.types.includes("application/x-powermove-effect") &&
    event.drag.data !== undefined;
  const content: ComponentChildren =
    target.kind === "no-project" ? (
      <EmptyState
        label="No Composition"
        explanation="Open a project to start editing."
        attr={{ symbol: "sidebar.trailing" }}
        action={null}
      />
    ) : (
      <>
        {target.kind === "layers" ? (
          <>
            <text
              role="heading"
              label={
                target.count > 1
                  ? `${target.count} layers selected`
                  : `${target.first.name}, ${target.first.typeLabel.toLowerCase()} layer`
              }
              title={target.first.name}
              style={{
                height: token("metrics.row"),
                paddingLeft: token("metrics.inset"),
                paddingRight: token("metrics.inset"),
                color: token("colors.text"),
                fontSize: token("typography.title"),
                fontWeight: token("typography.semibold_weight"),
              }}
            >
              {target.count > 1 ? `${target.count} Layers` : target.first.name}
              <text
                style={{
                  color: token("colors.muted"),
                  fontSize: token("typography.title"),
                }}
              >{` · ${target.count > 1 ? (target.sharedType ?? "Mixed") : target.first.typeLabel}`}</text>
              {target.first.locked ? (
                <text
                  attr={{ symbol: "lock" }}
                  title="Locked layers can’t be edited."
                  style={{
                    color: token("colors.muted"),
                    symbolSize: token("metrics.symbol_inline"),
                  }}
                />
              ) : null}
            </text>
            <AlignmentStrip key={target.key} session={session} />
          </>
        ) : null}
        {snapshot.sections.map((section) => (
          <DisclosureGroup
            key={section.id}
            id={`section:${section.id}`}
            label={section.title}
            open={!section.collapsed}
            onChange={(open) =>
              session.send({
                type: "collapse",
                section: section.id,
                collapsed: !open,
              })
            }
          >
            {section.action ? (
              <Action
                id={`section-action:${section.id}`}
                label={
                  section.action === "mask" ? "Add Mask" : "Add Text Animator"
                }
                onActivate={() =>
                  session.send({
                    type: "section-action",
                    section: section.id,
                    action: section.action!,
                  })
                }
              />
            ) : null}
            {section.rows.map((row) => (
              <Property key={row.id} row={row} session={session} />
            ))}
          </DisclosureGroup>
        ))}
        {target.kind === "composition" ? (
          <text
            style={{
              color: token("colors.muted"),
              fontSize: token("typography.minimum"),
              paddingLeft: token("metrics.inset"),
              paddingRight: token("metrics.inset"),
              paddingTop: token("metrics.gap"),
            }}
          >
            Select a layer to edit its properties.
          </text>
        ) : null}
      </>
    );
  return (
    <div
      id="inspector"
      role="group"
      label="Properties"
      style={{
        ...column,
        background: token("colors.panel"),
        borderRadius: token("metrics.panel_radius"),
      }}
    >
      <div
        id="inspector:header"
        style={{
          ...horizontal,
          justifyContent: "space-between",
          height: token("metrics.header"),
          paddingLeft: token("metrics.inset"),
          paddingRight: token("metrics.inset"),
        }}
      >
        <text
          style={{
            color: token("colors.text"),
            fontSize: token("typography.title"),
            fontWeight: token("typography.semibold_weight"),
          }}
        >
          Properties
        </text>
        <div style={horizontal}>
          {target.kind === "layers" && target.first.acceptsEffects ? (
            <Action
              id="inspector:add-effect"
              label="Add Effect"
              disabled={target.first.locked}
              onActivate={(event) =>
                session.nativeRequest({
                  type: "add-effect-menu",
                  anchor: event.currentTarget,
                  target: target.first.id,
                })
              }
            />
          ) : null}
          <Action
            id="inspector:new-layer"
            label="New Layer"
            disabled={target.kind === "no-project"}
            onActivate={(event) =>
              session.nativeRequest({
                type: "new-layer-menu",
                anchor: event.currentTarget,
                items: newLayerItems,
              })
            }
          />
        </div>
      </div>
      <scroll
        id="inspector:content"
        role="group"
        label="Properties"
        state={{ drop: drop && canDrop }}
        style={{
          ...column,
          ...(drop && canDrop
            ? { background: token("colors.accent_wash") }
            : {}),
        }}
        preventDefault={{ dragover: true, drop: true }}
        onDragOver={(event) => setDrop(!!accepts(event))}
        onDragLeave={() => setDrop(false)}
        onDrop={(event) => {
          setDrop(false);
          if (accepts(event) && target.kind === "layers")
            session.send({
              type: "drop-effect",
              effect: event.drag!.data!,
              target: target.first.id,
            });
        }}
      >
        {content}
      </scroll>
    </div>
  );
}
