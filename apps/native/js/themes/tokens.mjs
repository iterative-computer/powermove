import { ThemeError, closing, splitTop } from "./css.mjs";

const groups = {
  color:
    "accent accent-dim accent-hover accent-tx bg-field bg-float bg-panel bg-panel-2 bg-rail bg-row bg-row-hi bg-sunken bg-window blue blue-deep cream danger gray green ink-1 ink-2 ink-3 line line-2 line-strong on-accent red sand section-line success tx tx-2 tx-3 tx-4 violet warning yellow",
  channels: "ink-rgb paper-rgb",
  length:
    "ctl-h gap gut hairline hdr-h hit pad panel-edge-inset r-base r-lg r-md r-pill r-sm r-xl r-xs row-h val-w fs-lg fs-md fs-sm fs-xl fs-xs",
  font: "f-mono f-ui",
  number: "fw-medium fw-regular fw-semibold lh lh-tight disabled",
  shadow:
    "ctl-edge ctl-edge-inset focus-ring shadow-float shadow-lift shadow-panel shadow-raise",
  paint: "ctl-raised",
  duration: "dur-1 dur-2 dur-3",
  easing: "ease ease-io",
};

// Only semantically equivalent fields in pm-design are mapped (see
// docs/native/design/tokens.md, "Theme mapping"). Missing fields stay typed in
// v4; no approximation of focus stacks, fonts or easing. A token may feed more
// than one native role. Unmapped on purpose:
// - r-pill: "fully round" is a shape, not a radius; capsules are height / 2.
// - hairline: CSS device-pixel workaround; native separator width is owned by
//   pm-design and Increase Contrast.
// - fs-xl: v1.2.0's 18 px heading has no native role; display (22) is native-only.
const native = {
  accent: "colors.accent",
  "accent-dim": "colors.accent_wash",
  "accent-tx": "colors.accent_text",
  "bg-field": "colors.field",
  "bg-float": "colors.floating",
  "bg-panel": "colors.panel",
  "bg-panel-2": "colors.panel_secondary",
  "bg-sunken": "colors.viewer",
  "bg-row-hi": "colors.hover",
  "bg-window": "colors.window",
  danger: "colors.danger",
  line: "colors.separator",
  "on-accent": "colors.selected_text",
  success: "colors.success",
  tx: "colors.text",
  "tx-2": "colors.secondary",
  "tx-3": "colors.muted",
  warning: "colors.warning",
  "ctl-h": "metrics.control",
  gap: "metrics.gap",
  "hdr-h": "metrics.header",
  pad: "metrics.inset",
  "r-lg": "metrics.panel_radius",
  "r-sm": "metrics.control_radius",
  "r-xs": "metrics.inner_radius",
  "row-h": "metrics.row",
  "val-w": "metrics.value_column",
  "fs-xs": "typography.minimum",
  "fs-sm": ["typography.body", "typography.label"],
  "fs-md": ["typography.title", "typography.reading"],
  "fs-lg": "typography.heading",
};
// Translucent colours are composited over this backdrop before projection,
// because pm-design colours are opaque.
const backdrop = {
  "accent-dim": "--bg-panel",
  line: "--bg-panel",
  "tx-2": "--bg-panel",
  "tx-3": "--bg-panel",
};
const targets = (name) => [native[name] ?? []].flat();

export const FOUNDATION_TOKENS = Object.freeze(
  Object.fromEntries(
    Object.entries(groups).flatMap(([kind, names]) =>
      names.split(" ").map((name) => [
        `--${name}`,
        Object.freeze({
          kind,
          native: targets(name).length ? Object.freeze(targets(name)) : null,
          v4: targets(name)[0] ?? `legacy.${kind}.${name}`,
        }),
      ]),
    ),
  ),
);

function vars(text) {
  const found = [];
  let quote = "";
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\\") {
      i++;
      continue;
    }
    if (quote) {
      if (text[i] === quote) quote = "";
      continue;
    }
    if (text[i] === '"' || text[i] === "'") {
      quote = text[i];
      continue;
    }
    if (
      text.slice(i, i + 4).toLowerCase() === "var(" &&
      (i === 0 || !/[\w-]/.test(text[i - 1]))
    ) {
      const end = closing(text, i + 3),
        parts = splitTop(text.slice(i + 4, end));
      const name = parts.shift().trim();
      if (!/^--[^\s]+$/.test(name))
        throw new ThemeError(
          "invalid_variable",
          "Invalid custom property reference",
          { name },
        );
      const fallback = parts.length ? parts.join(",") : null;
      found.push({ start: i, end: end + 1, name, fallback });
      i = end;
    }
  }
  return found;
}

