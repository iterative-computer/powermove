import { parseCss, parseDeclarations, ThemeError } from "./css.mjs";
import { copyData } from "./data.mjs";
import {
  matches,
  parseSelectorList,
  selectorRequirements,
  specificity,
  SurfaceTree,
} from "./selectors.mjs";
import { conditionMatches } from "./conditions.mjs";
import { designTokens, resolveVariables, substitute } from "./tokens.mjs";
import { FOUNDATION_CSS } from "./foundation.mjs";

const domains = {
  layout:
    "display position inset inset-block inset-inline top right bottom left width height min-width max-width min-height max-height box-sizing aspect-ratio margin margin-top margin-right margin-bottom margin-left margin-block margin-inline padding padding-top padding-right padding-bottom padding-left padding-inline padding-block gap row-gap column-gap flex flex-grow flex-shrink flex-basis flex-direction flex-wrap align-items align-self align-content justify-content justify-items justify-self place-items place-content grid grid-template grid-template-columns grid-template-rows grid-auto-columns grid-auto-rows grid-auto-flow grid-column grid-row order overflow overflow-x overflow-y overflow-anchor overscroll-behavior contain container container-type container-name content-visibility scrollbar-gutter scrollbar-width scroll-behavior scroll-padding scroll-margin resize vertical-align float clear",
  paint:
    "background background-color background-image background-size background-position background-repeat background-clip background-origin background-attachment color opacity border border-width border-style border-color border-top border-right border-bottom border-left border-top-color border-bottom-color border-left-color border-right-color border-top-width border-bottom-width border-radius border-top-left-radius border-top-right-radius border-bottom-left-radius border-bottom-right-radius outline outline-color outline-width outline-style outline-offset box-shadow fill stroke stroke-width stroke-linecap stroke-linejoin paint-order accent-color caret-color color-scheme appearance",
  text: "font font-family font-size font-weight font-style font-variant font-feature-settings font-variation-settings font-optical-sizing font-variant-numeric line-height letter-spacing word-spacing text-align text-decoration text-decoration-color text-decoration-line text-decoration-thickness text-underline-offset text-transform text-overflow text-wrap text-indent text-shadow white-space overflow-wrap word-break word-wrap hyphens tab-size line-clamp -webkit-line-clamp -webkit-box-orient",
  interaction:
    "cursor pointer-events user-select touch-action visibility z-index isolation",
  material:
    "filter backdrop-filter mix-blend-mode background-blend-mode mask mask-image mask-size mask-position mask-repeat clip-path clip object-fit object-position",
  motion:
    "transform transform-origin transform-style perspective perspective-origin translate rotate scale transition transition-property transition-duration transition-delay transition-timing-function animation animation-name animation-duration animation-delay animation-timing-function animation-iteration-count animation-direction animation-fill-mode animation-play-state will-change",
  decoration:
    "content counter-reset counter-increment list-style list-style-type list-style-position list-style-image quotes",
};
const propertyDomains = Object.fromEntries(
  Object.entries(domains).flatMap(([domain, properties]) =>
    properties.split(" ").map((property) => [property, domain]),
  ),
);
const semantics = {
  "-webkit-app-region": {
    domain: "interaction",
    property: "window-drag-region",
  },
  "-webkit-user-select": { domain: "interaction", property: "user-select" },
  "-webkit-backdrop-filter": {
    domain: "material",
    property: "backdrop-filter",
  },
  "-webkit-mask": { domain: "material", property: "mask" },
  "-webkit-mask-image": { domain: "material", property: "mask-image" },
};
const rasterization = new Set([
  "-webkit-font-smoothing",
  "-moz-osx-font-smoothing",
  "text-rendering",
]);
const inheritedProperties = new Set(
  "color font font-family font-size font-weight font-style line-height letter-spacing word-spacing text-align text-transform white-space cursor visibility".split(
    " ",
  ),
);

export function lowerDeclaration(declaration) {
  const { property, value, important } = declaration;
  if (property.startsWith("--"))
    return { domain: "token", property, value, important };
  if (rasterization.has(property))
    return {
      domain: "platform",
      property,
      value,
      important,
      decision: "native_text_rasterization",
      rule: "N3/quirk policy",
    };
  return {
    domain:
      propertyDomains[property] ?? semantics[property]?.domain ?? "required",
    property: semantics[property]?.property ?? property,
    cssProperty: property,
    value,
    important,
  };
}

