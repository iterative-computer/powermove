/** Keep a malformed resource local to its layer, without flooding playback logs. */
const reported = new WeakMap<object, Set<string>>();
export function reportLayerFailure(owner: object, layer: string, stage: string, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  const key = `${layer}:${stage}:${message}`;
  let seen = reported.get(owner);
  if (!seen) { seen = new Set(); reported.set(owner, seen); }
  if (seen.has(key)) return;
  if (seen.size >= 64) seen.clear();
  seen.add(key);
  console.warn(`[Studio] Skipping layer ${JSON.stringify(layer)} during ${stage}: ${message}`);
}
