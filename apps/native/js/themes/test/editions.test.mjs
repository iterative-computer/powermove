import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  createCompatEdition,
  compatThemeMessage,
  CompatThemeReceiver,
  createWebKitThemeHost,
  translateTheme,
  evaluateTheme,
  color,
  scalar,
  FOUNDATION_TOKENS,
} from "../index.mjs";
import {
  nativeSnapshots,
  panelSources,
  variableUsage,
  appRootCss,
} from "../scripts/edition-inputs.mjs";
import { APP_ROOT_CSS } from "../edition-foundation.mjs";
const json = (path) =>
  JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
const natives = json("../fixtures/native-tokens-v1.json");
const manifest = json("../editions/v1/manifest.json");
const usages = json("../fixtures/builtin-variable-reads.json");
const tree = {
  id: "root",
  tag: "html",
  children: [{ id: "panel", tag: "section", classes: ["corpus-panel"] }],
};
function inspect(edition, nodeTree = tree) {
  const plan = translateTheme(
    {
      id: edition.themeId,
      scheme: "auto",
      css: edition.stylesheet + edition.themeCss,
    },
    { foundation: false, preference: edition.scheme },
  );
  plan.rootAttributes = edition.rootAttributes;
  plan.inlineTokens = edition.inlineTokens;
  return evaluateTheme(plan, nodeTree);
}
function variables(edition) {
  return inspect(edition).surfaces.root.base.variables.values;
}
function native(scheme = "Light", contrast = false, solid = false) {
  return structuredClone(
    natives.find(
      (n) =>
        n.scheme === scheme &&
        n.accessibility.increase_contrast === contrast &&
        n.accessibility.reduce_transparency === solid,
    ),
  );
}

test("native snapshot and app root source oracles are current", () => {
  assert.deepEqual(natives, nativeSnapshots());
  assert.equal(APP_ROOT_CSS, appRootCss());
  assert.deepEqual(
    usages.sources,
    panelSources()
      .map(variableUsage)
      .filter((s) => s.reads.length),
  );
  assert.equal(
    usages.sources.reduce((n, source) => n + source.reads.length, 0),
    390,
  );
});

test("edition artifacts check without writing", () => {
  execFileSync(process.execPath, [
    new URL("../scripts/editions.mjs", import.meta.url).pathname,
    "--check",
  ]);
});

for (const input of natives) {
  const edition = createCompatEdition(input);
  test(`golden ${edition.id}`, () => {
    const entry = manifest.editions.find((item) => item.id === edition.id);
    assert.ok(entry);
    assert.equal(
      readFileSync(
        new URL(`../editions/v1/${entry.file}`, import.meta.url),
        "utf8",
      ),
      edition.stylesheet,
    );
    assert.equal(
      createHash("sha256").update(edition.stylesheet).digest("hex"),
      entry.sha256,
    );
    const values = variables(edition);
    assert.equal(Object.keys(values).length, 97);
    for (const name of Object.keys(FOUNDATION_TOKENS))
      assert.ok(values[name], name);
    for (const [name, value] of Object.entries(values))
      assert.ok(value !== null && !value.includes("var("), name);
    assert.deepEqual(inspect(edition).blockers, []);
    // Every literal read in every public built-in, including timeline css(name),
    // has either an edition value or a declaration/setProperty in that source.
    // Local declarations stay local; do not invent global panel geometry.
    for (const source of usages.sources)
      for (const read of source.reads)
        assert.ok(
          values[read.name] != null || source.declarations.includes(read.name),
          `${source.path}:${read.line}: ${read.name}`,
        );
    for (const [token, field] of [
      ["--bg-panel", "panel"],
      ["--bg-panel-2", "panel_secondary"],
      ["--bg-float", "floating"],
      ["--bg-window", "window"],
      ["--tx", "text"],
      ["--tx-2", "secondary"],
      ["--tx-3", "muted"],
      ["--line", "separator"],
      ["--accent", "accent"],
    ]) {
      const typed = color(values[token]);
      assert.equal(
        typed.channels.reduce((n, c) => n * 256 + Math.round(c * 255), 0),
        input.colors[field],
      );
    }
    if (edition.scheme === "dark") {
      for (const token of [
        "--bg-window",
        "--bg-window-solid",
        "--bg-rail",
        "--bg-panel",
        "--bg-panel-2",
        "--bg-row-hi",
        "--bg-field",
        "--bg-float",
        "--bg-sunken",
        "--tx",
        "--tx-2",
        "--tx-3",
        "--gray",
      ])
        assert.equal(new Set(color(values[token]).channels).size, 1, token);
    }
    if (input.accessibility.reduce_transparency) {
      for (const name of [
        "--bg-window",
        "--bg-content",
        "--glass-fill",
        "--glass-fill-hi",
        "--ctl-raised",
      ])
        assert.equal(color(values[name]).alpha, 1, name);
      assert.equal(values["--glass-backdrop"], "none");
    }
  });
}

