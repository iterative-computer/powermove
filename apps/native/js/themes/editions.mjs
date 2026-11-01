import { FOUNDATION_CSS } from "./foundation.mjs";
import { APP_ROOT_CSS } from "./edition-foundation.mjs";
import { parseCss, parseDeclarations, ThemeError } from "./css.mjs";
import { copyData } from "./data.mjs";
import { FOUNDATION_TOKENS, scalar } from "./tokens.mjs";
import { translateTheme } from "./translator.mjs";

export const COMPAT_THEME_VERSION = 1;
export const COMPAT_THEME_MESSAGE = "powermove.compat-theme.replace";

function invalid(message) {
  throw new ThemeError("invalid_edition", message);
}

function hex(value) {
  if (!Number.isInteger(value) || value < 0 || value > 0xffffff)
    invalid("Native colours must be opaque sRGB u32 RGB values");
  return `#${value.toString(16).padStart(6, "0")}`;
}

function px(value) {
  if (!Number.isFinite(value) || value < 0) invalid("Invalid native metric");
  return `${value}px`;
}

function channels(value) {
  hex(value);
  return [16, 8, 0].map((shift) => (value >> shift) & 255).join(" ");
}

function nativeValues(native) {
  const values = {};
  for (const [name, spec] of Object.entries(FOUNDATION_TOKENS)) {
    if (!spec.native) continue;
    const projected = spec.native.map((target) => {
      const [group, key] = target.split(".");
      const value = native[group]?.[key];
      return spec.kind === "color" ? hex(value) : px(value);
    });
    // One CSS token can feed several native roles; the first is its CSS role.
    values[name] = projected[0];
  }
  // Keep authored alias chains reactive to theme/workspace overrides.
  const base = native.metrics.panel_radius / 3;
  if (!(base > 0)) invalid("Panel radius must be positive for legacy aliases");
  values["--r-base"] = px(base);
  values["--r-sm"] =
    `calc(var(--r-base) * ${native.metrics.control_radius / base})`;
  values["--r-lg"] = "calc(var(--r-base) * 3)";
  values["--ink-rgb"] = channels(native.colors.text);
  values["--bg-rail"] = hex(native.colors.window);
  values["--bg-row"] = "color-mix(in srgb,var(--tx) 4%,var(--bg-panel))";
  values["--gray"] = "var(--tx-3)";
  values["--accent-hover"] = "color-mix(in srgb,var(--accent) 90%,white)";
  values["--accent-dim"] = "color-mix(in srgb,var(--accent) 12%,transparent)";
  values["--accent-tx"] = "color-mix(in oklab,var(--accent) 82%,var(--tx))";
  values["--focus-ring"] =
    `0 0 0 2px var(--bg-panel),0 0 0 ${native.accessibility.increase_contrast ? 5 : 4}px ${hex(native.colors.focus)}`;
  if (native.accessibility.increase_contrast) {
    for (const name of ["--line-2", "--line-strong", "--section-line"])
      values[name] = "var(--line)";
    values["--tx-4"] = "var(--tx-3)";
  }
  return values;
}

function printRule(selector, declarations) {
  return `${selector}{\n${declarations.map(({ property, value, important }) => `  ${property}:${value}${important ? "!important" : ""};`).join("\n")}\n}\n`;
}

/** Reverse adapter for a serialized pm-design::Tokens snapshot. All generation
 * happens on the worker/realm. No global document, Node or host calls here.
 * A resolved native scheme is required; the kernel still owns preferences.
 */