export function substitute(text, lookup) {
  let result = "",
    offset = 0;
  for (const variable of vars(text)) {
    result += text.slice(offset, variable.start);
    let value = lookup(variable.name);
    if (value == null && variable.fallback !== null)
      value = substitute(variable.fallback, lookup);
    if (value == null) return null;
    result += value;
    offset = variable.end;
  }
  return result + text.slice(offset);
}

export function resolveVariables(raw, inherited = {}) {
  const resolved = { ...inherited },
    dependencies = {},
    cycles = new Set(),
    visiting = [],
    visited = new Set();
  const references = (text) =>
    vars(text).flatMap((variable) => [
      variable.name,
      ...(variable.fallback === null ? [] : references(variable.fallback)),
    ]);
  for (const [name, text] of Object.entries(raw))
    dependencies[name] = [...new Set(references(text))];
  function check(name) {
    if (visiting.includes(name)) {
      for (const member of visiting.slice(visiting.indexOf(name)))
        cycles.add(member);
      return;
    }
    if (visited.has(name) || !Object.hasOwn(raw, name)) return;
    visiting.push(name);
    for (const dependency of dependencies[name]) check(dependency);
    visiting.pop();
    visited.add(name);
  }
  for (const name of Object.keys(raw)) check(name);
  const computed = new Set();
  function resolve(name) {
    if (!Object.hasOwn(raw, name))
      return Object.hasOwn(inherited, name) ? inherited[name] : null;
    if (computed.has(name)) return resolved[name];
    computed.add(name);
    if (cycles.has(name) || /^(initial)$/i.test(raw[name].trim()))
      return (resolved[name] = null);
    if (/^(inherit|unset)$/i.test(raw[name].trim()))
      return (resolved[name] = inherited[name] ?? null);
    return (resolved[name] = substitute(raw[name], resolve));
  }
  for (const name of Object.keys(raw)) resolve(name);
  return { values: resolved, dependencies, cycles: [...cycles] };
}

// Small exact arithmetic evaluator for the metric aliases in T1. Rich CSS
// expressions remain expression tokens and a declared host requirement.
export function scalar(text) {
  const tokens =
    text
      .trim()
      .match(/(?:\d*\.\d+|\d+\.?\d*)(?:px|ms|s|rem|%)?|[()+*/-]|[a-z-]+/g) ??
    [];
  if (tokens.join("") !== text.replace(/\s+/g, "")) return null;
  let index = 0;
  function primary() {
    let sign = 1;
    if (tokens[index] === "-" || tokens[index] === "+")
      sign = tokens[index++] === "-" ? -1 : 1;
    let result;
    if (tokens[index] === "calc") {
      index++;
      result = primary();
    } else if (tokens[index] === "(") {
      index++;
      result = sum();
      if (tokens[index++] !== ")") return null;
    } else {
      const match = /^(\d*\.\d+|\d+\.?\d*)(px|ms|s|rem|%)?$/.exec(
        tokens[index++] ?? "",
      );
      if (!match) return null;
      result = { value: Number(match[1]), unit: match[2] ?? "" };
    }
    return result && { ...result, value: result.value * sign };
  }
  function product() {
    let left = primary();
    while (left && ["*", "/"].includes(tokens[index])) {
      const operator = tokens[index++],
        right = primary();
      if (
        !right ||
        (operator === "*" && left.unit && right.unit) ||
        (operator === "/" && right.unit)
      )
        return null;
      left = {
        value:
          operator === "*"
            ? left.value * right.value
            : left.value / right.value,
        unit: left.unit || right.unit,
      };
    }
    return left;
  }
  function sum() {
    let left = product();
    while (left && ["+", "-"].includes(tokens[index])) {
      const operator = tokens[index++],
        right = product();
      if (!right || left.unit !== right.unit) return null;
      left = {
        value: left.value + (operator === "+" ? right.value : -right.value),
        unit: left.unit,
      };
    }
    return left;
  }
  const result = sum();
  return result && index === tokens.length && Number.isFinite(result.value)
    ? result
    : null;
}