test("native values are inputs, not a frozen second palette", () => {
  const input = native("Dark");
  input.colors.panel = 0x123456;
  input.colors.window = 0x345678;
  input.colors.floating = 0x456789;
  input.colors.accent = 0x987654;
  input.metrics.row = 47;
  input.typography.body = 17;
  input.typography.label = 18;
  input.typography.title = 19;
  input.typography.reading = 23;
  const values = variables(createCompatEdition(input));
  assert.equal(values["--bg-panel"], "#123456");
  assert.equal(values["--bg-float"], "#456789");
  assert.equal(values["--bg-window"], "#345678");
  assert.equal(values["--accent"], "#987654");
  assert.equal(values["--row-h"], "47px");
  assert.equal(values["--fs-sm"], "17px");
  assert.equal(values["--fs-md"], "19px");
});

test("review 1: edition fixtures expand the design crate's serialized snapshots", () => {
  const snapshots = json("../../../crates/design/tests/tokens.snapshot.json");
  for (const input of natives) {
    const expected = structuredClone(
      snapshots.find(
        (snapshot) =>
          snapshot.scheme === input.scheme &&
          snapshot.accessibility.increase_contrast ===
            input.accessibility.increase_contrast,
      ),
    );
    expected.accessibility.reduce_transparency =
      input.accessibility.reduce_transparency;
    assert.deepEqual(input, expected);
  }
  assert.deepEqual(FOUNDATION_TOKENS["--fs-md"].native, [
    "typography.title",
    "typography.reading",
  ]);
});

test("reviews 2, 3, 5, 6, 9: native surfaces, capsules, type and CSS hairlines in every edition", () => {
  for (const input of natives) {
    const values = variables(createCompatEdition(input));
    for (const [name, field] of [
      ["--bg-float", "floating"],
      ["--bg-window", "window"],
    ]) {
      const expected = `#${input.colors[field].toString(16).padStart(6, "0")}`;
      assert.equal(values[name], expected);
      assert.equal(color(values[name]).alpha, 1);
    }
    assert.notEqual(values["--bg-float"], values["--bg-panel"]);
    assert.equal(values["--r-pill"], "999px");
    assert.equal(values["--fs-xs"], "11px");
    assert.equal(values["--fs-sm"], "12px");
    assert.equal(values["--fs-md"], "13px");
    assert.equal(values["--fs-lg"], "15px");
    assert.equal(values["--fs-xl"], "18px");
    assert.equal(values["--hairline"], "0.5px");
  }
});

test("review 4: density uses native-relative lengths for both schemes and accessibility axes", () => {
  const names = ["--row-h", "--gap", "--hdr-h", "--ctl-h", "--pad", "--fs-md"];
  for (const source of natives) {
    const input = structuredClone(source);
    for (const [density, expected] of [
      ["normal", [28, 6, 32, 28, 12, 13]],
      ["compact", [24, 4, 26, 24, 9, 12]],
      ["comfy", [32, 8, 36, 30, 14, 14]],
    ]) {
      const values = variables(
        createCompatEdition(input, { workspace: { density } }),
      );
      assert.deepEqual(
        names.map((name) => scalar(values[name]).value),
        expected,
      );
    }
    input.metrics.row += 10;
    input.metrics.gap += 10;
    input.metrics.header += 10;
    input.metrics.control += 10;
    input.metrics.inset += 10;
    input.typography.title += 10;
    for (const [density, expected] of [
      ["compact", [34, 14, 36, 34, 19, 22]],
      ["comfy", [42, 18, 46, 40, 24, 24]],
    ]) {
      const values = variables(
        createCompatEdition(input, { workspace: { density } }),
      );
      assert.deepEqual(
        names.map((name) => scalar(values[name]).value),
        expected,
      );
    }
  }
});

