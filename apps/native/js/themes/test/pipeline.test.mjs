import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import {
  parseCss,
  parseDeclarations,
  splitTop,
  translateTheme,
  evaluateTheme,
  resolveScheme,
  workspaceAppearance,
  resolveVariables,
  scalar,
  color,
  designTokens,
  ThemePublication,
  ThemeController,
  cutoverRequirements,
} from "../index.mjs";

const studio = JSON.parse(
  readFileSync(new URL("../fixtures/studio.json", import.meta.url)),
);
const builtins = JSON.parse(
  readFileSync(new URL("../fixtures/builtins.json", import.meta.url)),
);
const definition = (css) => ({
  id: "fixture",
  name: "Fixture",
  scheme: "auto",
  css,
});
const tree = () => ({
  id: "root",
  tag: "html",
  children: [
    {
      id: "body",
      cssId: "body",
      tag: "div",
      children: [
        {
          id: "dock-left",
          tag: "section",
          classes: ["dock"],
          children: [
            {
              id: "panel",
              tag: "article",
              classes: ["panel"],
              children: [
                { id: "header", tag: "header", classes: ["panel-header"] },
                { id: "content", tag: "div", classes: ["panel-body"] },
              ],
            },
            {
              id: "splitter",
              tag: "div",
              classes: ["splitter"],
              states: ["hover"],
            },
          ],
        },
        {
          id: "tabs",
          tag: "nav",
          container: { name: "tabs", width: 200, height: 40 },
          children: [
            {
              id: "tab",
              tag: "button",
              classes: ["project-tab"],
              attributes: { "aria-selected": "true" },
            },
          ],
        },
      ],
    },
  ],
});
const evaluate = (css, options = {}, environment = {}) =>
  evaluateTheme(translateTheme(definition(css), options), tree(), {
    systemScheme: "light",
    viewport: { width: 1200, height: 800 },
    ...environment,
  });

