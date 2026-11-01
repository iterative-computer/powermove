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

// The design crate owns serialization, including nested labels and new roles.
// Expand its four appearance snapshots with the material-only RT axis.
// Fixture generation is not a replacement for the design crate's Rust tests.
export function nativeSnapshots() {
  const snapshots = JSON.parse(
    readFileSync(
      `${root}/apps/native/crates/design/tests/tokens.snapshot.json`,
      "utf8",
    ),
  );
  return ["Light", "Dark"].flatMap((scheme) =>
    [false, true].flatMap((increase_contrast) => {
      const native = snapshots.find(
        (snapshot) =>
          snapshot.scheme === scheme &&
          snapshot.accessibility.increase_contrast === increase_contrast,
      );
      if (!native)
        throw Error(`Missing native snapshot ${scheme} ${increase_contrast}`);
      return [false, true].map((reduce_transparency) => ({
        ...native,
        accessibility: { ...native.accessibility, reduce_transparency },
      }));
    }),
  );
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
