import test from "node:test";
import assert from "node:assert/strict";
import { parseSelector, specificity, matches, SurfaceTree } from "../index.mjs";

const root = {
  id: "root",
  tag: "html",
  attributes: { "data-theme": "dark", "data-brand": "a b", lang: "en-US" },
  children: [
    {
      id: "dock",
      tag: "section",
      classes: ["dock"],
      children: [
        {
          id: "first",
          cssId: "title",
          tag: "button",
          classes: ["tab", "active"],
          attributes: { role: "tab", "data-value": "Alpha-end" },
          states: ["focus", "focus-visible", "hover"],
        },
        {
          id: "second",
          tag: "button",
          classes: ["tab"],
          attributes: { disabled: "", checked: "" },
        },
        { id: "third", tag: "div", classes: ["body"], text: "content" },
        { id: "fourth", tag: "button", classes: ["tab"] },
      ],
    },
    { id: "outside", tag: "aside", classes: ["dock"] },
  ],
};
const tree = new SurfaceTree(root);
function check(selector, id, expected = true, context = {}) {
  assert.equal(
    matches(parseSelector(selector), tree.nodes.get(id), tree, context),
    expected,
    `${selector} on ${id}`,
  );
}

test("tag/id/class/root, compound and descendant/child predicates", () => {
  check(':root[data-theme="dark"] > section.dock #title.tab', "first");
  check("section > .tab", "outside", false);
  check(":root", "root");
  check(":root", "dock", false);
  check("button#first", "first", false);
});
test("all six CSS attribute operators and case flags", () => {
  for (const selector of [
    "[role=tab]",
    "[data-value^=Alpha]",
    "[data-value$=end]",
    "[data-value*=ha-]",
    '[data-value="alpha-END" i]',
    '[data-value="Alpha-end" s]',
  ])
    check(selector, "first");
  check("[data-brand~=b][lang|=en]", "root");
  check('[data-value^=""]', "first", false);
  check("[missing]", "root", false);
  check("[data-value=alpha-end s]", "first", false);
});
test("adjacent and general sibling combinators never cross parent boundaries", () => {
  check("#title + button", "second");
  check("#title ~ button", "fourth");
  check("section + aside", "outside");
  check("section > button + aside", "outside", false);
});
test("logical wrappers and relative :has including sibling and nested branches", () => {
  check(":is(aside, .dock):not(:empty)", "dock");
  check(":where(section):has(> #title:hover)", "dock");
  check(".dock:has(> .body, + .missing)", "dock");
  check("button:has(+ button:disabled)", "first");
  check("button:has(~ .body)", "second");
  check("button:has(> .body)", "first", false);
  check(":global(:root[data-theme=dark]) :global(.tab)", "first");
});
test("structural selectors use one-based positions and correct node type", () => {
  check("button:first-child", "first");
  check("button:last-child", "fourth");
  check("button:nth-child(2n)", "second");
  check("button:nth-child(4n+2)", "second");
  check("button:nth-child(-n+2)", "fourth", false);
  check("button:nth-child(-n+2)", "second");
  check("button:nth-child(odd)", "first");
  check("button:nth-last-child(1)", "fourth");
  check("button:nth-of-type(3)", "fourth");
  check("div:only-of-type", "third");
  check(":nth-child(2 of .tab)", "second");
  check(":only-child", "first", false);
  check(":empty", "outside");
  check(":empty", "third", false);
});
test("native focus, focus-within, disabled, checked and enabled states", () => {
  check("section:focus-within", "dock");
  check(":focus-visible:hover", "first");
  check(":disabled:checked", "second");
  check(":enabled", "first");
  check(":enabled", "outside", false);
  check(":focus", "fourth", false);
});
test("decorations are separate from the originating surface", () => {
  check("button:not(:disabled)::before", "first", true, {
    pseudoElement: "before",
  });
  check("button:not(:disabled)::before", "second", false, {
    pseudoElement: "before",
  });
  check("button::before", "first", false);
  check("button", "first", false, { pseudoElement: "before" });
  check("button::highlight(search)", "first", true, {
    pseudoElement: "highlight",
    pseudoArgument: "search",
  });
});
test("specificity obeys :where zero and :is/:has/:not maximum, plus nth filter", () => {
  assert.deepEqual(specificity(parseSelector(":where(#id).tab")), [0, 1, 0]);
  assert.deepEqual(
    specificity(parseSelector(":is(#id, .tab):not(button):has(> .x)")),
    [1, 1, 1],
  );
  assert.deepEqual(
    specificity(parseSelector("button:nth-child(2 of #id, .tab)::before")),
    [1, 1, 2],
  );
  assert.deepEqual(specificity(parseSelector(":global(.tab)")), [0, 1, 0]);
});
test("escaped identifiers and strings round trip without executing source", () => {
  check("#ti\\74 le", "first");
  const selector = parseSelector('[data-value="Alpha\\2d end"]');
  assert.equal(matches(selector, tree.nodes.get("first"), tree), true);
});
test("unknown pseudo needs a host predicate; malformed selectors fail explicitly", () => {
  assert.throws(() => check("button:future", "first"), {
    code: "unsupported_selector",
  });
  check("button:future", "first", true, { pseudo: () => true });
  for (const text of [".a, .b", ".a >", ".a >> .b", "[x=", ":is()", ""])
    assert.throws(() => parseSelector(text));
  assert.throws(() => new SurfaceTree({ id: "x", children: [{ id: "x" }] }), {
    code: "invalid_tree",
  });
});