test("CSS parser retains quoted delimiters, nested functions, data URLs, comments and importance", () => {
  const css =
    '/* public */ :root, html { --f: "a;b}"; background: url("data:image/svg+xml,<svg>{}</svg>"); color: rgb(0 0 0 / .5) ! IMPORTANT; content: "!important" } @media (max-width:760px) { .a:is(.b,.c) {gap:var(--x, calc(3px * 2))} }';
  const ast = parseCss(css);
  assert.equal(ast.length, 2);
  assert.equal(ast[0].declarations[0].value, '"a;b}"');
  assert.equal(ast[0].declarations[2].important, true);
  assert.equal(ast[0].declarations[3].important, false);
  assert.equal(ast[1].children[0].selector, ".a:is(.b,.c)");
  assert.deepEqual(splitTop(':is(.a,.b), [x="a,b"]'), [
    ":is(.a,.b)",
    ' [x="a,b"]',
  ]);
});
test("malformed CSS is reported atomically, including unmatched escapes/comments/blocks", () => {
  for (const css of [
    ".a { color:red",
    '.a { content:"x }',
    ".a;",
    "/*",
    "@media(x){ .a{color:red}} trailing",
    ".a { broken }",
  ])
    assert.throws(() => parseCss(css));
  assert.throws(() => parseDeclarations("x{}:red"));
  assert.throws(
    () =>
      translateTheme({
        ...definition(""),
        tokens: { "--x": "red; color:blue" },
      }),
    { code: "invalid_token" },
  );
});
test("all scheme, user preference and system combinations keep fixed and auto distinct", () => {
  for (const scheme of ["auto", "light", "dark"])
    for (const preference of ["system", "light", "dark"])
      for (const systemScheme of ["light", "dark"]) {
        assert.equal(
          resolveScheme({ scheme }, preference, systemScheme),
          scheme === "auto"
            ? preference === "system"
              ? systemScheme
              : preference
            : scheme,
        );
      }
  assert.throws(() => resolveScheme({}, "auto"), { code: "invalid_scheme" });
});
test("both shipped built-in themes translate for both schemes", () => {
  for (const builtin of builtins)
    for (const systemScheme of ["light", "dark"]) {
      const result = evaluateTheme(
        translateTheme(builtin, { systemScheme }),
        tree(),
      );
      assert.equal(result.scheme, systemScheme);
      assert.equal(Object.keys(result.tokens).length, 82);
      assert.deepEqual(result.blockers, []);
      if (builtin.id === "high-contrast")
        assert.equal(
          result.nativePatch["colors.text"],
          systemScheme === "light" ? 0 : 0xffffff,
        );
    }
});
test("specificity, source order, !important and inline theme/workspace layers", () => {
  const input = {
    ...definition(
      ":root {--accent:#111111 !important;} :root[data-theme] {--bg-window:#121212;} #tab {color:#111} .project-tab {color:#222} #tabs #tab {color:#333} #tabs #tab {color:#444}",
    ),
    tokens: { "--accent": "#222222", "--bg-window": "#232323" },
  };
  const result = evaluateTheme(
    translateTheme(input, {
      workspace: { theme: { accent: "#333333", bg: "#343434" } },
    }),
    tree(),
  );
  assert.equal(result.nativePatch["colors.accent"], 0x111111);
  assert.equal(result.nativePatch["colors.window"], 0x343434);
  assert.equal(result.surfaces.tab.base.style.color, "#444");
});
test("custom properties resolve fallback chains, nested fallback commas and missing references", () => {
  const result = resolveVariables({
    "--a": "var(--missing, var(--b, rgb(1, 2, 3)))",
    "--b": "#123",
    "--c": "var(--unknown)",
    "--empty": "",
    "--literal": '"var(--b)"',
  });
  assert.equal(result.values["--a"].trim(), "#123");
  assert.equal(result.values["--c"], null);
  assert.equal(result.values["--empty"], "");
  assert.equal(result.values["--literal"], '"var(--b)"');
  assert.deepEqual(result.dependencies["--a"], ["--missing", "--b"]);
});
test("cycles including references inside fallbacks invalidate every cycle member", () => {
  const result = resolveVariables({
    "--a": "var(--b, red)",
    "--b": "var(--a, blue)",
    "--recover": "var(--a, green)",
    "--hidden": "var(--known, var(--hidden))",
    "--known": "ok",
  });
  assert.equal(result.values["--a"], null);
  assert.equal(result.values["--b"], null);
  assert.equal(result.values["--recover"].trim(), "green");
  assert.equal(result.values["--hidden"], null);
  assert.equal(result.cycles.length, 3);
});
test("custom property names stay case-sensitive and inherited aliases compute at their parent", () => {
  const result = evaluate(
    ":root{--A:red;--a:blue;--alias:var(--a)} #tab{--a:green; color:var(--alias); background:var(--A)}",
  );
  assert.equal(result.surfaces.tab.base.style.color, "blue");
  assert.equal(result.surfaces.tab.base.style.background, "red");
});
test("invalid-at-computed-time values inherit instead of reviving losing declarations", () => {
  const result = evaluate(
    "#tabs{color:blue} #tab{color:red;color:var(--missing);width:var(--missing)}",
  );
  assert.equal(result.surfaces.tab.base.style.color, "blue");
  assert.equal(result.surfaces.tab.base.style.width, undefined);
});
test("workspace overrides derive accent family and radius, preserve geometry by stable ids", () => {
  const workspace = {
    theme: { accent: "#789abc", radius: 1, font: "Fixture Sans" },
    density: "compact",
    chrome: {
      previewCornerRadius: "rounded",
      timelineSurfaceOrder: "reversed",
    },
    layout: {
      docks: [
        { id: "left", size: 280, panels: [{ id: "inspector", size: 240 }] },
      ],
    },
  };
  const plan = translateTheme(studio, { workspace });
  const result = evaluateTheme(plan, tree(), {
    viewport: { width: 1000, height: 800 },
    reduceMotion: true,
  });
  assert.equal(result.nativePatch["metrics.row"], 26);
  assert.equal(result.nativePatch["metrics.panel_radius"], 2);
  assert.equal(
    result.tokens["--accent-dim"].value,
    "color-mix(in srgb, #789abc 12%, transparent)",
  );
  assert.equal(result.tokens["--f-ui"].value, "Fixture Sans");
  assert.deepEqual(plan.workspace.layout, workspace.layout);
  assert.equal(
    result.painterInputs.timeline.track,
    result.tokens["--bg-panel-2"].value,
  );
  assert.equal(result.painterInputs.previewCorners, "rounded");
});
test("fixture end-to-end: docking, layout, tabs, decoration, container and motion", () => {
  const result = evaluateTheme(
    translateTheme(studio, { systemScheme: "dark" }),
    tree(),
    { viewport: { width: 900, height: 700 }, reduceMotion: true },
  );
  assert.deepEqual(result.blockers, []);
  assert.equal(result.rootAttributes["data-custom-attribute"], "preserved");
  assert.equal(result.nativePatch["colors.accent"], 0xabcdef);
  assert.equal(
    result.surfaces.body.base.style["grid-template-columns"],
    "280px minmax(180px, 1fr) 320px",
  );
  assert.equal(result.surfaces["dock-left"].base.style.width, "280px");
  assert.equal(result.surfaces.header.base.style.height, "38px");
  assert.equal(result.surfaces.tab.base.style.width, "64px");
  assert.equal(result.surfaces.tab.base.style.color, "#abcdef");
  assert.equal(result.surfaces.tab.before.style.background, "#abcdef");
  assert.equal(result.surfaces.tab.base.style["transition-duration"], "0ms");
  assert.equal(result.surfaces.splitter.base.style.cursor, "col-resize");
  assert.equal(result.motion[0].frames.length, 2);
  assert.equal(result.motion[0].name, "fixture-enter");
  assert.ok(
    result.surfaces.body.base.operations.some(
      (operation) => operation.domain === "layout",
    ),
  );
  assert.doesNotThrow(() => JSON.stringify(result));
});
test("media conditions, named ancestor containers and boundary dimensions", () => {
  const css =
    "@media (max-width:760px){#tab{width:1px}} @media (min-width:760px) and (hover: hover) and (pointer: fine){#tab{height:2px}} @container tabs (max-width:200px){#tab{padding:3px}}";
  const result = evaluate(css, {}, { viewport: { width: 760, height: 600 } });
  assert.equal(result.surfaces.tab.base.style.width, "1px");
  assert.equal(result.surfaces.tab.base.style.height, "2px");
  assert.equal(result.surfaces.tab.base.style.padding, "3px");
  const wider = evaluate(css, {}, { viewport: { width: 761, height: 600 } });
  assert.equal(wider.surfaces.tab.base.style.width, undefined);
});
test("unsupported shell declarations and conditions remain explicit native blockers", () => {
  const result = evaluate(
    "@media (future-feature: yes){#tab{width:1px}} #tab{future-paint:shiny}",
  );
  assert.ok(
    result.blockers.some((blocker) => blocker.code === "unsupported_condition"),
  );
  assert.ok(result.blockers.some((blocker) => blocker.code === "property"));
  assert.equal(result.compat.length, 0);
  const layers = evaluate("@layer intended {#tab{color:red}}");
  assert.ok(layers.blockers.some((blocker) => blocker.code === "at_rule"));
});
test("Store registration strips attributes, accepts only original global token CSS grammar", () => {
  const plan = translateTheme(
    {
      ...definition(":root,body {--accent:#123456}"),
      rootAttributes: { "data-brand": "x" },
    },
    { trust: "store" },
  );
  assert.equal(plan.rootAttributes["data-brand"], undefined);
  for (const css of [
    ".project-tab{--x:red}",
    ":root {color:red}",
    ":root{--x:url(https://example.invalid)}",
    ":root{--x:URL(x)}",
    '@import "x";',
    ":root{--x:var(\\78)}",
  ])
    assert.throws(() => translateTheme(definition(css), { trust: "store" }), {
      code: "store_theme_denied",
    });
});
test("compat content is isolated from mandatory shell translation", () => {
  const plan = translateTheme(definition("iframe{filter:blur(1px)}"), {
    document: "visual-reply",
  });
  assert.equal(plan.compat.length, 1);
  assert.ok(plan.rules.every((rule) => rule.document === "shell"));
  assert.equal(evaluateTheme(plan, tree()).compat[0].selector.raw, "iframe");
});
test("exact metric arithmetic, typed alpha colors, no lossy native colour projection", () => {
  assert.deepEqual(scalar("calc(4px * 3)"), { value: 12, unit: "px" });
  assert.deepEqual(scalar("calc(4px + 2px)"), { value: 6, unit: "px" });
  assert.equal(scalar("calc(4px + 2)"), null);
  assert.equal(scalar("calc(4px / 0)"), null);
  assert.equal(scalar("2px; evil()"), null);
  assert.equal(color("#1234").alpha, 0x44 / 255);
  const projection = designTokens({
    "--tx-2": "rgb(20 22 28 / .66)",
    "--accent": "#123456",
  });
  assert.equal(projection.nativePatch["colors.secondary"], undefined);
  assert.equal(projection.tokens["--tx-2"].typed.alpha, 0.66);
  assert.equal(projection.nativePatch["colors.accent"], 0x123456);
  assert.equal(
    projection.additions[0].reason,
    "native_field_cannot_represent_value",
  );
});
test("theme replacement un-applies old tokens, attributes and shell rules in full", () => {
  const controller = new ThemeController();
  const first = controller.api("one");
  first.register(studio);
  first.activate(studio.id);
  const previous = evaluateTheme(controller.plan(), tree(), {
    viewport: { width: 1000, height: 800 },
  });
  assert.equal(previous.surfaces["dock-left"].base.style.width, "280px");
  first.dispose();
  const next = evaluateTheme(controller.plan(), tree());
  assert.equal(next.rootAttributes["data-studio"], undefined);
  assert.equal(next.tokens["--fixture-line"], undefined);
  assert.equal(next.surfaces["dock-left"].base.style.width, undefined);
  assert.equal(next.surfaces.tab.before, undefined);
});
test("publication rejects stale work and copies full immutable snapshots to Linux host", () => {
  const host = {
    current: null,
    replace(value) {
      this.current = value;
    },
  };
  const publisher = new ThemePublication(host);
  const old = publisher.request(),
    current = publisher.request();
  const snapshot = evaluate("");
  assert.equal(publisher.commit(old, snapshot), false);
  assert.equal(host.current, null);
  assert.equal(publisher.commit(current, snapshot), true);
  snapshot.rootAttributes["data-stale"] = "changed";
  assert.equal(host.current.snapshot.rootAttributes["data-stale"], undefined);
  publisher.invalidate();
  assert.equal(publisher.commit(current, snapshot), false);
});
test("cutover capability audit cannot certify missing token fields or host style support", () => {
  const snapshot = evaluate("#tab{color:red}");
  const missing = cutoverRequirements(snapshot, {});
  assert.ok(missing.some((item) => item.code === "design_token"));
  assert.ok(missing.some((item) => item.code === "surface_property"));
  assert.deepEqual(
    cutoverRequirements(snapshot, {
      tokens: snapshot.additions.map((item) => item.target),
      properties: ["color", "color-scheme"],
    }),
    [],
  );
});
test("runtime modules have no Node/browser imports or browser globals", () => {
  const folder = new URL("../", import.meta.url);
  for (const file of readdirSync(folder).filter((file) =>
    file.endsWith(".mjs"),
  )) {
    const text = readFileSync(new URL(file, folder), "utf8");
    assert.doesNotMatch(
      text,
      /(?:from\s*|import\s*)['"](?:node:|https?:|fs['"]|path['"])/,
    );
    assert.doesNotMatch(
      text,
      /\b(?:document|window|process|require|fetch)\s*[.(]/,
    );
  }
});

