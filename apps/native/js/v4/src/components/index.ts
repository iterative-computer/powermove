/** Native UI kit, language.md §§3,5–9,13,16. Host resolves tokens via pm-design. */
import { h, type ComponentChildren } from "preact";
import { useContext, useId, useState } from "preact/hooks";
import type { SurfaceProps } from "../jsx-runtime";
import { Label } from "../label";
import { json, type Json, type SurfaceEvent } from "../protocol";

export const token = (name: string) => ({ token: name });
type Style = NonNullable<SurfaceProps["style"]>;
// Component callbacks/options override primitive event/JSON props; intersecting
// them would require a callback accepting both an event and a component value.
type Common = SurfaceProps & { label: string };
type Bound<T extends Json> = SurfaceProps & {
  label?: string;
  value: T;
  binding: string;
  mixed?: boolean;
};
const text: Style = {
  color: token("colors.text"),
  fontSize: token("typography.body"),
};
const target: Style = {
  minWidth: token("metrics.hit"),
  minHeight: token("metrics.hit"),
};
const well: Style = {
  ...text,
  ...target,
  height: token("metrics.field"),
  background: token("colors.field"),
  borderRadius: token("metrics.control_radius"),
};
function kit(
  kind: string,
  primitive: string,
  props: SurfaceProps,
  style: Style = {},
) {
  return h(primitive, {
    ...props,
    "data-pm-component": kind,
    style: { ...style, ...props.style },
  });
}
export function InspectorRow(
  props: Common & { leading?: ComponentChildren; trailing?: ComponentChildren },
) {
  const id = useId();
  const { label, children, leading, trailing, ...rest } = props;
  return kit(
    "inspector-row",
    "div",
    {
      ...rest,
      role: "group",
      label,
      children: [
        leading ? h("div", { "data-pm-slot": "leading" }, leading) : null,
        h(
          "text",
          {
            id,
            label,
            title: label,
            "data-pm-slot": "label",
            style: {
              color: token("colors.secondary"),
              fontSize: token("typography.label"),
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            },
          },
          label,
        ),
        // The label keeps `metrics.label_column` and the value takes the
        // rest of the row, never less than `metrics.value_column` (the host
        // sizes both slots).
        h(
          "div",
          { "data-pm-slot": "value" },
          h(Label.Provider, { value: id }, children),
        ),
        trailing ? h("div", { "data-pm-slot": "trailing" }, trailing) : null,
      ],
    },
    {
      display: "flex",
      flexDirection: "row",
      alignItems: "center",
      height: token("metrics.row"),
      paddingLeft: token("metrics.inset"),
      paddingRight: token("metrics.inset"),
    },
  );
}
function field(
  kind: string,
  control: string,
  primitive: string,
  props: SurfaceProps,
  style: Style = well,
) {
  const labelledBy = useContext(Label);
  return kit(
    kind,
    primitive,
    { ...props, control, labelledBy: props.labelledBy ?? labelledBy },
    style,
  );
}
export interface NumberFieldProps extends Bound<number> {
  label?: string;
  min?: number;
  max?: number;
  step?: number;
  speed?: number;
  precision?: number;
  unit?: string;
}
export function NumberField(props: NumberFieldProps) {
  return field("number-field", "number", "input", {
    ...props,
    style: {
      fontVariantNumeric: "tabular-nums",
      textAlign: "left",
      unitColor: token("colors.muted"),
      ...props.style,
    },
  });
}
/** `multiline`: a native multi-line editor (Return makes a new line). */
export function TextField(props: Bound<string> & { multiline?: boolean }) {
  const { multiline, ...rest } = props;
  return field("text-field", "text", multiline ? "textarea" : "input", rest);
}
export interface Option {
  v: Json;
  label: string;
  disabled?: boolean;
  separator?: boolean;
  keyEquivalent?: string;
  detail?: string;
  /** A checkmark beside the row (a pull-down showing several settings). */
  checked?: boolean;
}
export function PopUpButton(
  props: Omit<Bound<Json>, "options"> & {
    label?: string;
    options: (string | Option | { separator: true })[];
    widthSample?: string;
    /** The button's title when it differs from the selected row. */
    display?: string;
  },
) {
  // Inside an InspectorRow a pop-up is a `field`-high value well; standalone
  // it is a `control`-high button (language.md §9).
  const inRow = useContext(Label) !== undefined;
  const { widthSample, display, options, ...rest } = props;
  return field("pop-up-button", "select", "button", {
    ...rest,
    // Keep the frozen primitive option shape. Native NSMenu presentation lives
    // in additive metadata; values including null/false remain exact.
    options: json(
      options.flatMap<Json>((o) => {
        if (typeof o === "string") return [o];
        if (o.separator || !("v" in o)) return [];
        return [
          {
            v: o.v,
            label: o.label,
            ...(o.disabled === undefined ? {} : { disabled: o.disabled }),
          },
        ];
      }),
    ) as Json[],
    metadata: kitMetadata(props, {
      menu: json(options),
      ...(widthSample === undefined ? {} : { widthSample }),
      ...(display === undefined ? {} : { display }),
    }),
    style: {
      height: token(inRow ? "metrics.field" : "metrics.control"),
      ...props.style,
    },
  });
}
export function Checkbox(props: Bound<boolean>) {
  return field(
    "checkbox",
    "toggle",
    "button",
    {
      ...props,
      role: "checkbox",
      "aria-checked": props.mixed ? "mixed" : props.value,
    },
    { ...text, ...target },
  );
}
export function Toggle(props: Bound<boolean>) {
  return field(
    "toggle",
    "toggle",
    "button",
    {
      ...props,
      // Panels use checkboxes; switches are for Settings only (design review).
      role: "checkbox",
      "aria-checked": props.mixed ? "mixed" : props.value,
    },
    { ...text, ...target },
  );
}
export function ColourWell(props: Bound<string> & { minimal?: boolean }) {
  const { minimal, ...rest } = props;
  return field(
    "colour-well",
    "color",
    "button",
    {
      ...rest,
      metadata: kitMetadata(props, { minimal: !!minimal }),
      "aria-haspopup": "dialog",
    },
    {
      ...text,
      ...target,
      height: token("metrics.field"),
      borderRadius: token("metrics.control_radius"),
    },
  );
}
export interface SegmentOption {
  id: string;
  label: string;
  disabled?: boolean;
  /** SF Symbol shown instead of the label; the label becomes the tooltip. */
  symbol?: string;
  /** Tooltip line 1 when it differs from the label (a shortcut, say). */
  title?: string;
  /** Tooltip line 2: what it does, one sentence. */
  help?: string;
}
export function SegmentedControl(
  props: Omit<Common, "options" | "onChange"> & {
    options: SegmentOption[];
    value: string;
    onChange: (id: string) => void;
  },
) {
  const { options, value, onChange, ...rest } = props;
  const group = useId();
  const [focusRequest, setFocusRequest] = useState<string>();
  const enabled = options.filter((o) => !o.disabled);
  const active = enabled.find((o) => o.id === value) ?? enabled[0];
  const choose = (id: string) => {
    if (props.disabled || !enabled.some((o) => o.id === id)) return;
    onChange(id);
    setFocusRequest(`${group}:${id}`);
  };
  const keys = [
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Home",
    "End",
  ];
  return kit(
    "segmented-control",
    "div",
    {
      ...rest,
      role: "radiogroup",
      focusRequest,
      children: options.map((option) =>
        kit(
          "segment",
          "button",
          {
            key: option.id,
            id: `${group}:${option.id}`,
            label: option.label,
            ...(option.symbol ? { attr: { symbol: option.symbol } } : {}),
            ...(option.symbol || option.help || option.title
              ? {
                  title: [option.title ?? option.label, option.help]
                    .filter(Boolean)
                    .join("\n"),
                }
              : {}),
            role: "radio",
            disabled: props.disabled || option.disabled,
            "aria-checked": value === option.id,
            tabIndex: option.id === active?.id ? 0 : -1,
            preventDefault: { keydown: keys },
            onClick: () => choose(option.id),
            onKeyDown: (event: SurfaceEvent) => {
              const index = enabled.findIndex((o) => o.id === option.id);
              if (!enabled.length || index < 0) return;
              let next: number;
              switch (event.key) {
                case "ArrowLeft":
                case "ArrowUp":
                  next = (index + enabled.length - 1) % enabled.length;
                  break;
                case "ArrowRight":
                case "ArrowDown":
                  next = (index + 1) % enabled.length;
                  break;
                case "Home":
                  next = 0;
                  break;
                case "End":
                  next = enabled.length - 1;
                  break;
                default:
                  return;
              }
              choose(enabled[next].id);
            },
            children: option.symbol ? undefined : option.label,
          },
          // quiet-md toggle cell: the selected segment is a neutral wash with
          // full ink, the rest caption grey; hover is the renderer's spring wash.
          {
            ...target,
            ...text,
            display: "flex",
            flexGrow: 1,
            alignItems: "center",
            justifyContent: "center",
            paddingLeft: token("metrics.inset"),
            paddingRight: token("metrics.inset"),
            borderRadius: token("metrics.segment_radius"),
            ...(value === option.id
              ? { background: token("colors.fill_strong") }
              : { color: token("colors.secondary") }),
          },
        ),
      ),
    },
    // One raised pill with a hairline edge; segments sit concentric inside it.
    {
      display: "flex",
      flexDirection: "row",
      height: token("metrics.tab"),
      padding: token("metrics.pill_inset"),
      gap: token("metrics.pill_inset"),
      background: token("colors.floating"),
      borderWidth: token("metrics.border"),
      borderColor: token("colors.hairline"),
      borderRadius: token("metrics.pill_radius"),
    },
  );
}
export function DisclosureGroup(
  props: Omit<Common, "onChange"> & {
    open: boolean;
    onChange: (open: boolean) => void;
    /** The section's own action, at the trailing edge of its heading row
     * (Refresh Fonts, Add Mask). */
    accessory?: ComponentChildren;
  },
) {
  const { children, open, onChange, accessory, ...rest } = props;
  const id = useId();
  // A section heading is a label, a step under its panel's title, on a row
  // of its own.
  const button = kit(
    "disclosure-button",
    "button",
    {
      label: props.label,
      disabled: props.disabled,
      "aria-expanded": open,
      "aria-controls": id,
      onClick: () => {
        if (!props.disabled) onChange(!open);
      },
      children: props.label,
    },
    {
      ...target,
      color: token("colors.text"),
      fontSize: token("typography.label"),
      fontWeight: token("typography.semibold_weight"),
      height: token("metrics.row"),
      minHeight: token("metrics.row"),
      ...(accessory ? { flexGrow: 1 } : {}),
    },
  );
  return kit("disclosure-group", "div", {
    ...rest,
    role: "group",
    children: [
      accessory
        ? h("div", { "data-pm-slot": "heading" }, [
            button,
            h("div", { "data-pm-slot": "accessory" }, accessory),
          ])
        : button,
      open ? h("div", { id }, children) : null,
    ],
  });
}
export function List(props: Common) {
  return kit("list", "div", { ...props, role: "list" });
}
export function Tree(props: Common) {
  return kit("tree", "div", { ...props, role: "tree" });
}
type Item = Omit<Common, "selected" | "onSelect"> & {
  selected?: boolean;
  onSelect: () => void;
};
function item(
  kind: string,
  role: string,
  props: Item & { expanded?: boolean; onExpand?: (open: boolean) => void },
) {
  const { selected, onSelect, expanded, onExpand, children, ...rest } = props;
  return kit(
    kind,
    "div",
    {
      ...rest,
      role,
      tabIndex: props.tabIndex ?? 0,
      "aria-selected": !!selected,
      "aria-expanded": expanded,
      onClick: () => {
        if (!props.disabled) onSelect();
      },
      preventDefault: {
        keydown: [
          "Enter",
          " ",
          ...(onExpand ? ["ArrowLeft", "ArrowRight"] : []),
        ],
      },
      onKeyDown: (event: SurfaceEvent) => {
        if (props.disabled) return;
        if (event.key === "Enter" || event.key === " ") onSelect();
        else if (event.key === "ArrowLeft" && onExpand) onExpand(false);
        else if (event.key === "ArrowRight" && onExpand) onExpand(true);
        else props.onKeyDown?.(event);
      },
      children: children ?? props.label,
    },
    {
      ...text,
      ...target,
      height: token("metrics.row"),
      ...(selected ? { background: token("colors.selected_fill") } : {}),
    },
  );
}
export function ListItem(props: Item) {
  return item("list-item", "listitem", props);
}
export function TreeItem(
  props: Item & { expanded?: boolean; onExpand?: (open: boolean) => void },
) {
  return item("tree-item", "treeitem", props);
}
export function ToolbarItem(props: Common & { active?: boolean }) {
  const { active, ...rest } = props;
  return kit(
    "toolbar-item",
    "button",
    { ...rest, "aria-pressed": active },
    {
      ...target,
      // Active is the neutral selected state: primary ink on the wash a
      // selected row has, never the accent. An item that states its own
      // colour (a keyframe mark) keeps its own look.
      color: token(active ? "colors.text" : "colors.secondary"),
      ...(active && !props.style?.color
        ? { background: token("colors.selected_fill") }
        : {}),
      symbolSize: token("metrics.symbol_toolbar"),
    },
  );
}
export function SheetAnchor(props: Common & { anchor: number; open: boolean }) {
  const { open, ...rest } = props;
  return open
    ? kit(
        "sheet-anchor",
        "anchored",
        {
          ...rest,
          role: "dialog",
          "aria-modal": true,
          focusScope: true,
          "data-pm-system-placement": "system",
        },
        {
          width: token("metrics.sheet_width"),
          background: token("colors.panel"),
        },
      )
    : null;
}
export function PopoverAnchor(
  props: Common & { anchor: number; open: boolean; onClose: () => void },
) {
  const { open, onClose, ...rest } = props;
  return open
    ? kit(
        "popover-anchor",
        "anchored",
        {
          ...rest,
          role: "dialog",
          focusScope: true,
          "data-pm-dismiss-outside": true,
          "data-pm-live-edit": true,
          preventDefault: { keydown: ["Escape"] },
          onKeyDown: (event: SurfaceEvent) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              onClose();
            } else props.onKeyDown?.(event);
          },
        },
        {
          width: token("metrics.popover_width"),
          background: token("colors.floating"),
        },
      )
    : null;
}
export function ProgressRow(props: Common & { value?: number }) {
  if (
    props.value !== undefined &&
    (!Number.isFinite(props.value) || props.value < 0 || props.value > 1)
  )
    throw new RangeError("Progress must be a finite fraction or omitted");
  return kit(
    "progress-row",
    "div",
    {
      ...props,
      role: "progressbar",
      "aria-valuenow": props.value,
      "data-pm-indeterminate": props.value === undefined,
      children: props.children ?? props.label,
    },
    {
      ...text,
      height: token("metrics.row"),
      fontVariantNumeric: "tabular-nums",
    },
  );
}
export function EmptyState(
  props: Common & {
    explanation: string;
    action?: ComponentChildren;
    symbol?: string;
  },
) {
  const { explanation, action, symbol, ...rest } = props;
  return kit(
    "empty-state",
    "div",
    {
      ...rest,
      role: "group",
      children: [
        symbol
          ? h("div", {
              "data-pm-slot": "symbol",
              metadata: { pmKit: { symbol, decorative: true } },
            })
          : null,
        h(
          "text",
          {
            role: "heading",
            style: {
              color: token("colors.text"),
              fontSize: token("typography.heading"),
              fontWeight: token("typography.semibold_weight"),
            },
          },
          props.label,
        ),
        h(
          "text",
          {
            style: {
              color: token("colors.secondary"),
              fontSize: token("typography.reading"),
            },
          },
          explanation,
        ),
        action,
      ],
    },
    {
      display: "flex",
      flexDirection: "column",
      alignItems: "center",
      justifyContent: "center",
    },
  );
}
// Names used by the existing v4 ports. The compiler/legacy ui.ts export switch is
// outside this lane: consumers currently import this components entry explicitly.
export {
  InspectorRow as Row,
  NumberField as NumField,
  PopUpButton as SelectField,
  Toggle as ToggleField,
  ColourWell as ColorField,
  SegmentedControl as Segmented,
  DisclosureGroup as Disclosure,
};
// Resolved in docs/native/design/reviews/na-cloud-uikit.md: the colour picker is
// specified in panels/inspector.md §7 (host-provided popover); Toggle and Checkbox
// sit at the leading edge of the value column; tree indentation follows the
// system outline default until pm-design adds metrics.tree_indent (follow-up F2).

