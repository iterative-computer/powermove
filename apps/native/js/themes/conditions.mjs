import { ThemeError, splitTop, topLevel } from "./css.mjs";

function booleanParts(text, word) {
  const boundaries = [];
  topLevel(text, (c, i) => {
    if (
      /\s/.test(c) &&
      text.slice(i).match(new RegExp(`^\\s+${word}\\s+`, "i"))
    ) {
      const length = text
        .slice(i)
        .match(new RegExp(`^\\s+${word}\\s+`, "i"))[0].length;
      if (!boundaries.some(([start, end]) => i >= start && i < end))
        boundaries.push([i, i + length]);
    }
  });
  if (!boundaries.length) return null;
  let offset = 0;
  const result = boundaries.map(([start, end]) => {
    const part = text.slice(offset, start);
    offset = end;
    return part;
  });
  return [...result, text.slice(offset)];
}

function query(text, dimensions, environment) {
  text = text.trim();
  const ors = booleanParts(text, "or");
  if (ors) return ors.some((part) => query(part, dimensions, environment));
  const ands = booleanParts(text, "and");
  if (ands) return ands.every((part) => query(part, dimensions, environment));
  if (/^not\s+/i.test(text))
    return !query(text.replace(/^not\s+/i, ""), dimensions, environment);
  if (/^(all|screen)$/i.test(text)) return true;
  if (/^print$/i.test(text)) return false;
  const match = /^\(\s*([\w-]+)\s*:\s*([^()]+)\s*\)$/.exec(text);
  if (!match)
    throw new ThemeError(
      "unsupported_condition",
      "Condition requires a host implementation",
      { text },
    );
  const feature = match[1],
    value = match[2].trim();
  const size = /^(min-|max-)?(width|height)$/.exec(feature);
  if (size) {
    const length = /^(\d+(?:\.\d+)?)(px|rem)$/.exec(value);
    if (!length)
      throw new ThemeError(
        "unsupported_condition",
        "Unsupported condition unit",
        { text },
      );
    const actual = dimensions?.[size[2]];
    if (!Number.isFinite(actual))
      throw new ThemeError(
        "missing_environment",
        "Condition dimensions are required",
        { feature },
      );
    const expected =
      Number(length[1]) *
      (length[2] === "rem" ? (environment.rootFontSize ?? 16) : 1);
    return size[1] === "min-"
      ? actual >= expected
      : size[1] === "max-"
        ? actual <= expected
        : actual === expected;
  }
  const features = {
    "prefers-color-scheme": environment.systemScheme,
    "prefers-reduced-motion": environment.reduceMotion
      ? "reduce"
      : "no-preference",
    "prefers-reduced-transparency": environment.reduceTransparency
      ? "reduce"
      : "no-preference",
    "prefers-contrast": environment.increaseContrast ? "more" : "no-preference",
    hover: environment.hover ?? "hover",
    pointer: environment.pointer ?? "fine",
  };
  if (!Object.hasOwn(features, feature))
    throw new ThemeError("unsupported_condition", "Unknown condition feature", {
      feature,
    });
  if (features[feature] === undefined)
    throw new ThemeError(
      "missing_environment",
      "Condition environment is required",
      { feature },
    );
  return value === features[feature];
}

export function conditionMatches(condition, node, tree, environment) {
  if (condition.kind === "media")
    return splitTop(condition.query).some((part) =>
      query(part, environment.viewport, environment),
    );
  if (condition.kind === "container") {
    const match = /^(?:([\w-]+)\s+)?(\(.*)$/s.exec(condition.query);
    if (!match)
      throw new ThemeError(
        "unsupported_condition",
        "Container query needs semantic dimensions",
      );
    for (let parent = tree.parent(node); parent; parent = tree.parent(parent)) {
      if (parent.container && (!match[1] || parent.container.name === match[1]))
        return query(match[2], parent.container, environment);
    }
    return false;
  }
  if (condition.kind === "supports" && environment.supports)
    return Boolean(environment.supports(condition.query));
  throw new ThemeError(
    "unsupported_condition",
    "Host condition implementation needed",
    { condition },
  );
}
