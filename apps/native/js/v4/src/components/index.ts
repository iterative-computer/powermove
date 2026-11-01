/** Native UI kit, language.md §§3,5–9,13,16. Host resolves tokens via pm-design. */
import { createContext, h, type ComponentChildren } from "preact";
import { useContext, useId, useState } from "preact/hooks";
import type { SurfaceProps } from "../jsx-runtime";
import type { Json, SurfaceEvent } from "../protocol";

export const token = (name: string) => ({ token: name });
type Style = NonNullable<SurfaceProps["style"]>;
// Component callbacks/options override primitive event/JSON props; intersecting
// them would require a callback accepting both an event and a component value.
type Common = Omit<
  SurfaceProps,
  "options" | "selected" | "onChange" | "onSelect"
> & {
  label: string;
};
type Bound<T extends Json> = SurfaceProps & {
  label?: string;
  value: T;
  binding: string;
  mixed?: boolean;
};
const Label = createContext<string | undefined>(undefined);
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
        h(
          "div",
          {
            "data-pm-slot": "value",
            style: {
              width: token("metrics.value_column"),
              minWidth: token("metrics.value_column"),
              maxWidth: token("metrics.value_column"),
            },
          },
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
export function TextField(props: Bound<string>) {
  return field("text-field", "text", "input", props);
}
export interface Option {
  v: Json;
  label: string;
  disabled?: boolean;
}
export function PopUpButton(
  props: Bound<Json> & {
    label?: string;
    options: (string | Option)[];
  },
) {
  // Inside an InspectorRow a pop-up is a `field`-high value well; standalone
  // it is a `control`-high button (language.md §9).
  const inRow = useContext(Label) !== undefined;
  return field("pop-up-button", "select", "button", {
    ...props,
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
export function ColourWell(props: Bound<string>) {
  return field(
    "colour-well",
    "color",
    "button",
    {
      ...props,
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
}
export function SegmentedControl(
  props: Common & {
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
            children: option.label,
          },
          {
            ...target,
            ...text,
            borderRadius: token("metrics.inner_radius"),
            ...(value === option.id
              ? { background: token("colors.floating") }
              : {}),
          },
        ),
      ),
    },
    {
      display: "flex",
      flexDirection: "row",
      background: token("colors.field"),
      borderRadius: token("metrics.control_radius"),
    },
  );
}
export function DisclosureGroup(
  props: Common & { open: boolean; onChange: (open: boolean) => void },
) {
  const { children, open, onChange, ...rest } = props;
  const id = useId();
  return kit("disclosure-group", "div", {
    ...rest,
    role: "group",
    children: [
      kit(
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
          fontSize: token("typography.title"),
          fontWeight: token("typography.semibold_weight"),
          paddingTop: token("metrics.inset"),
          paddingBottom: token("metrics.gap"),
        },
      ),
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
type Item = Common & { selected?: boolean; onSelect: () => void };
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
      color: token(active ? "colors.accent" : "colors.secondary"),
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
  props: Common & { explanation: string; action: ComponentChildren },
) {
  const { explanation, action, ...rest } = props;
  return kit(
    "empty-state",
    "div",
    {
      ...rest,
      role: "group",
      children: [
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