export function resolveScheme(
  definition,
  preference = "system",
  systemScheme = "light",
) {
  if (
    !["light", "dark", "system"].includes(preference) ||
    !["light", "dark"].includes(systemScheme)
  )
    throw new ThemeError("invalid_scheme", "Invalid appearance preference");
  return definition?.scheme === "light" || definition?.scheme === "dark"
    ? definition.scheme
    : preference === "system"
      ? systemScheme
      : preference;
}

export function workspaceAppearance(workspace = {}) {
  const tokens = {},
    attributes = {};
  const map = {
    accent: "--accent",
    bg: "--bg-window",
    panel: "--bg-panel",
    text: "--tx",
    line: "--line",
    font: "--f-ui",
    mono: "--f-mono",
  };
  const theme = workspace.theme ?? {};
  for (const [key, property] of Object.entries(map))
    if (theme[key]) tokens[property] = String(theme[key]);
  if (theme.accent) {
    tokens["--accent-dim"] =
      `color-mix(in srgb, ${theme.accent} 12%, transparent)`;
    tokens["--accent-tx"] =
      `color-mix(in oklab, ${theme.accent} 82%, var(--tx))`;
  }
  if (theme.radius != null) {
    if (!Number.isFinite(theme.radius))
      throw new ThemeError("invalid_workspace", "Radius must be finite");
    tokens["--r-base"] = `${Math.max(2, theme.radius) / 3}px`;
  }
  attributes["data-density"] = workspace.density ?? "normal";
  attributes["data-preview-corners"] =
    workspace.chrome?.previewCornerRadius ?? "square";
  attributes["data-timeline-surfaces"] =
    workspace.chrome?.timelineSurfaceOrder ?? "normal";
  return {
    tokens,
    attributes,
    layout: copyData(workspace.layout ?? null),
    chrome: copyData(workspace.chrome ?? {}),
  };
}

