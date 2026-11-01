import {
  ThemeError,
  closing,
  identifier,
  splitTop,
  stripComments,
  unescapeCss,
} from "./css.mjs";

const logical = new Set(["is", "where", "not", "has", "global"]);
const structural = new Set([
  "root",
  "scope",
  "empty",
  "first-child",
  "last-child",
  "only-child",
  "first-of-type",
  "last-of-type",
  "only-of-type",
  "nth-child",
  "nth-last-child",
  "nth-of-type",
  "nth-last-of-type",
]);
const states = new Set([
  "hover",
  "active",
  "focus",
  "focus-visible",
  "focus-within",
  "disabled",
  "enabled",
  "checked",
  "selected",
  "open",
  "link",
  "visited",
  "target",
]);
const elements = new Set([
  "before",
  "after",
  "placeholder",
  "selection",
  "backdrop",
  "details-content",
  "highlight",
  "-webkit-scrollbar",
  "-webkit-scrollbar-thumb",
  "-webkit-scrollbar-track",
  "-webkit-search-cancel-button",
  "-webkit-slider-thumb",
  "-webkit-inner-spin-button",
  "-webkit-outer-spin-button",
  "-webkit-details-marker",
]);

export function parseSelector(source, relative = false) {
  const raw = source.trim();
  const text = stripComments(raw);
  let i = 0,
    pending = null;
  const parts = [];
  while (i < text.length) {
    let space = false;
    while (/\s/.test(text[i] ?? "") && i < text.length) {
      i++;
      space = true;
    }
    if (space && parts.length && pending === null) pending = " ";
    if (i >= text.length) break;
    if (">+~".includes(text[i])) {
      if (pending && pending !== " ")
        throw new ThemeError("invalid_selector", "Repeated combinator");
      if (!parts.length && !relative)
        throw new ThemeError(
          "invalid_selector",
          "Unexpected relative selector",
        );
      pending = text[i++];
      continue;
    }
    const tests = [];
    while (i < text.length && !/[\s>+~]/.test(text[i])) {
      const c = text[i++];
      if (c === "*") tests.push({ kind: "universal" });
      else if (c === "." || c === "#") {
        const ident = identifier(text, i);
        i = ident.end;
        tests.push({ kind: c === "." ? "class" : "id", value: ident.value });
      } else if (c === "[") {
        const end = closing(text, i - 1),
          content = text.slice(i, end).trim();
        i = end + 1;
        const name = identifier(content);
        const rest = content.slice(name.end).trim();
        const attr = {
          kind: "attribute",
          name: name.value,
          operator: null,
          value: null,
          flag: null,
        };
        if (rest) {
          const match =
            /^(~=|\|=|\^=|\$=|\*=|=)\s*("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|(?:\\.|[^\s])+?)(?:\s+([is]))?$/i.exec(
              rest,
            );
          if (!match)
            throw new ThemeError(
              "invalid_selector",
              "Invalid attribute selector",
              { content },
            );
          Object.assign(attr, {
            operator: match[1],
            value: unescapeCss(match[2].replace(/^(['"])(.*)\1$/s, "$2")),
            flag: match[3]?.toLowerCase() ?? null,
          });
        }
        tests.push(attr);
      } else if (c === ":") {
        const element = text[i] === ":";
        if (element) i++;
        const ident = identifier(text, i);
        i = ident.end;
        const name = ident.value.toLowerCase();
        let argument = null;
        if (text[i] === "(") {
          const end = closing(text, i);
          argument = text.slice(i + 1, end);
          i = end + 1;
        }
        const test = {
          kind:
            element ||
            ["before", "after", "first-line", "first-letter"].includes(name)
              ? "element"
              : "pseudo",
          name,
          argument,
        };
        if (logical.has(name) && test.kind === "pseudo") {
          if (argument === null)
            throw new ThemeError(
              "invalid_selector",
              "Logical selector needs arguments",
            );
          test.selectors = parseSelectorList(argument, name === "has");
        }
        tests.push(test);
      } else {
        const ident = identifier(text, i - 1);
        i = ident.end;
        tests.push({ kind: "tag", value: ident.value.toLowerCase() });
      }
    }
    if (!tests.length)
      throw new ThemeError("invalid_selector", "Empty compound");
    if (parts.length && !pending)
      throw new ThemeError("invalid_selector", "Missing combinator");
    parts.push({ combinator: pending, tests });
    pending = null;
  }
  if (!parts.length || (pending && pending !== " "))
    throw new ThemeError("invalid_selector", "Incomplete selector");
  return { raw, parts };
}

export function parseSelectorList(text, relative = false) {
  return splitTop(text).map((value) => parseSelector(value, relative));
}

function escapeIdentifier(value) {
  return [...value]
    .map((character, index) => {
      const point = character.codePointAt(0);
      if (point === 0) return "\ufffd";
      if (
        point < 32 ||
        point === 127 ||
        (/\d/.test(character) &&
          (index === 0 || (index === 1 && value[0] === "-")))
      )
        return `\\${point.toString(16)} `;
      if (/[\w-]/.test(character) || point >= 128)
        return value === "-" ? "\\-" : character;
      return `\\${character}`;
    })
    .join("");
}

function cssString(value) {
  return (
    '"' +
    [...value]
      .map((character) => {
        const point = character.codePointAt(0);
        if (character === '"' || character === "\\") return "\\" + character;
        if (point < 32 || point === 127) return `\\${point.toString(16)} `;
        return character;
      })
      .join("") +
    '"'
  );
}

// Reconstruct from the semantic AST, without consulting raw source. Raw stays
// separately available for archives/provenance; regeneration validates lowering.
export function printSelector(selector) {
  return selector.parts
    .map((part, index) => {
      const prefix = part.combinator
        ? part.combinator === " "
          ? " "
          : `${index ? " " : ""}${part.combinator} `
        : "";
      const compound = part.tests
        .map((test) => {
          if (test.kind === "universal") return "*";
          if (test.kind === "tag") return escapeIdentifier(test.value);
          if (test.kind === "id" || test.kind === "class")
            return (
              (test.kind === "id" ? "#" : ".") + escapeIdentifier(test.value)
            );
          if (test.kind === "attribute")
            return `[${escapeIdentifier(test.name)}${test.operator ? test.operator + cssString(test.value) + (test.flag ? " " + test.flag : "") : ""}]`;
          const argument = test.selectors
            ? test.selectors.map(printSelector).join(", ")
            : test.argument;
          return (
            (test.kind === "element" ? "::" : ":") +
            escapeIdentifier(test.name) +
            (argument === null ? "" : `(${argument})`)
          );
        })
        .join("");
      return prefix + compound;
    })
    .join("");
}
export const compareSpecificity = (a, b) =>
  a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
const add = (a, b) => a.map((value, i) => value + b[i]);
export function specificity(selector) {
  let result = [0, 0, 0];
  for (const part of selector.parts)
    for (const test of part.tests) {
      if (test.kind === "id") result[0]++;
      else if (test.kind === "class" || test.kind === "attribute") result[1]++;
      else if (test.kind === "tag" || test.kind === "element") result[2]++;
      else if (test.kind === "pseudo") {
        if (test.name === "where") continue;
        if (test.selectors)
          result = add(
            result,
            test.selectors.map(specificity).sort(compareSpecificity).at(-1),
          );
        else {
          result[1]++;
          const of = test.argument?.match(/\s+of\s+(.+)$/i);
          if (of && test.name.startsWith("nth-"))
            result = add(
              result,
              parseSelectorList(of[1])
                .map(specificity)
                .sort(compareSpecificity)
                .at(-1),
            );
        }
      }
    }
  return result;
}

export function selectorRequirements(selector) {
  const needed = [];
  for (const { tests } of selector.parts)
    for (const test of tests) {
      if (test.selectors)
        needed.push(...test.selectors.flatMap(selectorRequirements));
      else if (
        test.kind === "pseudo" &&
        !structural.has(test.name) &&
        !states.has(test.name)
      )
        needed.push(`pseudo:${test.name}`);
      else if (test.kind === "element" && !elements.has(test.name))
        needed.push(`decoration:${test.name}`);
    }
  return [...new Set(needed)];
}

// A Linux double and the native adapter use exactly this semantic tree shape.
// Stable ids are separate from CSS ids; no platform view or live DOM is needed.
export class SurfaceTree {
  constructor(root) {
    this.nodes = new Map();
    this.parents = new Map();
    this.root = root;
    const visit = (node, parent) => {
      if (!node.id || this.nodes.has(node.id))
        throw new ThemeError("invalid_tree", "Surface ids must be unique");
      this.nodes.set(node.id, node);
      this.parents.set(node.id, parent);
      for (const child of node.children ?? []) visit(child, node);
    };
    visit(root, null);
  }
  parent(node) {
    return this.parents.get(node.id);
  }
  siblings(node) {
    return this.parent(node)?.children ?? [this.root];
  }
  descendants(node) {
    return (node.children ?? []).flatMap((child) => [
      child,
      ...this.descendants(child),
    ]);
  }
}

function nth(argument, node, tree, name, context) {
  const [formula, filter] = argument.trim().split(/\s+of\s+/i);
  let siblings = tree.siblings(node);
  if (name.includes("of-type"))
    siblings = siblings.filter((sibling) => sibling.tag === node.tag);
  if (filter)
    siblings = siblings.filter((sibling) =>
      parseSelectorList(filter).some((selector) =>
        matches(selector, sibling, tree, context),
      ),
    );
  let position = siblings.indexOf(node) + 1;
  if (!position) return false;
  if (name.includes("last")) position = siblings.length - position + 1;
  const f = formula.replace(/\s+/g, "").toLowerCase();
  if (f === "odd") return position % 2 === 1;
  if (f === "even") return position % 2 === 0;
  if (/^[+-]?\d+$/.test(f)) return position === Number(f);
  const m = /^([+-]?\d*)n([+-]\d+)?$/.exec(f);
  if (!m)
    throw new ThemeError("unsupported_selector", "Invalid nth formula", {
      argument,
    });
  const a = m[1] === "" || m[1] === "+" ? 1 : m[1] === "-" ? -1 : Number(m[1]);
  const b = Number(m[2] ?? 0);
  return a === 0
    ? position === b
    : (position - b) / a >= 0 && Number.isInteger((position - b) / a);
}

function testSimple(test, node, tree, context) {
  const attrs = node.attributes ?? {},
    active = new Set(node.states ?? []);
  switch (test.kind) {
    case "universal":
      return true;
    case "tag":
      return node.tag === test.value;
    case "id":
      return (node.cssId ?? node.id) === test.value;
    case "class":
      return (node.classes ?? []).includes(test.value);
    case "attribute": {
      if (!Object.hasOwn(attrs, test.name)) return false;
      if (!test.operator) return true;
      let value = String(attrs[test.name]),
        expected = test.value;
      if (test.flag === "i") {
        value = value.toLowerCase();
        expected = expected.toLowerCase();
      }
      if (test.operator === "=") return value === expected;
      if (test.operator === "|=")
        return value === expected || value.startsWith(`${expected}-`);
      if (!expected) return false;
      if (test.operator === "~=") return value.split(/\s+/).includes(expected);
      if (test.operator === "^=") return value.startsWith(expected);
      if (test.operator === "$=") return value.endsWith(expected);
      return value.includes(expected);
    }
    case "element":
      return (
        context.pseudoElement === test.name &&
        (test.argument === null || context.pseudoArgument === test.argument)
      );
    case "pseudo": {
      const { name, argument } = test;
      if (name === "has") {
        return test.selectors.some((selector) => {
          const first = selector.parts[0].combinator ?? " ";
          const anchored = {
            parts: [
              {
                combinator: null,
                tests: [{ kind: "pseudo", name: "scope", argument: null }],
              },
              ...selector.parts.map((part, i) =>
                i ? part : { ...part, combinator: first },
              ),
            ],
          };
          return [...tree.nodes.values()].some((candidate) =>
            matches(anchored, candidate, tree, {
              ...context,
              scope: node,
              pseudoElement: null,
            }),
          );
        });
      }
      if (test.selectors) {
        const found = test.selectors.some((selector) =>
          matches(selector, node, tree, { ...context, pseudoElement: null }),
        );
        return name === "not" ? !found : found;
      }
      if (name === "root") return node === tree.root;
      if (name === "scope") return node === (context.scope ?? tree.root);
      if (name === "empty")
        return !(node.children?.length || node.text?.length);
      if (name === "focus-within")
        return (
          active.has("focus") ||
          active.has(name) ||
          tree
            .descendants(node)
            .some((child) => child.states?.includes("focus"))
        );
      if (name === "disabled")
        return active.has(name) || Object.hasOwn(attrs, "disabled");
      if (name === "enabled")
        return (
          !testSimple(
            { kind: "pseudo", name: "disabled" },
            node,
            tree,
            context,
          ) &&
          [
            "button",
            "input",
            "select",
            "textarea",
            "option",
            "fieldset",
          ].includes(node.tag)
        );
      if (name === "checked")
        return active.has(name) || Object.hasOwn(attrs, "checked");
      if (name.startsWith("nth-"))
        return nth(argument, node, tree, name, context);
      if (
        [
          "first-child",
          "last-child",
          "only-child",
          "first-of-type",
          "last-of-type",
          "only-of-type",
        ].includes(name)
      ) {
        const siblings = tree
          .siblings(node)
          .filter(
            (sibling) => !name.includes("of-type") || sibling.tag === node.tag,
          );
        return name.startsWith("first")
          ? siblings[0] === node
          : name.startsWith("last")
            ? siblings.at(-1) === node
            : siblings.length === 1;
      }
      if (states.has(name)) return active.has(name);
      if (context.pseudo) return context.pseudo(test, node);
      throw new ThemeError("unsupported_selector", "Host predicate needed", {
        name,
      });
    }
    default:
      throw new ThemeError("invalid_selector", "Unknown selector test");
  }
}

export function matches(selector, node, tree, context = {}) {
  const last = selector.parts.at(-1);
  const hasElement = last.tests.some((test) => test.kind === "element");
  if (Boolean(context.pseudoElement) !== hasElement) return false;
  function at(index, current) {
    if (
      !current ||
      !selector.parts[index].tests.every((test) =>
        testSimple(test, current, tree, context),
      )
    )
      return false;
    if (index === 0) return true;
    const combinator = selector.parts[index].combinator;
    if (combinator === ">") return at(index - 1, tree.parent(current));
    if (combinator === " ") {
      for (
        let parent = tree.parent(current);
        parent;
        parent = tree.parent(parent)
      )
        if (at(index - 1, parent)) return true;
      return false;
    }
    const siblings = tree.siblings(current),
      position = siblings.indexOf(current);
    if (combinator === "+") return at(index - 1, siblings[position - 1]);
    if (combinator === "~")
      return siblings
        .slice(0, position)
        .some((sibling) => at(index - 1, sibling));
    throw new ThemeError("unsupported_selector", "Unknown combinator", {
      combinator,
    });
  }
  return at(selector.parts.length - 1, node);
}