test("translator, registry and publication run with bare V8 intrinsics", () => {
  execFileSync(
    process.execPath,
    [
      "--experimental-vm-modules",
      new URL("./bare-v8.mjs", import.meta.url).pathname,
    ],
    { stdio: "pipe" },
  );
});

test("authored root style replaces theme inline tokens, then workspace applies last", () => {
  const input = {
    ...definition(""),
    tokens: { "--accent": "#111111", "--only-inline": "lost" },
    rootAttributes: {
      style: "--accent:#222222; --authored:preserved",
      "data-extra": 42,
    },
  };
  const plan = translateTheme(input, {
    workspace: { theme: { accent: "#333333" } },
  });
  const result = evaluateTheme(plan, tree());
  assert.equal(result.nativePatch["colors.accent"], 0x333333);
  assert.equal(result.tokens["--only-inline"], undefined);
  assert.equal(result.tokens["--authored"].value, "preserved");
  assert.equal(result.rootAttributes["data-extra"], "42");
});

test("root inline importance and direct trusted CSS/class/attribute side effects participate", () => {
  const t = tree();
  t.attributes = {
    style: "--accent:#111111 !important; color:red",
    "data-audio-state": "playing",
  };
  const plan = translateTheme({
    ...definition(
      "html.native-app[data-audio-state=playing] #tab{height:31px}",
    ),
    tokens: { "--accent": "#222222" },
    rootAttributes: { class: "native-app fixture" },
  });
  const result = evaluateTheme(plan, t);
  assert.equal(result.nativePatch["colors.accent"], 0x111111);
  assert.equal(result.surfaces.tab.base.style.height, "31px");
  assert.equal(result.surfaces.tab.base.style.color, "red");
});

