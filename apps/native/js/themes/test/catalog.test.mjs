import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import {
  compatSelectors,
  parseSelector,
  printSelector,
  selectorRequirements,
  specificity,
  translateTheme,
  FOUNDATION_TOKENS,
  parseCss,
  FOUNDATION_CSS,
} from "../index.mjs";

const catalog = JSON.parse(
  readFileSync(new URL("../fixtures/t1-catalog.json", import.meta.url)),
);

test("frozen catalog, foundation and built-ins exactly reproduce public source", () => {
  execFileSync(process.execPath, [
    new URL("../scripts/catalog.mjs", import.meta.url).pathname,
    "--check",
  ]);
  assert.equal(catalog.rows.length, 3329);
  assert.equal(catalog.rows.filter((row) => row.class === "T").length, 3310);
  assert.equal(new Set(catalog.rows.map((row) => row.source)).size, 74);
});

test("all 82 foundation tokens have an explicit typed v4 destination", () => {
  const properties = new Set(
    parseCss(FOUNDATION_CSS)
      .flatMap(
        (node) =>
          node.declarations?.map((declaration) => declaration.property) ?? [],
      )
      .filter((property) => property.startsWith("--")),
  );
  assert.deepEqual(new Set(Object.keys(FOUNDATION_TOKENS)), properties);
  assert.equal(properties.size, 82);
  for (const spec of Object.values(FOUNDATION_TOKENS))
    assert.ok(spec.v4 && spec.kind);
});

test("19 compat occurrences stay confined to three non-shell source files", () => {
  const compat = compatSelectors(catalog);
  assert.equal(compat.length, 19);
  assert.equal(new Set(compat.map((row) => row.source)).size, 3);
  assert.ok(compat.some((row) => row.selector === "iframe"));
  assert.ok(compat.some((row) => row.selector === "html[data-noscroll]"));
  assert.ok(
    compat.some(
      (row) =>
        row.selector === '[contenteditable]:not([contenteditable="false"])',
    ),
  );
});

for (const [index, row] of catalog.rows.entries()) {
  test(`T1 ${index + 1} ${row.class}: ${row.selector}`, () => {
    const selector = parseSelector(row.selector);
    assert.equal(selector.raw, row.selector);
    const serialized = JSON.parse(JSON.stringify(selector));
    function semantic(value) {
      if (Array.isArray(value)) return value.map(semantic);
      if (!value || typeof value !== "object") return value;
      return Object.fromEntries(
        Object.entries(value)
          .filter(
            ([key]) =>
              key !== "raw" && !(key === "argument" && value.selectors),
          )
          .map(([key, item]) => [key, semantic(item)]),
      );
    }
    delete serialized.raw;
    const regenerated = parseSelector(printSelector(serialized));
    assert.deepEqual(semantic(regenerated), semantic(selector));
    assert.deepEqual(specificity(regenerated), specificity(selector));
    assert.deepEqual(selectorRequirements(selector), []);
    const body = `${row.selector}{--catalog-probe:${index};}`;
    const css = row.conditions ? `${row.conditions}{${body}}` : body;
    const plan = translateTheme(
      { id: "catalog", scheme: "auto", css },
      {
        foundation: false,
        document: row.class === "C" ? "legacy-content" : "shell",
      },
    );
    const [rule] = row.class === "C" ? plan.compat : plan.rules;
    assert.equal(rule.selector.raw, row.selector);
    assert.deepEqual(rule.specificity, specificity(selector));
    assert.equal(rule.operations[0].value, String(index));
    assert.equal(rule.conditions.length, row.conditions ? 1 : 0);
    assert.equal(plan.requirements.length, 0);
    assert.equal(row.class === "C" ? plan.rules.length : plan.compat.length, 0);
  });
}