test("reviews 7 and 8: row lift and translucent accent remain reactive to authored backgrounds", () => {
  for (const input of natives) {
    const edition = createCompatEdition(input);
    assert.ok(
      edition.stylesheet.includes(
        "--bg-row:color-mix(in srgb,var(--tx) 4%,var(--bg-panel));",
      ),
    );
    assert.ok(
      edition.stylesheet.includes(
        "--accent-dim:color-mix(in srgb,var(--accent) 12%,transparent);",
      ),
    );
    const authored = createCompatEdition(input, {
      theme: {
        id: "authored",
        scheme: "auto",
        tokens: {
          "--tx": "#ffffff",
          "--bg-panel": "#202020",
          "--accent": "#ff0000",
        },
      },
    });
    const values = variables(authored);
    assert.equal(values["--bg-row"], "color-mix(in srgb,#ffffff 4%,#202020)");
    assert.equal(
      values["--accent-dim"],
      "color-mix(in srgb,#ff0000 12%,transparent)",
    );
  }
});

test("density, source specificity, inline/workspace, important and radius alias cascade", () => {
  const theme = {
    id: "corpus",
    scheme: "auto",
    tokens: { "--accent": "#112233", "--local-brand": "#123456" },
    darkTokens: { "--accent": "#abcdef" },
    css: ":root{--bg-panel:#998877;--accent:#000000!important}.corpus-panel{--local-fill:var(--local-brand);color:var(--tx);background:var(--local-fill);padding:var(--gut)}",
    rootAttributes: { "data-corpus": "enabled", class: "corpus-root" },
  };
  const edition = createCompatEdition(native("Dark"), {
    theme,
    workspace: { density: "compact", theme: { accent: "#335577", radius: 24 } },
  });
  const result = inspect(edition);
  const values = result.surfaces.root.base.variables.values;
  assert.equal(values["--accent"], "#000000"); // !important wins over normal inline
  assert.equal(values["--bg-panel"], "#1a1a1a"); // dark specificity beats later :root
  assert.equal(values["--row-h"], "24px");
  assert.equal(scalar(values["--r-lg"]).value, 24);
  assert.equal(scalar(values["--r-sm"]).value, 12);
  assert.equal(result.surfaces.panel.base.style.background, "#123456");
  assert.equal(result.surfaces.panel.base.style.padding, "7px");
  assert.equal(edition.rootAttributes["data-corpus"], "enabled");
  const normal = createCompatEdition(native("Dark"), {
    theme: { ...theme, css: "" },
    workspace: { theme: { accent: "#335577" } },
  });
  assert.equal(variables(normal)["--accent"], "#335577");
  assert.equal(
    variables(
      createCompatEdition(native("Dark"), { theme: { ...theme, css: "" } }),
    )["--accent"],
    "#abcdef",
  );
  assert.equal(
    variables(
      createCompatEdition(native(), { workspace: { density: "comfy" } }),
    )["--row-h"],
    "32px",
  );
});

test("root style replacement, custom attributes and Store permissions retain shipped semantics", () => {
  const edition = createCompatEdition(native(), {
    theme: {
      id: "style",
      scheme: "auto",
      tokens: { "--stale": "bad" },
      rootAttributes: {
        style: "--accent:#112233;--root-only:retained",
        "data-other": "yes",
      },
    },
    workspace: { theme: { accent: "#445566" } },
  });
  assert.equal(variables(edition)["--accent"], "#445566");
  assert.equal(variables(edition)["--root-only"], "retained");
  assert.equal(variables(edition)["--stale"], undefined);
  const store = createCompatEdition(native(), {
    trust: "store",
    theme: {
      id: "store",
      scheme: "auto",
      rootAttributes: { "data-denied": "yes" },
      css: ":root{--store-color:#123456}",
    },
  });
  assert.equal(store.rootAttributes["data-denied"], undefined);
  assert.equal(variables(store)["--store-color"], "#123456");
  assert.throws(
    () =>
      createCompatEdition(native(), {
        trust: "store",
        theme: { id: "store", scheme: "auto", css: ".panel{color:red}" },
      }),
    { code: "store_theme_denied" },
  );
});