test("motion and transparency preferences and system CSS media follow environment", () => {
  const css =
    "@media (prefers-reduced-transparency:reduce){#tab{background:#123}} @media (prefers-color-scheme:dark){#tab{color:#456}} @media (prefers-reduced-motion:no-preference){#tab{transition-duration:75ms}} @supports (backdrop-filter:blur(1px)){#tab{backdrop-filter:blur(1px)}}";
  const result = evaluate(
    css,
    {},
    { reduceTransparency: true, systemScheme: "dark", supports: () => true },
  );
  assert.deepEqual(result.blockers, []);
  assert.equal(result.surfaces.tab.base.style.background, "#123");
  assert.equal(result.surfaces.tab.base.style.color, "#456");
  assert.equal(result.surfaces.tab.base.style["transition-duration"], "75ms");
  assert.equal(result.surfaces.tab.base.style["backdrop-filter"], "blur(1px)");
});

test("ordered operations preserve shorthand/longhand declaration and priority order", () => {
  const result = evaluate(
    "#tab{background:red;background-color:blue;background:green} .project-tab{background-color:yellow !important}",
  );
  const ops = result.surfaces.tab.base.operations.filter((op) =>
    op.property.startsWith("background"),
  );
  assert.deepEqual(
    ops.map((op) => [op.property, op.value]),
    [
      ["background", "green"],
      ["background-color", "yellow"],
    ],
  );
  const sameRule = evaluate(
    "#tab{background:red;background-color:blue;background:green}",
  );
  assert.deepEqual(
    sameRule.surfaces.tab.base.operations
      .filter((op) => op.property.startsWith("background"))
      .map((op) => op.value),
    ["blue", "green"],
  );
});