/** Additive metadata uses the frozen six-op contract; primitive schemas stay unchanged. */
function kitMetadata(
  props: { metadata?: Record<string, Json> },
  value: Record<string, Json>,
): Record<string, Json> {
  const previous = props.metadata?.pmKit;
  return {
    ...props.metadata,
    pmKit: {
      ...(previous && typeof previous === "object" && !Array.isArray(previous)
        ? previous
        : {}),
      ...value,
    },
  };
}
/** The shell verifies this declaration against its actual Settings window. */
export function SettingsRoot(props: Common) {
  return h("div", {
    ...props,
    role: "group",
    metadata: { ...props.metadata, hostWindow: "settings" },
  });
}
export interface SliderProps extends Bound<number> {
  min: number;
  max: number;
  step?: number;
  /** A logarithmic track (brightness, size): equal moves multiply the value. */
  log?: boolean;
  unit?: string;
}
/** A native NSSlider for a bounded value. It changes once per pull (one
 * Undo) and steps with the arrow keys; VoiceOver reads the value itself. */
export function Slider(props: SliderProps) {
  const { log, ...rest } = props;
  return field(
    "slider",
    "slider",
    "input",
    {
      ...rest,
      metadata: kitMetadata(props, { log: !!log }),
    },
    { ...target, width: token("metrics.value_column") },
  );
}
export interface ColorWheelProps extends SurfaceProps {
  label: string;
  /** A grouped binding over the wheel's red, green and blue rows. */
  binding: string;
  /** Red, green and blue as the parameters hold them. */
  value: [number, number, number];
  /** Where the channels are neutral: 0 (lift, gamma, offset) or 1 (gain). */
  neutral: number;
  min: number;
  max: number;
  /** The wheel's master and its limits: shown on the wheel's bezel, so the
   * wheel and its master read as one instrument (set in its own field). */
  master?: { value: number; min: number; max: number };
}
/** A colour wheel (drawn by the host): dragging the puck pushes red, green
 * and blue together, keeping their brightness, in one Undo step; Option
 * drags finely; a double-click returns the wheel to neutral. */