test("synthetic corpus panel reads all edition names and resolves local aliases/fallbacks", () => {
  const names = Object.keys(variables(createCompatEdition(native())));
  for (const input of natives) {
    const edition = createCompatEdition(input, {
      theme: {
        id: "corpus",
        scheme: "auto",
        css: `.corpus-panel{${names.map((name, i) => `--read-${i}:var(${name});`).join("")}--local:var(--missing, var(--tx));color:var(--local);}`,
      },
    });
    const values = inspect(edition).surfaces.panel.base.variables.values;
    for (const [i, name] of names.entries())
      assert.equal(values[`--read-${i}`], values[name]);
    assert.equal(values["--local"].trim(), values["--tx"]);
  }
});

// Minimal render-side Document double. No browser or native UI is claimed.
function documentDouble() {
  const styles = new Map(),
    attrs = new Map();
  const make = () => ({
    attrs: new Map(),
    textContent: "",
    setAttribute(k, v) {
      this.attrs.set(k, v);
    },
  });
  const panelStyle = make(),
    head = {
      children: [panelStyle],
      get firstChild() {
        return this.children[0];
      },
      insertBefore(node, before) {
        this.children.splice(this.children.indexOf(before), 0, node);
      },
      appendChild(node) {
        this.children.push(node);
      },
    };
  const root = {
    style: {
      removeProperty(k) {
        styles.delete(k);
      },
      setProperty(k, v) {
        styles.set(k, v);
      },
    },
    setAttribute(k, v) {
      attrs.set(k, v);
      if (k === "style") styles.clear();
    },
    removeAttribute(k) {
      attrs.delete(k);
      if (k === "style") styles.clear();
    },
  };
  return {
    head,
    documentElement: root,
    createElement: make,
    createAttribute(k) {
      if (!/^[a-z_:][\w.:-]*$/i.test(k)) throw Error("Invalid attribute");
      return { name: k };
    },
    styles,
    attrs,
    panelStyle,
  };
}

test("live full replacement retains nodes, focus/selection, panel CSS and unrelated root state", () => {
  const doc = documentDouble(),
    events = [];
  doc.activeElement = { id: "text-editor", selectionStart: 4, selectionEnd: 9 };
  const focus = doc.activeElement;
  doc.styles.set("--panel-owned", "keep");
  doc.attrs.set("data-panel-owned", "keep");
  const receiver = new CompatThemeReceiver(
    "panel/editor@1",
    createWebKitThemeHost(doc, (event) =>
      events.push({
        event,
        accent: doc.styles.get("--accent"),
        attrs: new Map(doc.attrs),
      }),
    ),
  );
  const first = createCompatEdition(native(), {
    theme: {
      id: "one",
      scheme: "auto",
      tokens: { "--old-only": "remove", "--accent": "#123456" },
      rootAttributes: { "data-old": "yes" },
      css: ".panel{color:red}/* </style><script>ignored</script> */",
    },
  });
  const payload = compatThemeMessage("panel/editor@1", 1, first);
  assert.equal(receiver.receive(payload), true);
  assert.equal(doc.head.children[1], doc.panelStyle);
  const nodes = [...doc.head.children];
  assert.ok(nodes[2].textContent.includes("</style>"));
  const second = createCompatEdition(native("Dark", true, true));
  assert.equal(
    receiver.receive(compatThemeMessage("panel/editor@1", 3, second)),
    true,
  );
  assert.deepEqual(doc.head.children, nodes);
  assert.equal(doc.activeElement, focus);
  assert.equal(focus.selectionStart, 4);
  assert.equal(focus.selectionEnd, 9);
  assert.equal(doc.styles.has("--old-only"), false);
  assert.equal(doc.attrs.has("data-old"), false);
  assert.equal(doc.attrs.get("data-theme"), "dark");
  assert.equal(doc.styles.get("--panel-owned"), "keep");
  assert.equal(doc.attrs.get("data-panel-owned"), "keep");
  assert.equal(nodes[2].textContent, "");
  assert.equal(events.length, 2);
  assert.equal(events[0].accent, "#123456");
  assert.equal(events[1].attrs.get("data-theme"), "dark");
  assert.equal(receiver.receive(payload), false);
  assert.equal(
    receiver.receive(compatThemeMessage("panel/editor@1", 3, second)),
    false,
  );
  assert.equal(
    receiver.receive(compatThemeMessage("panel/editor@0", 99, first)),
    false,
  );
  assert.equal(events.length, 2);
});