test("named highlights stay separate and invalid computed declarations carry a reset operation", () => {
  const result = evaluate(
    "#tab::highlight(search){color:#123} #tab::highlight(other){color:#456} #tab{width:var(--missing)}",
  );
  assert.equal(result.surfaces.tab["highlight(search)"].style.color, "#123");
  assert.equal(result.surfaces.tab["highlight(other)"].style.color, "#456");
  const reset = result.surfaces.tab.base.operations.find(
    (operation) => operation.property === "width",
  );
  assert.equal(reset.invalidAtComputedTime, true);
  assert.equal(reset.value, null);
  assert.equal(reset.authoredValue, "var(--missing)");
});

test("semantic document boundaries isolate legacy DOM descendants from shell rules", () => {
  const input = tree();
  input.children.push({
    id: "legacy",
    tag: "iframe",
    document: "legacy-content",
    children: [{ id: "legacy-button", tag: "button" }],
  });
  const result = evaluateTheme(
    translateTheme(definition("button{color:red}")),
    input,
  );
  assert.equal(result.surfaces.tab.base.style.color, "red");
  assert.equal(result.surfaces["legacy-button"].base.style.color, undefined);
  assert.deepEqual(result.surfaces["legacy-button"].base.variables.values, {});
});
test("no theme leaves pm-design's native defaults alone (no v1.2.0 Stone dark, no 999 capsule)", () => {
  for (const scheme of ["light", "dark"]) {
    const result = evaluateTheme(
      translateTheme(definition(""), { preference: scheme, systemScheme: scheme }),
      tree(),
    );
    assert.deepEqual(result.nativePatch, {}, scheme);
    assert.ok(result.nativeDefaults.includes("--bg-window"));
    assert.ok(result.nativeDefaults.includes("--gap"));
  }
  const dark = evaluateTheme(
    translateTheme(
      { ...definition(""), tokens: { "--bg-panel": "#202020" } },
      { systemScheme: "dark" },
    ),
    tree(),
  );
  assert.deepEqual(dark.nativePatch, { "colors.panel": 0x202020 });
});
test("translucent theme colours composite over the panel; lengths move relative to native", () => {
  const result = evaluateTheme(
    translateTheme(
      {
        ...definition(""),
        tokens: { "--bg-panel": "#000000", "--line": "rgb(255 255 255 / .25)" },
      },
      { systemScheme: "dark", workspace: { density: "compact" } },
    ),
    tree(),
    { nativeTokens: { "metrics.row": 28, "metrics.gap": 6, "metrics.header": 32 } },
  );
  assert.equal(result.nativePatch["colors.separator"], 0x404040);
  // compact: v1.2.0 row 30 -> 26 and gap 10 -> 8, applied as offsets.
  assert.equal(result.nativePatch["metrics.row"], 24);
  assert.equal(result.nativePatch["metrics.gap"], 4);
  assert.equal(result.nativePatch["typography.reading"], 12);
  assert.equal(result.nativePatch["typography.title"], 12);
  assert.equal(result.nativePatch["metrics.capsule_radius"], undefined);
  assert.equal(result.nativePatch["metrics.border"], undefined);
});
