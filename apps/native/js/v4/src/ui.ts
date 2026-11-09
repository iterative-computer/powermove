/** v4 ports of the tagged controls. Semantic trees only; no private visual language. */
import { h, type ComponentChildren } from "preact";
import { useContext, useId, useState } from "preact/hooks";
import type { SurfaceProps } from "./jsx-runtime";
import { Label } from "./label";
import type { Json, SurfaceEvent } from "./protocol";
// The v1-3 control math, frozen at the tagged revision: v4 fields parse and
// convert exactly as the controls users already know.
export * from "../../compat/src/frozen/reference/controls/color-space";
export { parseArithmetic } from "../../compat/src/frozen/reference/controls/arith";

export interface ControlProps extends SurfaceProps {
  value: Json;
  /** Opaque registered Rust binding handle; never a JS mutation callback. */
  binding: string;
  label?: string;
  mixed?: boolean;
}
function field(kind: string, primitive: string, props: ControlProps) {
  const labelledBy = useContext(Label);
  return h(primitive, {
    ...props,
    control: kind,
    labelledBy: props.labelledBy ?? labelledBy,
  });
}
// Native binding driver handles transactional focus/input/blur/Escape, numeric scrubs,
// transport invalidation, and live reads. No JS roundtrip is needed per pointer sample.
export function TextField(props: ControlProps & { value: string }) {
  return field("text", "input", props);
}
export interface NumFieldProps extends ControlProps {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  speed?: number;
  unit?: string;
  precision?: number;
}
export function NumField(props: NumFieldProps) {
  return field("number", "input", props);
}
export function SliderField(props: NumFieldProps & { angle?: boolean }) {
  return field(props.angle ? "angle" : "slider", "input", props);
}
export interface PointFieldProps extends ControlProps {
  value: [number, number];
  xBinding: string;
  yBinding: string;
  minX?: number;
  maxX?: number;
  minY?: number;
  maxY?: number;
  step?: number;
  stepX?: number;
  stepY?: number;
}
export function PointField(props: PointFieldProps) {
  const { xBinding, yBinding, ...rest } = props;
  return field("point", "div", {
    ...rest,
    attr: { ...props.attr, xBinding, yBinding },
  });
}
export function ColorField(props: ControlProps & { value: string }) {
  return field("color", "button", props);
}
export function FillField(props: ControlProps) {
  return field("fill", "button", props);
}
export interface FillStop {
  id: string;
  color: string;
  position: number;
}
export function RampField(
  props: ControlProps & {
    value: (FillStop & Record<string, Json>)[];
    selected?: string;
    midpointBinding?: string;
  },
) {
  return field("ramp", "div", props);
}
export function FontField(
  props: ControlProps & { value: string; options: string[] },
) {
  return field("font", "button", props);
}
export function ToggleField(props: ControlProps & { value: boolean }) {
  return field("toggle", "div", { ...props, role: "radiogroup" });
}
export interface SelectOption {
  v: Json;
  label: string;
  disabled?: boolean;
}
export function SelectField(
  props: ControlProps & { options: (string | SelectOption)[] },
) {
  return field("select", "button", props);
}

