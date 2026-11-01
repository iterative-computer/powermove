// Extracted from v1.2.0 renderer/src/legacy/core/matte.ts; local type/duplicate-helper paths adapted.
export const MATTE_MODES = ["alpha", "alpha-inverted", "luma", "luma-inverted"];
export function validMatteSource(
  layers: any[],
  layer: any,
  id: string | null,
): boolean {
  const seen = new Set([layer.id]);
  let cursor = id;
  while (cursor) {
    if (seen.has(cursor)) return false;
    seen.add(cursor);
    const source = layers.find((l) => l.id === cursor);
    if (
      !source ||
      ["audio", "null", "adjustment", "group"].includes(source.type)
    )
      return false;
    cursor = source.matteSource;
  }
  return true;
}
