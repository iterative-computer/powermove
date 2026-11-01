// structuredClone is a Web API, not a V8 intrinsic. Keep the pipeline usable in
// a bare V8 realm. Inputs/publications are data, never functions or DOM objects.
export function copyData(value, ancestors = new Set()) {
  if (
    value === null ||
    value === undefined ||
    ["string", "boolean", "number"].includes(typeof value)
  )
    return value;
  if (typeof value !== "object" || ancestors.has(value))
    throw new TypeError("Theme data must be acyclic plain objects/arrays");
  const prototype = Object.getPrototypeOf(value);
  if (
    !Array.isArray(value) &&
    prototype !== Object.prototype &&
    prototype !== null
  )
    throw new TypeError("Theme data must contain only plain objects/arrays");
  ancestors.add(value);
  const result = Array.isArray(value)
    ? value.map((item) => copyData(item, ancestors))
    : Object.fromEntries(
        Object.entries(value).map(([key, item]) => [
          key,
          copyData(item, ancestors),
        ]),
      );
  ancestors.delete(value);
  return result;
}