export function color(text) {
  let match = /^#([\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$/i.exec(text.trim());
  if (match) {
    let hex = match[1];
    if (hex.length <= 4) hex = [...hex].map((c) => c + c).join("");
    return {
      space: "srgb",
      channels: [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255),
      alpha: hex.length === 8 ? parseInt(hex.slice(6), 16) / 255 : 1,
    };
  }
  if (text.trim() === "transparent")
    return { space: "srgb", channels: [0, 0, 0], alpha: 0 };
  match = /^rgba?\(([^)]+)\)$/i.exec(text.trim());
  if (match) {
    const parts = match[1].trim().split(/[\s,/]+/);
    if (
      parts.length < 3 ||
      parts.length > 4 ||
      parts.some((part) => !/^[+-]?(\d*\.)?\d+%?$/.test(part))
    )
      return null;
    const component = (value, scale) =>
      Math.min(
        1,
        Math.max(0, parseFloat(value) / (value.endsWith("%") ? 100 : scale)),
      );
    return {
      space: "srgb",
      channels: parts.slice(0, 3).map((value) => component(value, 255)),
      alpha: parts.length === 4 ? component(parts[3], 1) : 1,
    };
  }
  return null;
}

/** Project resolved v4 values onto pm-design fields.
 *
 * `baseline` holds the frozen v1.2.0 foundation values for the same scheme at
 * normal density. A value equal to its baseline is the old app's default, not
 * an authored choice, so pm-design's native default stands (listed in
 * `nativeDefaults`). Without a baseline every value projects (legacy callers).
 *
 * `nativeTokens` (`"metrics.gap" -> 6`, from the host's current pm-design
 * tokens) makes length changes relative: a compact workspace that takes 4 px
 * off a v1.2.0 row takes 4 pt off the native row, instead of imposing the old
 * absolute size. Colours always project absolutely.
 */
export function designTokens(values, { baseline = null, nativeTokens = null } = {}) {
  const tokens = {},
    nativePatch = {},
    nativeDefaults = [],
    additions = [];
  const opaque = (name) => {
    const typed = values[name] == null ? null : color(values[name]);
    return typed?.alpha === 1 ? typed : null;
  };
  for (const [name, value] of Object.entries(values)) {
    if (value === null) continue;
    const spec = FOUNDATION_TOKENS[name] ?? {
      kind: "expression",
      native: null,
      v4: `custom.${name.slice(2)}`,
    };
    const typed =
      spec.kind === "color"
        ? color(value)
        : ["length", "number", "duration"].includes(spec.kind)
          ? scalar(value)
          : null;
    tokens[name] = { ...spec, value, typed };
    const fields = spec.native ?? [];
    if (fields.length && baseline && baseline[name] === value) {
      nativeDefaults.push(name);
      continue;
    }
    let projected = false;
    if (fields.length && spec.kind === "color" && typed) {
      let channels = typed.alpha === 1 ? typed.channels : null;
      const under = backdrop[name.slice(2)] && opaque(backdrop[name.slice(2)]);
      if (!channels && under)
        channels = typed.channels.map(
          (c, i) => c * typed.alpha + under.channels[i] * (1 - typed.alpha),
        );
      if (channels) {
        const rgb = channels.reduce(
          (sum, channel) => sum * 256 + Math.round(channel * 255),
          0,
        );
        for (const field of fields) nativePatch[field] = rgb;
        projected = true;
      }
    } else if (
      fields.length &&
      spec.kind === "length" &&
      typed &&
      typed.value >= 0 &&
      (typed.unit === "px" || (typed.unit === "" && typed.value === 0))
    ) {
      const before = baseline?.[name] == null ? null : scalar(baseline[name]);
      for (const field of fields) {
        const current = nativeTokens?.[field];
        nativePatch[field] =
          typeof current === "number" && before?.unit === "px"
            ? Math.max(0, current + typed.value - before.value)
            : typed.value;
      }
      projected = true;
    }
    if (!projected)
      additions.push({
        token: name,
        target: spec.v4,
        kind: spec.kind,
        reason: fields.length
          ? "native_field_cannot_represent_value"
          : "native_field_missing",
      });
  }
  return { tokens, nativePatch, nativeDefaults, additions };
}
