import { ThemeError } from "./css.mjs";
import { copyData } from "./data.mjs";

/** Native-host port: replace({generation, snapshot}) atomically replaces the
 * previous appearance, including decorations/layout/painter inputs. It must
 * queue publication, never call JS from the UI thread or merge stale rules.
 * The Linux host in tests implements this same port with an in-memory snapshot.
 */
export class ThemePublication {
  constructor(host) {
    if (typeof host?.replace !== "function")
      throw new ThemeError(
        "invalid_host",
        "Theme host requires replace(publication)",
      );
    this.host = host;
    this.generation = 0;
  }
  request() {
    return ++this.generation;
  }
  commit(generation, snapshot) {
    if (generation !== this.generation) return false;
    // Copy on publication: retained JS references cannot change native state.
    this.host.replace({ generation, snapshot: copyData(snapshot) });
    return true;
  }
  invalidate() {
    this.generation++;
  }
}

export function cutoverRequirements(snapshot, capabilities) {
  const requirements = [...snapshot.blockers];
  const tokens = new Set(capabilities.tokens ?? []),
    properties = new Set(capabilities.properties ?? []);
  for (const addition of snapshot.additions)
    if (!tokens.has(addition.target))
      requirements.push({ code: "design_token", ...addition });
  for (const [surface, contexts] of Object.entries(snapshot.surfaces))
    for (const [context, result] of Object.entries(contexts)) {
      for (const operation of result.operations) {
        if (
          operation.domain !== "platform" &&
          !properties.has(operation.property)
        )
          requirements.push({
            code: "surface_property",
            surface,
            context,
            property: operation.property,
          });
      }
    }
  if (snapshot.motion.length && !capabilities.motion)
    requirements.push({ code: "native_motion" });
  if (snapshot.compat.length && !capabilities.compat)
    requirements.push({ code: "compat_content" });
  return requirements;
}
