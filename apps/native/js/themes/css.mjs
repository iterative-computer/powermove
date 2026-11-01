// Pure ECMAScript: usable inside the trusted V8 realm. No browser or Node APIs.
export class ThemeError extends Error {
  constructor(code, message, detail = {}) {
    super(message);
    this.name = "ThemeError";
    this.code = code;
    this.detail = detail;
  }
}

export function unescapeCss(value) {
  return value.replace(/\\([0-9a-f]{1,6}\s?|[^\n\r\f])/gi, (_, escape) => {
    if (!/^[0-9a-f]/i.test(escape)) return escape;
    const point = parseInt(escape.trim(), 16);
    return String.fromCodePoint(
      point === 0 || point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)
        ? 0xfffd
        : point,
    );
  });
}

export function identifier(text, start = 0) {
  const match =
    /^(?:[\w\u0080-\uffff-]|\\(?:[0-9a-f]{1,6}\s?|[^\n\r\f]))+/i.exec(
      text.slice(start),
    );
  if (!match)
    throw new ThemeError("invalid_css", "Expected CSS identifier", {
      text,
      start,
    });
  return { value: unescapeCss(match[0]), end: start + match[0].length };
}

// Visit only top-level characters. Brackets, strings, escapes and comments are
// indivisible, including data URLs and commas in selectors/var() fallbacks.
export function topLevel(text, visit) {
  const stack = [];
  let quote = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\\") {
      const escape = /^\\(?:[0-9a-f]{1,6}\s?|[^\n\r\f])/i.exec(text.slice(i));
      if (!escape) throw new ThemeError("invalid_css", "Incomplete escape");
      i += escape[0].length - 1;
      continue;
    }
    if (quote) {
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'") {
      quote = c;
      continue;
    }
    if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      if (end < 0) throw new ThemeError("invalid_css", "Unclosed comment");
      i = end + 1;
      continue;
    }
    if (!stack.length) visit(c, i);
    if ("([{".includes(c)) stack.push(c);
    else if (")]}".includes(c)) {
      if (stack.pop() !== { ")": "(", "]": "[", "}": "{" }[c]) {
        throw new ThemeError("invalid_css", "Unbalanced CSS delimiter", {
          offset: i,
        });
      }
    }
  }
  if (quote || stack.length)
    throw new ThemeError("invalid_css", "Unclosed CSS string or block");
}

export function splitTop(text, delimiter = ",") {
  const parts = [];
  let start = 0;
  topLevel(text, (c, i) => {
    if (c === delimiter) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  });
  parts.push(text.slice(start));
  return parts;
}

export function closing(text, start) {
  const open = text[start],
    close = { "(": ")", "[": "]", "{": "}" }[open];
  if (!close) throw new ThemeError("invalid_css", "Expected opening delimiter");
  let end = -1;
  // A suffix bounded by its matching delimiter is validated independently.
  let depth = 0,
    quote = "";
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (c === "\\") {
      i++;
      continue;
    }
    if (quote) {
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'") {
      quote = c;
      continue;
    }
    if (text.startsWith("/*", i)) {
      const commentEnd = text.indexOf("*/", i + 2);
      if (commentEnd < 0) break;
      i = commentEnd + 1;
      continue;
    }
    if (c === open) depth++;
    if (c === close) {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end < 0)
    throw new ThemeError("invalid_css", "Unclosed CSS block", { start });
  return end;
}

export function stripComments(text) {
  let result = "",
    quote = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\\") {
      result += text.slice(i, i + 2);
      i++;
      continue;
    }
    if (quote) {
      result += c;
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'") {
      quote = c;
      result += c;
      continue;
    }
    if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      if (end < 0) throw new ThemeError("invalid_css", "Unclosed CSS comment");
      i = end + 1;
    } else result += c;
  }
  return result;
}

export function parseDeclarations(text) {
  return splitTop(stripComments(text), ";")
    .filter((part) => part.trim())
    .map((part) => {
      let colon = -1;
      topLevel(part, (c, i) => {
        if (c === ":" && colon < 0) colon = i;
      });
      if (colon < 0)
        throw new ThemeError("invalid_css", "Declaration has no colon", {
          declaration: part,
        });
      let property = unescapeCss(part.slice(0, colon).trim());
      if (
        !/^--[^\s:;{}]+$/.test(property) &&
        !/^-?[a-z][a-z-]*$/i.test(property)
      ) {
        throw new ThemeError("invalid_css", "Invalid property", { property });
      }
      if (!property.startsWith("--")) property = property.toLowerCase();
      const rawValue = part.slice(colon + 1).trim();
      let important = false;
      topLevel(rawValue, (c, i) => {
        if (c === "!" && /^!\s*important\s*$/i.test(rawValue.slice(i)))
          important = true;
      });
      const value = important
        ? rawValue.replace(/!\s*important\s*$/i, "").trim()
        : rawValue;
      return { property, value, important };
    });
}

export function parseCss(source) {
  const text = stripComments(source);
  const nodes = [];
  let start = 0;
  while (start < text.length) {
    while (/\s/.test(text[start] ?? "") && start < text.length) start++;
    if (start >= text.length) break;
    let boundary = -1,
      block = false;
    // Validate the entire suffix too: malformed CSS never becomes a partial
    // successful translation. Imported originals remain the caller's archive.
    topLevel(text.slice(start), (c, i) => {
      if (boundary < 0 && (c === "{" || c === ";")) {
        boundary = start + i;
        block = c === "{";
      }
    });
    if (boundary < 0)
      throw new ThemeError("invalid_css", "Rule has no block or terminator");
    const prelude = text.slice(start, boundary).trim();
    const end = block ? closing(text, boundary) : boundary;
    const body = block ? text.slice(boundary + 1, end) : null;
    if (prelude.startsWith("@")) {
      const name = identifier(prelude, 1);
      const node = {
        type: "at",
        name: name.value.toLowerCase(),
        prelude: prelude.slice(name.end).trim(),
        body,
      };
      if (
        body !== null &&
        [
          "media",
          "container",
          "supports",
          "layer",
          "scope",
          "starting-style",
        ].includes(node.name)
      )
        node.children = parseCss(body);
      nodes.push(node);
    } else {
      if (!block)
        throw new ThemeError("invalid_css", "Selector rule needs a block");
      nodes.push({
        type: "rule",
        selector: prelude,
        declarations: parseDeclarations(body),
      });
    }
    start = end + 1;
  }
  return nodes;
}
