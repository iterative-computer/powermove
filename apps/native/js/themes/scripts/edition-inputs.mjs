// Build/test tooling only. Read public source; never load an installed profile.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { closing, parseCss } from "../css.mjs";
export const root = fileURLToPath(new URL("../../../../../", import.meta.url));
export const reference = "07ed5fa17ee640659cb075f9cad1af2095011d60";
export function tagged(path) {
  return execFileSync("git", ["show", `${reference}:${path}`], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 8e6,
  });
}
export function appRootCss() {
  return parseCss(tagged("apps/desktop/css/app.css"))
    .filter(
      (rule) =>
        rule.type === "rule" &&
        /^:root(?:\[data-theme="dark"\])?$/.test(rule.selector),
    )
    .map(
      (rule) =>
        `${rule.selector}{${rule.declarations
          .filter((d) => d.property.startsWith("--"))
          .map((d) => `${d.property}:${d.value};`)
          .join("")}}`,
    )
    .join("\n");
}

// Fail closed on new native field syntax. No Rust expressions are evaluated.
// Fixture generation is not a replacement for the design crate's Rust tests.
export function nativeSnapshots() {
  const rust = readFileSync(
    `${root}/apps/native/crates/design/src/lib.rs`,
    "utf8",
  );
  const number = (text) => {
    if (!/^(?:0x[\da-f]+|\d+(?:\.\d*)?)$/.test(text))
      throw Error(`Unrecognised native literal ${text}`);
    return Number(text);
  };
  const fields = (name) =>
    [
      ...rust
        .match(new RegExp(`pub struct ${name} \\{([^}]+)\\}`))[1]
        .matchAll(/pub (\w+):/g),
    ].map((m) => m[1]);
  function literals(struct, body) {
    return Object.fromEntries(
      fields(struct).map((key) => {
        const text = body
          .match(new RegExp(`\\b${key}:\\s*([^,]+),`))?.[1]
          .trim();
        return [key, number(text ?? "missing")];
      }),
    );
  }
  const snapshots = [];
  for (const scheme of ["Light", "Dark"])
    for (const increase_contrast of [false, true])
      for (const reduce_transparency of [false, true]) {
        const colors = literals(
          "Colors",
          rust.match(
            new RegExp(`Scheme::${scheme} => Colors \\{([^}]+)\\}`),
          )[1],
        );
        if (increase_contrast) {
          const block = rust.slice(
            rust.indexOf("if accessibility.increase_contrast"),
          );
          const changes = block.match(
            new RegExp(`Scheme::${scheme} => \\{([^}]+)\\}`),
          )[1];
          if (changes.replace(/colors\.\w+ = 0x[\da-f]+;/g, "").trim())
            throw Error("Native contrast expression syntax changed");
          for (const m of changes.matchAll(/colors\.(\w+) = (0x[\da-f]+);/g))
            colors[m[1]] = number(m[2]);
        }
        const metricBody = rust.match(
          /metrics: Metrics \{([\s\S]+?)\n            \},/,
        )[1];
        const border = metricBody.match(
          /border: if accessibility.increase_contrast \{\s*([\d.]+)\s*\} else \{\s*([\d.]+)\s*\}/,
        );
        if (!border) throw Error("Native border syntax changed");
        snapshots.push({
          scheme,
          accessibility: {
            reduce_transparency,
            increase_contrast,
            reduce_motion: false,
          },
          colors,
          metrics: literals(
            "Metrics",
            metricBody.replace(
              border[0],
              `border: ${border[increase_contrast ? 1 : 2]}`,
            ),
          ),
          typography: literals(
            "TypeScale",
            rust.match(/typography: TypeScale \{([^}]+)\}/)[1],
          ),
          motion: literals(
            "Motion",
            rust.match(/motion: Motion \{([^}]+)\}/)[1],
          ),
        });
      }
  return snapshots;
}

export function panelSources() {
  const paths = execFileSync(
    "git",
    [
      "ls-tree",
      "-r",
      "--name-only",
      reference,
      "--",
      "apps/desktop/src/extensions",
    ],
    { cwd: root, encoding: "utf8" },
  )
    .trim()
    .split("\n");
  return paths
    .filter(
      (path) =>
        /\.(ts|svelte|css)$/.test(path) && !/\.(test|spec)\./.test(path),
    )
    .map((path) => ({ path, text: tagged(path) }));
}

export function variableUsage({ path, text }) {
  const declarations = new Set();
  for (const m of text.matchAll(
    /(--[\w-]+)\s*:|setProperty\(\s*['"](--[\w-]+)['"]/g,
  ))
    declarations.add(m[1] ?? m[2]);
  const reads = [];
  for (const m of text.matchAll(/var\(\s*(--[\w-]+)/g))
    reads.push({ name: m[1], offset: m.index });
  for (const call of text.matchAll(/\b(?:getPropertyValue|css)\s*\(/g)) {
    const start = call.index + call[0].length;
    const argument = text.slice(start, closing(text, start - 1));
    // Include both branches of calls such as css(reversed ? '--bg-panel-2'
    // : '--bg-panel'). Nonliteral arbitrary dynamic keys remain a host gate.
    for (const m of argument.matchAll(/['"](--[\w-]+)['"]/g))
      reads.push({ name: m[1], offset: start + m.index });
  }
  return {
    path,
    declarations: [...declarations].sort(),
    reads: reads
      .sort((a, b) => a.offset - b.offset)
      .map(({ name, offset }) => ({
        name,
        line: text.slice(0, offset).split("\n").length,
      })),
  };
}