export function ColorWheel(props: ColorWheelProps) {
  const { neutral, min, max, master, ...rest } = props;
  return kit(
    "color-wheel",
    "div",
    {
      ...rest,
      control: "color-wheel",
      metadata: kitMetadata(props, {
        neutral,
        min,
        max,
        ...(master
          ? {
              master: master.value,
              masterMin: master.min,
              masterMax: master.max,
            }
          : {}),
      }),
    },
    // Across its group; the host makes it as tall as it is wide, between
    // the least and greatest wheel (past the greatest, centred).
    {
      minWidth: token("metrics.grade_wheel"),
      minHeight: token("metrics.grade_wheel"),
    },
  );
}
export interface CurveEditorProps extends SurfaceProps {
  label: string;
  /** A grouped binding over the curve's point count and every point. */
  binding: string;
  /** `[count, x0, y0, x1, y1, …]` as the parameters hold them. */
  value: number[];
  /** 0 Master, 1 Red, 2 Green, 3 Blue: the curve's ink. */
  channel: number;
}
/** A curve editor (drawn by the host): a press adds a point, a drag moves
 * one, dragging one well off the graph removes it, a double-click returns
 * the curve to a straight line; each gesture is one Undo step. */
export function CurveEditor(props: CurveEditorProps) {
  const { channel, ...rest } = props;
  return kit(
    "curve-editor",
    "div",
    {
      ...rest,
      control: "curve-editor",
      metadata: kitMetadata(props, { channel }),
    },
    // Across its group; the host makes it as tall as it is wide, between
    // the least and greatest graph.
    {
      minHeight: token("metrics.curve_editor"),
    },
  );
}
export function Switch(props: Bound<boolean>) {
  return field(
    "switch",
    "toggle",
    "button",
    {
      ...props,
      role: "switch",
      "aria-checked": props.value,
    },
    { ...text, ...target },
  );
}
type SecureProps = Omit<
  Common,
  "value" | "control" | "onInput" | "onChange" | "children"