test("messages copy state and reject unsupported/malformed input before replacement", () => {
  const applied = [];
  const receiver = new CompatThemeReceiver("doc", {
    replaceTheme: (e) => applied.push(e),
  });
  const edition = createCompatEdition(native());
  const payload = compatThemeMessage("doc", 1, edition);
  edition.inlineTokens["--later"] = "ignored";
  assert.equal(receiver.receive(payload), true);
  payload.edition.inlineTokens["--after"] = "ignored";
  assert.deepEqual(applied[0].inlineTokens, {});
  const good = compatThemeMessage("doc", 2, createCompatEdition(native()));
  for (const mutate of [
    (m) => (m.version = 2),
    (m) => (m.edition.version = 2),
    (m) => (m.generation = NaN),
    (m) => (m.edition.inlineTokens = { "--x": "red; color:blue" }),
    (m) => (m.edition.inlineTokens = { "--x": "red!important" }),
    (m) => (m.edition.rootAttributes = { "bad name": "x" }),
    (m) => (m.edition.rootAttributes = []),
    (m) => (m.edition.inlineTokens = []),
    (m) => (m.edition.accessibility = null),
    (m) => (m.edition.accessibility.increase_contrast = "true"),
    (m) => (m.edition.id = "compat-v2/dark"),
  ]) {
    const bad = structuredClone(good);
    mutate(bad);
    assert.throws(() => receiver.receive(bad));
    assert.equal(applied.length, 1);
    assert.equal(receiver.generation, 1);
  }
  assert.equal(receiver.receive(good), true);
  assert.equal(applied.length, 2);
  assert.throws(() => compatThemeMessage("doc", 0, edition));
  assert.throws(() => compatThemeMessage("", 1, edition));
});

test("failed host publication can be retried; popouts start from a complete snapshot", () => {
  let fail = true;
  const copies = [];
  const host = {
    replaceTheme: (e) => {
      if (fail) throw Error("port unavailable");
      copies.push(e);
    },
  };
  const receiver = new CompatThemeReceiver("doc", host);
  const payload = compatThemeMessage(
    "doc",
    7,
    createCompatEdition(native("Dark", true, true)),
  );
  assert.throws(() => receiver.receive(payload));
  assert.equal(receiver.generation, 0);
  fail = false;
  assert.equal(receiver.receive(payload), true);
  const popout = new CompatThemeReceiver("popout@1", host);
  assert.equal(
    popout.receive(compatThemeMessage("popout@1", 7, payload.edition)),
    true,
  );
  assert.deepEqual(copies[0], copies[1]);
});

test("accessibility modes compose and preserve explicit authored colours", () => {
  const input = native("Dark", true, true);
  input.accessibility.reduce_motion = true;
  const values = variables(
    createCompatEdition(input, {
      theme: {
        id: "custom",
        scheme: "auto",
        tokens: { "--bg-panel": "#123456" },
      },
    }),
  );
  assert.equal(values["--bg-panel"], "#123456");
  assert.equal(values["--line-strong"], values["--line"]);
  assert.equal(values["--dur-1"], "75ms"); // existing extension media/motion rules remain in charge
  assert.equal(values["--hairline"], "0.5px");
  assert.equal(values["--focus-ring"], "0 0 0 2px #123456,0 0 0 5px #ffa073");
});