export function createCompatEdition(nativeInput, options = {}) {
  const native = copyData(nativeInput);
  if (!["Light", "Dark"].includes(native?.scheme))
    invalid("Unknown native scheme");
  for (const key of [
    "increase_contrast",
    "reduce_transparency",
    "reduce_motion",
  ])
    if (typeof native.accessibility?.[key] !== "boolean")
      invalid(`Missing native accessibility.${key}`);
  const scheme = native.scheme.toLowerCase();
  const plan = translateTheme(
    options.theme ?? { id: "default", scheme: "auto" },
    {
      ...options,
      preference: scheme,
      systemScheme: scheme,
      foundation: false,
      document: "legacy-content",
    },
  );
  if (plan.scheme !== scheme)
    invalid("Resolve fixed theme scheme before creating native tokens");
  const values = nativeValues(native);
  const solid = native.accessibility.reduce_transparency;
  const window = hex(native.colors.window);
  const id = `compat-v1/${scheme}${native.accessibility.increase_contrast ? "-contrast" : ""}${solid ? "-solid" : ""}`;
  let stylesheet = `/* Powermove ${id}; generated from pm-design::Tokens. */\n`;
  const foundation = parseCss(FOUNDATION_CSS);
  const normal = Object.fromEntries(
    foundation
      .filter((rule) => rule.selector === ":root")
      .flatMap((rule) => rule.declarations.map((d) => [d.property, d.value])),
  );
  // Preserve rule order/specificity, with the review's native surface and
  // relative-density corrections. Authored theme CSS still follows app CSS.
  for (const rule of parseCss(FOUNDATION_CSS + "\n" + APP_ROOT_CSS)) {
    const density = rule.selector.includes("data-density");
    const dark = rule.selector.includes("data-theme");
    if (dark && scheme !== "dark") continue;
    const declarations = rule.declarations.map((declaration) => {
      const { property } = declaration;
      let value = density
        ? declaration.value
        : (values[property] ?? declaration.value);
      const spec = FOUNDATION_TOKENS[property];
      if (density && spec?.native && spec.kind === "length") {
        const before = scalar(normal[property]);
        const after = scalar(declaration.value);
        const current = scalar(values[property]);
        if (before?.unit !== "px" || after?.unit !== "px" || !current)
          invalid(`Invalid density metric ${property}`);
        value = px(Math.max(0, current.value + after.value - before.value));
      }
      if (property === "color-scheme") value = scheme;
      if (property === "--bg-window-solid") value = window;
      // Compat documents occupy solid panels; AppKit owns window materials.
      if (property === "--bg-window") value = window;
      if (solid) {
        const replacements = {
          "--bg-content": "var(--bg-panel)",
          "--glass-fill": "var(--bg-window-solid)",
          "--glass-fill-hi": "var(--bg-field)",
          "--glass-backdrop": "none",
          "--ctl-raised": "var(--bg-field)",
        };
        value = replacements[property] ?? value;
      }
      return { ...declaration, value };
    });
    stylesheet += printRule(rule.selector, declarations);
  }
  // These shipped consumers had no global definition. Define semantic aliases
  // rather than preserving invalid-at-computed-value browser accidents (N3).
  stylesheet += printRule(
    ":root",
    Object.entries({
      "font-variant-numeric": "tabular-nums",
      "--bg-hover": "var(--bg-row-hi)",
      "--font-mono": "var(--f-mono)",
      "--orange": "var(--accent)",
      "--tx-1": "var(--tx)",
      "--bg-2": "var(--bg-panel-2)",
    }).map(([property, value]) => ({ property, value })),
  );
  return {
    version: COMPAT_THEME_VERSION,
    id,
    themeId: plan.id,
    scheme,
    accessibility: native.accessibility,
    stylesheet,
    themeCss: String(plan.archive.css ?? ""),
    rootAttributes: plan.rootAttributes,
    inlineTokens: plan.inlineTokens,
  };
}

/** A full replacement, never a delta: coalescing/reordering cannot leave stale
 * theme-only variables behind. documentId identifies a document incarnation.
 */
export function compatThemeMessage(documentId, generation, edition) {
  if (
    typeof documentId !== "string" ||
    !documentId ||
    !Number.isSafeInteger(generation) ||
    generation < 1
  )
    invalid(
      "A document incarnation id and positive safe generation are required",
    );
  validateEdition(edition);
  return copyData({
    type: COMPAT_THEME_MESSAGE,
    version: COMPAT_THEME_VERSION,
    documentId,
    generation,
    edition,
  });
}

function validateEdition(edition) {
  const dictionary = (value) =>
    value !== null && typeof value === "object" && !Array.isArray(value);
  if (
    edition?.version !== COMPAT_THEME_VERSION ||
    !["light", "dark"].includes(edition.scheme) ||
    typeof edition.id !== "string" ||
    !/^compat-v1\/(light|dark)(-contrast)?(-solid)?$/.test(edition.id) ||
    typeof edition.themeId !== "string" ||
    !edition.themeId ||
    typeof edition.stylesheet !== "string" ||
    typeof edition.themeCss !== "string" ||
    !dictionary(edition.rootAttributes) ||
    !dictionary(edition.inlineTokens) ||
    !dictionary(edition.accessibility)
  )
    invalid("Malformed theme edition");
  for (const key of [
    "increase_contrast",
    "reduce_transparency",
    "reduce_motion",
  ])
    if (typeof edition.accessibility[key] !== "boolean")
      invalid("Malformed accessibility flags");
  for (const [key, value] of Object.entries(edition.rootAttributes))
    if (!key || /[\s<>/="'\u0000]/.test(key) || typeof value !== "string")
      invalid("Invalid root attribute");
  for (const [key, value] of Object.entries(edition.inlineTokens)) {
    if (typeof value !== "string") invalid("Invalid inline token");
    const declarations = parseDeclarations(`${key}:${value}`);
    if (
      !key.startsWith("--") ||
      declarations.length !== 1 ||
      declarations[0].property !== key ||
      declarations[0].important
    )
      invalid("Invalid inline token");
  }
}

/** Transport-independent receiver. The WebKit adapter and Linux test double
 * implement the same replaceTheme port. Native UI only queues data to WebKit.
 */
export class CompatThemeReceiver {
  constructor(documentId, host) {
    if (
      typeof documentId !== "string" ||
      !documentId ||
      typeof host?.replaceTheme !== "function"
    )
      invalid("Invalid compat theme host");
    this.documentId = documentId;
    this.host = host;
    this.generation = 0;
  }
  receive(input) {
    const message = copyData(input);
    if (
      message?.type !== COMPAT_THEME_MESSAGE ||
      message.version !== COMPAT_THEME_VERSION
    )
      invalid("Unsupported compat theme message");
    if (message.documentId !== this.documentId) return false;
    if (!Number.isSafeInteger(message.generation) || message.generation < 1)
      invalid("Invalid generation");
    if (message.generation <= this.generation) return false;
    validateEdition(message.edition);
    const previous = this.generation;
    this.generation = message.generation;
    try {
      this.host.replaceTheme(message.edition);
    } catch (error) {
      // A completion callback may have synchronously delivered a newer edition.
      if (this.generation === message.generation) this.generation = previous;
      throw error;
    }
    return true;
  }
}
