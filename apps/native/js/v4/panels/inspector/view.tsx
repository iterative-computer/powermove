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
import type {
  Alignment,
  Finish,
  Binding,
  MenuItem,
  Row,
  RowAction,
  SectionAction,
} from "./contracts";
import { InspectorSession } from "./session";

const well = {
  background: token("colors.field"),
  borderRadius: token("metrics.inner_radius"),
};
const menuWell = {
  ...well,
  color: token("colors.text"),
  fontSize: token("typography.body"),
};
/** What a well that opens a menu shows: its value, then the pop-up's
 * chevrons at the trailing end. */
function menuValue(value: string | null) {
  return [
    <text
      key="value"
      style={{
        flexGrow: 1,
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
      }}
    >
      {value ?? ""}
    </text>,
    <text
      key="chevrons"
      attr={{ symbol: "chevron.up.chevron.down" }}
      style={{
        color: token("colors.text"),
        symbolSize: token("metrics.symbol_inline"),
      }}
    />,
  ];
}
const problem = {
  color: token("colors.danger"),
  fontSize: token("typography.minimum"),
};
/** The gutter a row's keyframe mark stands in, kept by rows that have none,
 * so every label of the panel starts on one x. */
const gutter = <div role="presentation" />;
const column = { display: "flex", flexDirection: "column" };
const horizontal = {
  display: "flex",
  flexDirection: "row",
  alignItems: "center",
};
const sectionActions: Record<SectionAction, string> = {
  mask: "Add Mask",
  animator: "Add Text Animator",
  "release-precomp": "Release Pre-comp",
  "refresh-fonts": "Refresh Fonts",
  "set-up-studio": "Set Up Studio",
  "time-remap": "Time Remap",
  "freeze-frame": "Freeze Frame",
  reverse: "Reverse",
};
const sectionSymbols: Partial<Record<SectionAction, string>> = {
  "refresh-fonts": "arrow.clockwise",
  "set-up-studio": "light.max",
  // Time's three share the heading row: short words, no glyph, so they fit
  // beside "Time" in a narrow panel (their labels keep the full names).
  "time-remap": "",
  "freeze-frame": "",
  reverse: "",
};
/** What a heading action shows when its full name would not fit. */
const sectionShort: Partial<Record<SectionAction, string>> = {
  "time-remap": "Remap",
  "freeze-frame": "Freeze",
  reverse: "Reverse",
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
/** Equal gaps between three or more selected layers. */
export const distributeButtons: {
  id: "horizontal" | "vertical";
  label: string;
  symbol: string;
}[] = [
  {
    id: "horizontal",
    label: "Distribute Horizontally",
    symbol: "distribute.horizontal.center",
  },
  {
    id: "vertical",
    label: "Distribute Vertically",
    symbol: "distribute.vertical.center",
  },
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
      metadata={{
        symbol:
          symbol ??
          (props.id === "inspector:add-effect"
            ? "wand.and.rays"
            : props.id === "inspector:new-layer"
              ? "plus"
              : props.id.startsWith("key:")
                ? "diamond"
                : ""),
      }}
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
        minHeight: token("metrics.row"),
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
          // Two wells of three equal cells, sharing the row (v1's two
          // button groups).
          <div
            key={index}
            style={{
              ...horizontal,
              ...well,
              flexGrow: 1,
              // Cells keep the pointer's least target, so the strip's
              // wells are that high.
              height: token("metrics.hit"),
            }}
          >
            {group.map((button) => (
              <Action
                key={button.id}
                id={`align:${button.id}`}
                label={button.label}
                tooltip={`${button.label}\nTo composition`}
                symbol={button.symbol}
                style={{
                  flexGrow: 1,
                  symbolSize: token("metrics.symbol_inline"),
                }}
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
      {/* Equal gaps between three or more layers, the outermost staying
          put. The well keeps its place with fewer, its buttons dimmed. */}
      <div style={{ ...horizontal, ...well, gap: token("metrics.gap") }}>
        {distributeButtons.map((button) => (
          <Action
            key={button.id}
            id={`distribute:${button.id}`}
            label={button.label}
            tooltip={`${button.label}\nEqual gaps between three or more layers`}
            symbol={button.symbol}
            disabled={target.first.locked || target.count < 3}
            tabIndex={-1}
            onActivate={() =>
              session.send({ type: "distribute", axis: button.id })
            }
          />
        ))}
      </div>
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
  if (control.kind === "status")
    return (
      <text
        style={{
          color: token("colors.muted"),
          fontSize: token("typography.label"),
        }}
      >
        {control.value}
      </text>
    );
  const bound = {
    ...common,
    binding: control.binding.handle,
    mixed: control.mixed,
    disabled: disabled(control.binding, control.disabled),
    style,
  };
  switch (control.kind) {
    case "text":
      return (
        <TextField
          {...bound}
          value={control.value}
          multiline={control.multiline}
        />
      );
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
          // A well that opens a menu: the value in the text ink and the
          // pop-up's chevrons at its trailing end.
          style={menuWell}
          children={menuValue(
            typeof control.value === "string" && control.value
              ? control.value
              : null,
          )}
          onActivate={(event) =>
            session.nativeRequest({
              type: "font-menu",
              row: row.id,
              anchor: event.currentTarget,
              current:
                typeof control.value === "string" ? control.value : undefined,
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
          style={menuWell}
          children={menuValue(control.description)}
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
        // The label is its own tooltip when it is cut; the row says only
        // why it cannot be edited.
        title={row.locked ? "Locked layers can’t be edited." : undefined}
        // A multi-line editor grows its row; the label and diamond sit at its top.
        style={
          row.control.kind === "text" && row.control.multiline
            ? {
                height: null,
                minHeight: token("metrics.row"),
                alignItems: "start",
                paddingTop: token("metrics.inner_radius"),
                paddingBottom: token("metrics.inner_radius"),
              }
            : undefined
        }
        leading={
          row.animation ? (
            <Action
              id={`key:${row.id}`}
              label={`Keyframe, ${row.label}`}
              value={keyValue}
              disabled={disabled}
              // One small mark whose shape says the state, as the timeline's:
              // outline = static, half = animated, filled = key at playhead.
              symbol={
                row.animation === "key-at-playhead"
                  ? "diamond.fill"
                  : row.animation === "animated"
                    ? "diamond.lefthalf.filled"
                    : "diamond"
              }
              style={{
                symbolSize: token("metrics.symbol_inline"),
                color: token(
                  row.animation === "static" ? "colors.muted" : "colors.accent",
                ),
              }}
              onActivate={(event) =>
                session.send({
                  type: "key",
                  row: row.id,
                  removeAnimation: !!event.altKey,
                })
              }
            />
          ) : (
            gutter
          )
        }
        trailing={
          row.linked !== undefined ? (
            <Action
              id={`link:${row.id}`}
              // One glyph (the system has no slashed link: it drew
              // nothing); its ink says linked or not.
              symbol="link"
              label="Lock aspect ratio"
              tooltip={row.linked ? "Unlock Aspect Ratio" : "Lock Aspect Ratio"}
              value={row.linked ? "linked" : "not linked"}
              style={{
                color: token(row.linked ? "colors.secondary" : "colors.faint"),
                symbolSize: token("metrics.symbol_inline"),
              }}
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
      {row.expression || row.error ? (
        // Under its row, from the label's x to the panel's inset: the
        // expression in the fixed-width face, as many lines as it has, and
        // what is wrong with it or with the value. A valid one says nothing.
        <div data-pm-slot="detail" style={column}>
          {row.expression ? (
            <TextField
              id={`expression:${row.id}`}
              label={`Expression for ${row.label}`}
              binding={row.expression.binding.handle}
              value={row.expression.source}
              multiline
              metadata={{ code: true }}
              disabled={
                disabled || !session.ready(row.expression.binding.handle)
              }
            />
          ) : null}
          {row.expression?.error ? (
            <text role="status" style={problem}>
              {row.expression.error}
            </text>
          ) : null}
          {row.error ? (
            <text role="status" style={problem}>
              {row.error.kind === "number"
                ? "Enter a number."
                : `Enter a value from ${row.error.min} to ${row.error.max}.`}
            </text>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
/** 3d-studio.md §8 Finish shelf: `m.row` squares, `m.gap` apart, with a
 * hairline edge. Swatches are content, so they are the one coloured thing
 * here. No selected state: a finish is a starting point. One click, one Undo. */
function FinishShelf({
  finishes,
  disabled,
  session,
}: {
  finishes: Finish[];
  disabled: boolean;
  session: InspectorSession;
}) {
  // The kit has no wrapping row: split into balanced rows that fit the
  // narrowest docked inspector (six tiles of m.row plus gaps).
  const perRow = Math.ceil(finishes.length / Math.ceil(finishes.length / 6));
  const rows: (typeof finishes)[] = [];
  for (let i = 0; i < finishes.length; i += perRow)
    rows.push(finishes.slice(i, i + perRow));
  return (
    <div
      id="finish-shelf"
      role="group"
      label="Finish"
      style={{
        ...column,
        gap: token("metrics.gap"),
        paddingLeft: token("metrics.inset"),
        paddingRight: token("metrics.inset"),
        paddingTop: token("metrics.inner_radius"),
        paddingBottom: token("metrics.gap"),
      }}
    >
      {rows.map((row, index) => (
        <div
          key={`finish-row-${index}`}
          role="presentation"
          style={{ ...horizontal, gap: token("metrics.gap") }}
        >
          {row.map((finish) => (
            <div
              key={finish.id}
              role="presentation"
              style={{
                position: "relative",
                width: token("metrics.row"),
                height: token("metrics.row"),
                minWidth: token("metrics.row"),
              }}
            >
              {/* Rim, lit body up-left, highlight, and a coat's glint. */}
              <div
                role="presentation"
                style={{
                  position: "absolute",
                  inset: "fill",
                  borderRadius: token("metrics.row"),
                  background: finish.rim,
                  borderWidth: token("metrics.border"),
                  borderColor: token("colors.separator"),
                }}
              />
              <div
                role="presentation"
                style={{
                  position: "absolute",
                  left: "8%",
                  top: "6%",
                  width: finish.size,
                  height: finish.size,
                  borderRadius: token("metrics.row"),
                  background: finish.body,
                }}
              />
              <div
                role="presentation"
                style={{
                  position: "absolute",
                  left: "20%",
                  top: "18%",
                  width: finish.spot,
                  height: finish.spot,
                  borderRadius: token("metrics.row"),
                  background: finish.highlight,
                }}
              />
              {finish.glint ? (
                <div
                  role="presentation"
                  style={{
                    position: "absolute",
                    left: "26%",
                    top: "24%",
                    width: finish.glintSize,
                    height: finish.glintSize,
                    borderRadius: token("metrics.row"),
                    background: finish.glint,
                  }}
                />
              ) : null}
              <Action
                id={`finish:${finish.id}`}
                label={finish.label}
                tooltip={finish.label}
                disabled={disabled}
                style={{
                  position: "absolute",
                  inset: "fill",
                  borderRadius: token("metrics.row"),
                }}
                onActivate={() =>
                  session.send({
                    type: "set-value",
                    row: "3d.finish",
                    value: finish.id,
                  })
                }
              />
            </div>
          ))}
        </div>
      ))}
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
  // A section's own action: a quiet text action at the trailing edge of its
  // heading (design language, Properties).
  const sectionAction = (
    section: (typeof snapshot.sections)[number],
    action: SectionAction,
    several: boolean,
  ) => (
    <Action
      key={action}
      id={
        several
          ? `section-action:${section.id}:${action}`
          : `section-action:${section.id}`
      }
      label={sectionActions[action]}
      symbol={sectionSymbols[action] ?? "plus"}
      children={sectionShort[action] ?? sectionActions[action]}
      style={{
        gap: token("metrics.border"),
        paddingLeft: token("metrics.gap"),
        paddingRight: token("metrics.gap"),
        color: token("colors.muted"),
        fontSize: token("typography.label"),
        symbolSize: token("metrics.symbol_inline"),
      }}
      disabled={target.kind === "layers" && target.first.locked}
      onActivate={(event) =>
        action === "animator"
          ? session.nativeRequest({
              type: "animator-menu",
              anchor: event.currentTarget,
              items: section.presets ?? [],
            })
          : session.send({
              type: "section-action",
              section: section.id,
              action,
            })
      }
    />
  );
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
            <div
              role="heading"
              label={
                target.count > 1
                  ? `${target.count} layers selected`
                  : `${target.first.name}, ${target.first.typeLabel.toLowerCase().replace(/\b3d\b/g, "3D")} layer`
              }
              style={{
                ...horizontal,
                gap: token("metrics.inner_radius"),
                height: token("metrics.row"),
                minHeight: token("metrics.row"),
                // Clear of the header's rule.
                marginTop: token("metrics.pill_inset"),
                paddingLeft: token("metrics.inset"),
                paddingRight: token("metrics.inset"),
              }}
            >
              <text
                title={target.first.name}
                style={{
                  color: token("colors.text"),
                  fontSize: token("typography.label"),
                  fontWeight: token("typography.semibold_weight"),
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {target.count > 1
                  ? `${target.count} Layers`
                  : target.first.name}
              </text>
              <text
                style={{
                  color: token("colors.muted"),
                  fontSize: token("typography.label"),
                  whiteSpace: "nowrap",
                }}
              >
                {`· ${target.count > 1 ? (target.sharedType ?? "Mixed") : target.first.typeLabel}`}
              </text>
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
            </div>
            {target.count === 1 && target.first.error ? (
              <div
                role="status"
                label={target.first.error}
                style={{
                  ...horizontal,
                  alignItems: "start",
                  gap: token("metrics.gap"),
                  paddingLeft: token("metrics.inset"),
                  paddingRight: token("metrics.inset"),
                  paddingTop: token("metrics.inner_radius"),
                  paddingBottom: token("metrics.inner_radius"),
                }}
              >
                <text
                  attr={{ symbol: "exclamationmark.triangle.fill" }}
                  style={{
                    color: token("colors.danger"),
                    symbolSize: token("metrics.symbol_inline"),
                    // Centred on the first line of the message, not its top.
                    paddingTop: token("metrics.inner_radius"),
                  }}
                />
                <text
                  style={{
                    color: token("colors.danger"),
                    fontSize: token("typography.body"),
                  }}
                >
                  {target.first.error}
                </text>
              </div>
            ) : null}
            {target.first.studio ? null : (
              <AlignmentStrip key={target.key} session={session} />
            )}
          </>
        ) : null}
        {snapshot.sections.map((section) => (
          <DisclosureGroup
            key={section.id}
            id={`section:${section.id}`}
            label={section.title}
            open={!section.collapsed}
            style={
              section.focused
                ? { background: token("colors.selected_fill") }
                : undefined
            }
            onChange={(open) =>
              session.send({
                type: "collapse",
                section: section.id,
                collapsed: !open,
              })
            }
            accessory={
              section.actions ? (
                // Several actions (Time) stand in a row at the trailing edge.
                <div style={{ ...horizontal, gap: token("metrics.border") }}>
                  {section.actions.map((action) =>
                    sectionAction(section, action, true),
                  )}
                </div>
              ) : section.action ? (
                sectionAction(section, section.action, false)
              ) : undefined
            }
          >
            {section.finishes ? (
              <FinishShelf
                finishes={section.finishes}
                disabled={target.kind === "layers" && target.first.locked}
                session={session}
              />
            ) : null}
            {section.animators?.map((animator) => (
              <InspectorRow
                key={animator.id}
                label={animator.name}
                leading={gutter}
              >
                <Action
                  id={`remove-animator:${animator.id}`}
                  label={`Remove ${animator.name}`}
                  tooltip="Remove Text Animator"
                  symbol="minus"
                  disabled={target.kind === "layers" && target.first.locked}
                  onActivate={() =>
                    session.send({
                      type: "remove-animator",
                      animator: animator.id,
                    })
                  }
                />
              </InspectorRow>
            ))}
            {section.masks?.flatMap((mask) => [
              <InspectorRow key={mask.id} label={mask.name} leading={gutter}>
                <Action
                  id={`remove-mask:${mask.id}`}
                  label={`Remove ${mask.name}`}
                  tooltip="Remove Mask"
                  symbol="minus"
                  disabled={target.kind === "layers" && target.first.locked}
                  onActivate={() =>
                    session.send({ type: "remove-mask", mask: mask.id })
                  }
                />
              </InspectorRow>,
              ...mask.rows.map((row) => (
                <Property key={row.id} row={row} session={session} />
              )),
            ])}
            {section.rows
              .filter((row) => !row.hidden)
              .map((row) => (
                <Property key={row.id} row={row} session={session} />
              ))}
            {section.note ? (
              <text
                style={{
                  color: token("colors.muted"),
                  fontSize: token("typography.minimum"),
                  paddingLeft: token("metrics.inset"),
                  paddingRight: token("metrics.inset"),
                  paddingBottom: token("metrics.inner_radius"),
                }}
              >
                {section.note}
              </text>
            ) : null}
          </DisclosureGroup>
        ))}
        {target.kind === "composition" ? (
          <text
            style={{
              color: token("colors.muted"),
              fontSize: token("typography.minimum"),
              paddingLeft: token("metrics.inset"),
              paddingRight: token("metrics.inset"),
              paddingTop: token("metrics.inset"),
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
        data-pm-slot="header"
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