test("bad native inputs and unresolved fixed scheme fail visibly", () => {
  for (const mutate of [
    (n) => (n.scheme = "System"),
    (n) => (n.accessibility.increase_contrast = 1),
    (n) => (n.colors.accent = -1),
    (n) => (n.metrics.row = Infinity),
    (n) => (n.typography.reading = Infinity),
    (n) => (n.metrics.panel_radius = 0),
  ]) {
    const input = native();
    mutate(input);
    assert.throws(() => createCompatEdition(input), {
      code: "invalid_edition",
    });
  }
  assert.throws(
    () =>
      createCompatEdition(native(), { theme: { id: "dark", scheme: "dark" } }),
    { code: "invalid_edition" },
  );
  assert.equal(
    createCompatEdition(native("Dark"), {
      theme: { id: "dark", scheme: "dark" },
    }).scheme,
    "dark",
  );
});

test("render completion can deliver a newer edition without generation regression", () => {
  const first = compatThemeMessage("doc", 1, createCompatEdition(native()));
  const next = compatThemeMessage(
    "doc",
    2,
    createCompatEdition(native("Dark")),
  );
  const receiver = new CompatThemeReceiver("doc", {
    replaceTheme() {
      if (receiver.generation === 1) {
        assert.equal(receiver.receive(first), false);
        assert.equal(receiver.receive(next), true);
      }
    },
  });
  assert.equal(receiver.receive(first), true);
  assert.equal(receiver.generation, 2);
  assert.equal(receiver.receive(next), false);
});

test("invalid DOM attribute names do not touch the working edition", () => {
  const doc = documentDouble();
  const receiver = new CompatThemeReceiver("doc", createWebKitThemeHost(doc));
  const first = compatThemeMessage("doc", 1, createCompatEdition(native()));
  receiver.receive(first);
  const current = doc.head.children[0].textContent;
  const bad = compatThemeMessage("doc", 2, createCompatEdition(native("Dark")));
  bad.edition.rootAttributes["@invalid"] = "x";
  assert.throws(() => receiver.receive(bad));
  assert.equal(receiver.generation, 1);
  assert.equal(doc.head.children[0].textContent, current);
  assert.equal(doc.attrs.get("data-theme"), "light");
});

test("coverage detects missing dependencies in a new synthetic panel", () => {
  const values = variables(createCompatEdition(native()));
  const usage = variableUsage({
    path: "synthetic.css",
    text: ".panel{color:var(--not-provided);background:var(--bg-panel)}",
  });
  const missing = usage.reads.filter(
    (read) => !(read.name in values) && !usage.declarations.includes(read.name),
  );
  assert.deepEqual(
    missing.map((read) => read.name),
    ["--not-provided"],
  );
  assert.deepEqual(
    variableUsage({
      path: "conditional.ts",
      text: "css(reversed ? '--bg-panel-2' : '--bg-panel')",
    }).reads.map((read) => read.name),
    ["--bg-panel-2", "--bg-panel"],
  );
});

function contrast(a, b) {
  const luminance = (text) =>
    color(text)
      .channels.map((c) =>
        c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
      )
      .reduce((n, c, i) => n + c * [0.2126, 0.7152, 0.0722][i], 0);
  const x = luminance(a),
    y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
test("text and Increase Contrast separators retain native contrast on panel surfaces", () => {
  for (const input of natives) {
    const values = variables(createCompatEdition(input));
    for (const surface of ["--bg-panel", "--bg-panel-2"])
      for (const ink of ["--tx", "--tx-2", "--tx-3"])
        assert.ok(
          contrast(values[ink], values[surface]) >= 4.5,
          `${input.scheme} ${ink} on ${surface}`,
        );
    if (input.accessibility.increase_contrast)
      for (const line of [
        "--line",
        "--line-2",
        "--line-strong",
        "--section-line",
      ])
        assert.ok(contrast(values[line], values["--bg-panel"]) >= 3, line);
  }
});