export function Row(
  props: SurfaceProps & { label: string; children?: ComponentChildren },
) {
  const id = useId();
  return h(
    "div",
    { role: "group", control: "row", ...props, children: undefined },
    h("text", { id }, props.label),
    h(Label.Provider, { value: id }, props.children),
  );
}
export function Section(
  props: SurfaceProps & { title: string; action?: ComponentChildren },
) {
  const { title, action, children, ...rest } = props;
  return h(
    "div",
    { ...rest, control: "section", role: "group", label: title },
    h("text", null, title),
    action,
    children,
  );
}
export function BackTitle(
  props: SurfaceProps & { title: string; onBack: () => void },
) {
  const { title, onBack, ...rest } = props;
  return h(
    "button",
    { ...rest, control: "back-title", label: title, onClick: onBack },
    title,
  );
}
export function Disclosure(
  props: SurfaceProps & {
    label: string;
    open?: boolean;
    onChange?: (open: boolean) => void;
  },
) {
  const [local, setLocal] = useState(props.open ?? false);
  const open = props.open ?? local;
  const id = useId();
  return h(
    "div",
    { control: "disclosure" },
    h(
      "button",
      {
        label: props.label,
        "aria-expanded": open,
        "aria-controls": id,
        onClick: () => {
          setLocal(!open);
          props.onChange?.(!open);
        },
      },
      props.label,
    ),
    open && h("div", { id }, props.children),
  );
}
export interface SegmentOption {
  id: string;
  label: string;
}
export function Segmented(props: {
  options: SegmentOption[];
  value: string;
  onChange: (id: string) => void;
  label?: string;
}) {
  const [focusRequest, setFocusRequest] = useState<string | undefined>();
  const group = useId();
  const choose = (index: number) => {
    if (!props.options.length) return;
    const next =
      props.options[(index + props.options.length) % props.options.length];
    props.onChange(next.id);
    setFocusRequest(`${group}:${next.id}`);
  };
  const keys = [
    "ArrowLeft",
    "ArrowUp",
    "ArrowRight",
    "ArrowDown",
    "Home",
    "End",
  ];
  return h(
    "div",
    { control: "segmented", role: "tablist", label: props.label, focusRequest },
    props.options.map((option, i) =>
      h(
        "button",
        {
          key: option.id,
          id: `${group}:${option.id}`,
          role: "tab",
          label: option.label,
          "aria-selected": option.id === props.value,
          tabIndex: option.id === props.value ? 0 : -1,
          preventDefault: { keydown: keys },
          onClick: () => choose(i),
          onKeyDown: (event: SurfaceEvent) => {
            const target: Record<string, number> = {
              ArrowLeft: i - 1,
              ArrowUp: i - 1,
              ArrowRight: i + 1,
              ArrowDown: i + 1,
              Home: 0,
              End: props.options.length - 1,
            };
            if (event.key && event.key in target) choose(target[event.key]);
          },
        },
        option.label,
      ),
    ),
  );
}
export interface MenuItem {
  id: string;
  label: string;
  disabled?: boolean;
  checked?: boolean;
  onSelect: () => void;
}
export function Menu(props: {
  items: MenuItem[];
  label?: string;
  onClose?: () => void;
}) {
  const [index, setIndex] = useState(() =>
    Math.max(
      0,
      props.items.findIndex((i) => !i.disabled),
    ),
  );
  const group = useId();
  function move(delta: number) {
    for (let n = 1; n <= props.items.length; n++) {
      const i = (index + delta * n + props.items.length) % props.items.length;
      if (!props.items[i].disabled) {
        setIndex(i);
        return;
      }
    }
  }
  return h(
    "div",
    {
      role: "menu",
      control: "menu",
      label: props.label,
      focusScope: true,
      focusRequest: `${group}:${props.items[index]?.id ?? ""}`,
      preventDefault: {
        keydown: [
          "ArrowUp",
          "ArrowDown",
          "Home",
          "End",
          "Enter",
          " ",
          "Escape",
        ],
      },
      onKeyDown: (event: SurfaceEvent) => {
        if (event.key === "ArrowUp") move(-1);
        else if (event.key === "ArrowDown") move(1);
        else if (event.key === "Home")
          setIndex(
            Math.max(
              0,
              props.items.findIndex((i) => !i.disabled),
            ),
          );
        else if (event.key === "End") {
          for (let i = props.items.length - 1; i >= 0; i--)
            if (!props.items[i].disabled) {
              setIndex(i);
              break;
            }
        } else if (event.key === "Escape") {
          event.stopPropagation();
          props.onClose?.();
        } else if (
          (event.key === "Enter" || event.key === " ") &&
          !props.items[index]?.disabled
        )
          props.items[index]?.onSelect();
      },
    },
    props.items.map((item, i) =>
      h(
        "button",
        {
          key: item.id,
          id: `${group}:${item.id}`,
          role: item.checked === undefined ? "menuitem" : "menuitemcheckbox",
          disabled: item.disabled,
          "aria-checked": item.checked,
          tabIndex: i === index ? 0 : -1,
          onClick: () => {
            if (!item.disabled) item.onSelect();
          },
        },
        item.label,
      ),
    ),
  );
}
export function FieldMenu(props: {
  anchor: number;
  open: boolean;
  items: MenuItem[];
  label?: string;
  onClose?: () => void;
}) {
  return props.open
    ? h(
        "anchored",
        { anchor: props.anchor, control: "field-menu" },
        h(Menu, props),
      )
    : null;
}
export function ToolbarItem(
  props: SurfaceProps & { label: string; active?: boolean },
) {
  const { active, ...rest } = props;
  return h("button", {
    ...rest,
    control: "toolbar-item",
    "aria-pressed": active,
  });
}
export function Glass(props: SurfaceProps & { surface: string }) {
  return h("Glass", props);
}
// DESIGN-TODO(opus): control renderers, picker surfaces, menus, and all composition
// layout/motion need approved v4 specs. Host must resolve semantic tokens through
// pm-design; these neutral trees deliberately contain no new visual defaults.