> & { binding: string };
export function SecureField(props: SecureProps) {
  for (const key of ["value", "control", "onInput", "onChange", "children"])
    if (key in props)
      throw new TypeError(
        "SecureField accepts only a native binding, never secret text or input callbacks",
      );
  return kit(
    "secure-field",
    "div",
    { ...props, tabIndex: props.tabIndex ?? 0 },
    well,
  );
}
export function FormGroup(
  props: Omit<Common, "label" | "title"> & {
    title?: string;
    label?: string;
    muted?: boolean;
  },
) {
  const { title, children, muted, ...rest } = props;
  return h("div", { role: "group", label: props.label ?? title }, [
    title
      ? h(
          "text",
          {
            role: "heading",
            // A quiet label over its card, on the rows' text column.
            style: {
              color: token(muted ? "colors.muted" : "colors.secondary"),
              fontSize: token("typography.label"),
              fontWeight: token("typography.medium_weight"),
              paddingLeft: token("metrics.inset"),
              marginBottom: token("metrics.gap"),
            },
          },
          title,
        )
      : null,
    kit("form-group", "div", {
      ...rest,
      role: "group",
      label: props.label ?? title,
      children,
    }),
  ]);
}
export function FormRow(
  props: Omit<Common, "control"> & {
    detail?: string;
    control: ComponentChildren;
  },
) {
  const { label, detail, control, ...rest } = props;
  return kit(
    "form-row",
    "div",
    {
      ...rest,
      role: "group",
      label,
      children: [
        // Title over its description on the left; the control trails,
        // centred on the pair (Settings-sheet anatomy).
        h(
          "div",
          {
            "data-pm-slot": "label",
            style: {
              display: "flex",
              flexDirection: "column",
              flexGrow: 1,
              gap: token("metrics.border"),
            },
          },
          [
            h(
              "text",
              { style: { ...text, fontSize: token("typography.title") } },
              label,
            ),
            detail
              ? h(
                  "text",
                  {
                    style: {
                      color: token("colors.muted"),
                      fontSize: token("typography.label"),
                    },
                  },
                  detail,
                )
              : null,
          ],
        ),
        h(
          "div",
          {
            "data-pm-slot": "control",
            style: { display: "flex", flexShrink: 0, alignItems: "center" },
          },
          control,
        ),
      ],
    },
    {
      display: "flex",
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      gap: token("metrics.inset"),
      minHeight: token("metrics.form_row"),
      paddingLeft: token("metrics.inset"),
      paddingRight: token("metrics.inset"),
      paddingTop: token("metrics.gap"),
      paddingBottom: token("metrics.gap"),
    },
  );
}
export function PushButton(
  props: Omit<Common, "role"> & { role?: "normal" | "default" | "destructive" },
) {
  const { role, ...rest } = props;
  return kit(
    "push-button",
    "button",
    {
      ...rest,
      metadata: kitMetadata(props, { buttonRole: role ?? "normal" }),
      children: props.children ?? props.label,
    },
    { ...text, ...target, height: token("metrics.control") },
  );
}
export function ColourPopover(
  props: Common & {
    anchor: number;
    binding: string;
    open: boolean;
    fill?: boolean;
    onClose: (cancel: boolean) => void;
  },
) {
  const { open, fill, onClose, ...rest } = props;
  if (!open) return null;
  return kit(
    "colour-popover",
    "anchored",
    {
      ...rest,
      role: "dialog",
      focusScope: true,
      "data-pm-dismiss-outside": true,
      "data-pm-live-edit": true,
      metadata: kitMetadata(props, { fill: !!fill, nativeColourPicker: true }),
      preventDefault: { keydown: ["Escape"] },
      onKeyDown: (event: SurfaceEvent) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose(true);
        } else props.onKeyDown?.(event);
      },
    },
    {
      width: token("metrics.popover_width"),
      background: token("colors.floating"),
    },
  );
}
export type GizmoKind =
  | "transform"
  | "rotation"
  | "anchor"
  | "path-vertex"
  | "tangent";