function storeDefinition(definition) {
  // Keep the shipped Store gate at least as restrictive, including stripping
  // attributes. This is a permission gate, not the trusted CSS parser.
  const css = definition.css ? String(definition.css) : "";
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  if (
    rules
      .map((match) => match[0])
      .join("")
      .replace(/\s/g, "") !== css.replace(/\s/g, "") ||
    rules.some(
      (match) =>
        !/^(?:\s*(?::root|html|body)\s*,?)+$/.test(match[1]) ||
        match[2]
          .split(";")
          .filter(Boolean)
          .some(
            (declaration) =>
              !/^\s*--[\w-]+\s*:\s*[^;{}]+\s*$/.test(declaration) ||
              /url\s*\(|@|\\/i.test(declaration),
          ),
    )
  ) {
    throw new ThemeError(
      "store_theme_denied",
      "Store theme CSS may only set custom properties on :root, html, or body",
    );
  }
  return { ...definition, rootAttributes: {} };
}

function inlineTokens(dictionary = {}) {
  return Object.fromEntries(
    Object.entries(dictionary)
      .filter(([key]) => key.startsWith("--"))
      .map(([key, value]) => {
        // setProperty(name, value) does not interpret a trailing !important.
        // Treat that as invalid input rather than accidentally escalating priority.
        const declarations = parseDeclarations(`${key}:${String(value)}`);
        if (
          declarations.length !== 1 ||
          declarations[0].property !== key ||
          declarations[0].important
        ) {
          throw new ThemeError(
            "invalid_token",
            "Token dictionary value is not a single CSS value",
            { key },
          );
        }
        return [key, String(value)];
      }),
  );
}

/** Compile a v1–3 definition into an inspectable, JSON-serializable v4 program.
 * Work is performed on a realm/worker. The shell consumes a published result;
 * it must never synchronously invoke this compiler on its main thread (N8).
 */
export function translateTheme(input, options = {}) {
  const definition = options.trust === "store" ? storeDefinition(input) : input;
  if (
    !definition ||
    typeof definition.id !== "string" ||
    !definition.id ||
    !["auto", "light", "dark"].includes(definition.scheme)
  ) {
    throw new ThemeError(
      "invalid_theme",
      "Theme needs an id and light/dark/auto scheme",
    );
  }
  const scheme = resolveScheme(
    definition,
    options.preference,
    options.systemScheme,
  );
  const workspace = workspaceAppearance(options.workspace);
  const rootAttributes = Object.fromEntries(
    Object.entries({
      "data-theme": scheme,
      ...definition.rootAttributes,
      ...workspace.attributes,
    }).map(([key, value]) => [key, String(value)]),
  );
  // rootAttributes.style is authored setAttribute('style', ...): it replaces
  // the inline dictionary written immediately before it. Workspace goes last.
  const themeInline = Object.hasOwn(definition.rootAttributes ?? {}, "style")
    ? {}
    : {
        ...inlineTokens(definition.tokens),
        ...(scheme === "dark" ? inlineTokens(definition.darkTokens) : {}),
      };
  const tokens = { ...themeInline, ...workspace.tokens };
  const plan = {
    version: 4,
    id: definition.id,
    scheme,
    preference: options.preference ?? "system",
    rootAttributes,
    inlineTokens: tokens,
    workspace,
    rules: [],
    motion: [],
    compat: [],
    requirements: [],
    // Caller persists this in its private archive; never commit imported themes.
    archive: copyData(input),
  };
  let order = 0;
  function compile(nodes, conditions, document, source) {
    for (const node of nodes) {
      if (node.type === "at") {
        if (["media", "container", "supports"].includes(node.name)) {
          compile(
            node.children,
            [...conditions, { kind: node.name, query: node.prelude }],
            document,
            source,
          );
        } else if (["keyframes", "-webkit-keyframes"].includes(node.name)) {
          plan.motion.push({
            name: node.prelude,
            frames: parseCss(node.body),
            conditions,
            document,
            source,
          });
        } else {
          plan.requirements.push({
            code: "at_rule",
            atRule: node,
            conditions,
            document,
            source,
            disposition: document === "shell" ? "native_required" : "compat",
          });
        }
        continue;
      }
      for (const selector of parseSelectorList(node.selector)) {
        const rule = {
          id: `${source}:${order}`,
          selector,
          specificity: specificity(selector),
          conditions,
          operations: node.declarations.map(lowerDeclaration),
          order: order++,
          document,
          source,
        };
        if (document === "shell" || document === "native-content") {
          plan.rules.push(rule);
          for (const needed of selectorRequirements(selector))
            plan.requirements.push({
              code: "selector",
              needed,
              rule: rule.id,
              disposition: "native_required",
            });
          for (const operation of rule.operations)
            if (operation.domain === "required")
              plan.requirements.push({
                code: "property",
                property: operation.property,
                rule: rule.id,
                disposition: "native_required",
              });
        } else plan.compat.push(rule);
      }
    }
  }
  if (options.foundation !== false)
    compile(parseCss(FOUNDATION_CSS), [], "shell", "v1.2.0/tokens");
  compile(
    parseCss(String(definition.css ?? "")),
    [],
    options.document ?? "shell",
    definition.id,
  );
  return plan;
}

const compare = (a, b) => {
  if (a.important !== b.important)
    return Number(a.important) - Number(b.important);
  for (let i = 0; i < a.specificity.length; i++)
    if (a.specificity[i] !== b.specificity[i])
      return a.specificity[i] - b.specificity[i];
  return a.order - b.order || (a.declaration ?? 0) - (b.declaration ?? 0);
};

/** Execute against semantic surface ids. Unknown features produce blocking
 * diagnostics; a false condition is the only reason a valid rule is skipped.
 * Native consumers must refuse a cutover certificate if blockers are nonempty.
 */
export function evaluateTheme(plan, inputTree, environment = {}) {
  const root = copyData(inputTree.root ?? inputTree);
  root.attributes = { ...root.attributes, ...plan.rootAttributes };
  if (Object.hasOwn(root.attributes, "class"))
    root.classes = String(root.attributes.class).split(/\s+/).filter(Boolean);
  const tree = new SurfaceTree(root);
  const documents = new Map();
  const surfaces = {},
    blockers = [...plan.requirements];
  const decorations = new Map(
    plan.rules.flatMap((rule) =>
      rule.selector.parts
        .at(-1)
        .tests.filter((test) => test.kind === "element")
        .map((test) => [
          `${test.name}${test.argument === null ? "" : `(${test.argument})`}`,
          test,
        ]),
    ),
  );
  const contexts = [null, ...decorations.keys()];
  const rootStyle = parseDeclarations(root.attributes.style ?? "");
  for (const node of tree.nodes.values()) {
    const parent = tree.parent(node),
      document = node.document ?? (parent ? documents.get(parent.id) : "shell");
    documents.set(node.id, document);
    const parentResult =
      parent && documents.get(parent.id) === document
        ? surfaces[parent.id]?.base
        : null;
    surfaces[node.id] = {};
    for (const context of contexts) {
      const decoration = decorations.get(context),
        pseudoElement = decoration?.name ?? null;
      const winners = new Map();
      const add = (property, candidate) => {
        if (
          !winners.has(property) ||
          compare(candidate, winners.get(property)) >= 0
        )
          winners.set(property, candidate);
      };
      for (const rule of plan.rules) {
        if (document !== rule.document) continue;
        try {
          if (
            !rule.conditions.every((condition) =>
              conditionMatches(condition, node, tree, environment),
            )
          )
            continue;
          if (
            !matches(rule.selector, node, tree, {
              pseudoElement,
              pseudoArgument: decoration?.argument,
              pseudo: environment.pseudo,
            })
          )
            continue;
        } catch (error) {
          if (!(error instanceof ThemeError)) throw error;
          blockers.push({
            code: error.code,
            detail: error.detail,
            rule: rule.id,
            surface: node.id,
          });
          continue;
        }
        for (const [declaration, operation] of rule.operations.entries())
          add(operation.cssProperty ?? operation.property, {
            ...operation,
            specificity: [0, ...rule.specificity],
            order: rule.order,
            declaration,
            rule: rule.id,
          });
      }
      if (!pseudoElement) {
        const inline = [
          ...(node === root ? rootStyle : []),
          ...parseDeclarations(node.inlineStyle ?? ""),
        ];
        for (const [index, declaration] of inline.entries())
          add(declaration.property, {
            ...lowerDeclaration(declaration),
            specificity: [1, 0, 0, 0],
            order: plan.rules.length + index,
            rule: `${node.id}/inline`,
          });
        if (node === root)
          for (const [property, value] of Object.entries(plan.inlineTokens))
            add(property, {
              domain: "token",
              property,
              value,
              important: false,
              specificity: [1, 0, 0, 0],
              order: Number.MAX_SAFE_INTEGER,
              rule: "theme/workspace",
            });
      }
      const inherited = pseudoElement ? surfaces[node.id].base : parentResult;
      const raw = Object.fromEntries(
        [...winners]
          .filter(([property]) => property.startsWith("--"))
          .map(([property, winner]) => [property, winner.value]),
      );
      const variables = resolveVariables(raw, inherited?.variables.values);
      const style = {},
        operations = [],
        provenance = {};
      for (const [property, winner] of winners) {
        if (property.startsWith("--")) continue;
        let value = substitute(
          winner.value,
          (name) => variables.values[name] ?? null,
        );
        if (value === null || /^(unset|inherit)$/i.test(value))
          value =
            inheritedProperties.has(property) || value === "inherit"
              ? (inherited?.style[property] ?? null)
              : null;
        if (value === "initial") value = null;
        if (value !== null) {
          style[property] = value;
          operations.push({ ...winner, authoredValue: winner.value, value });
          provenance[property] = winner.rule;
        } else
          operations.push({
            ...winner,
            authoredValue: winner.value,
            value: null,
            invalidAtComputedTime: true,
          });
        if (
          winner.domain === "required" ||
          /\b(revert|revert-layer)\b/.test(winner.value)
        )
          blockers.push({
            code: "native_property_required",
            surface: node.id,
            property,
            value: winner.value,
          });
      }
      for (const property of inheritedProperties)
        if (!winners.has(property) && inherited?.style[property] !== undefined)
          style[property] = inherited.style[property];
      operations.sort(compare);
      surfaces[node.id][context ?? "base"] = {
        style,
        operations,
        provenance,
        variables,
      };
    }
  }
  const palette = designTokens(surfaces[root.id].base.variables.values);
  const reversed = root.attributes["data-timeline-surfaces"] === "reversed";
  const values = surfaces[root.id].base.variables.values;
  return {
    version: 4,
    id: plan.id,
    scheme: plan.scheme,
    rootAttributes: root.attributes,
    surfaces,
    ...palette,
    painterInputs: {
      timeline: {
        track: values[reversed ? "--bg-panel-2" : "--bg-panel"],
        gutter: values[reversed ? "--bg-panel" : "--bg-panel-2"],
        accent: values["--accent"],
        text: values["--tx"],
        line: values["--line"],
      },
      gizmo: { accent: values["--accent"] },
      previewCorners: root.attributes["data-preview-corners"],
    },
    motion: copyData(plan.motion),
    compat: copyData(plan.compat),
    blockers,
  };
}

export function compatSelectors(catalog) {
  return catalog.rows
    .filter((row) => row.class === "C")
    .map((row) => ({
      source: row.source,
      line: row.line,
      selector: row.selector,
      conditions: row.conditions,
      surface: row.surface,
    }));
}