export function ViewerGizmo(
  props: Omit<Common, "selected"> & {
    kind: GizmoKind;
    selected?: boolean;
    position: { x: number; y: number };
  },
) {
  const { kind, selected, position, ...rest } = props;
  if (!Number.isFinite(position.x) || !Number.isFinite(position.y))
    throw new RangeError("Gizmo position must be finite");
  return kit(
    "viewer-gizmo",
    "button",
    {
      ...rest,
      metadata: kitMetadata(props, { kind, selected: !!selected, position }),
    },
    target,
  );
}
export function ViewerOverlay(props: Common) {
  return kit("viewer-overlay", "div", { ...props, role: "group" });
}
export function Banner(
  props: Common & {
    severity?: "info" | "warning" | "error";
    placement?: "viewer" | "inline";
    symbol: string;
    action?: ComponentChildren;
    onDismiss?: () => void;
  },
) {
  const {
    severity = "info",
    placement = "viewer",
    symbol,
    action,
    onDismiss,
    ...rest
  } = props;
  return kit("banner", "div", {
    ...rest,
    role: "status",
    metadata: kitMetadata(props, {
      severity,
      placement,
      symbol,
      hasAction: !!action,
    }),
    children: [
      h("div", {
        "data-pm-slot": "symbol",
        metadata: { pmKit: { symbol, decorative: true } },
      }),
      h("text", { style: text }, props.label),
      action,
      action && onDismiss
        ? h("button", {
            label: "Dismiss",
            title: "Dismiss",
            onClick: onDismiss,
            metadata: { pmKit: { symbol: "xmark" } },
            style: {
              ...target,
              color: token("colors.secondary"),
              symbolSize: token("metrics.symbol_inline"),
            },
          })
        : null,
    ],
  });
}
export { ColourWell as ColorWell, ColourPopover as ColorPopover };
