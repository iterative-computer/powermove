/** Engine definition snapshots of contributed effects, transitions and shader
 * layers, wrapped by the reference's own generators with its own preambles.
 * The native renderer validates and translates them; a bad one fails alone.
 */
import { GLSL_PRE, SHADER_HEADER } from "./frozen/reference/legacy/gl/shader-prelude";
import { extensionLayerFragment } from "./frozen/host/extension-layers";
import { toLegacyFx, toLegacyTransition } from "./frozen/host/glsl";
import type { Data } from "./types";

const utf8 = (text: string) => new TextEncoder().encode(text).length;
function wrapped(source: string, body: string) {
  const at = source.lastIndexOf(body);
  return {
    source,
    bodyLineOffset: at < 0 ? 0 : source.slice(0, at).split("\n").length - 1,
    bodyUtf16Len: body.length,
    bodyUtf8Len: utf8(body),
  };
}
function params(list: Data[] | undefined) {
  return (list ?? []).map((param) => {
    if (param.type === "color")
      return {
        type: "color",
        key: param.k,
        default: String(param.def ?? "#000000").slice(0, 7),
      };
    if (param.type === "toggle")
      return { type: "toggle", key: param.k, default: !!param.def };
    return {
      type: "number",
      key: param.k,
      default: Number.isFinite(param.def) ? param.def : 0,
      min: Number.isFinite(param.min) ? param.min : -1e12,
      max: Number.isFinite(param.max) ? param.max : 1e12,
      step: Number.isFinite(param.step) ? param.step : null,
    };
  });
}
const flags = (def: Data) => ({
  passes: def.passes ?? 1,
  keepOrig: !!def.keepOrig,
  backdrop: !!def.backdrop,
  rawShader: !!def.rawShader,
  viewportSafe: !!def.viewportSafe,
  viewportPadding: (def.viewportPadding ?? []).map(String),
  clamp: !!def.clamp,
});
const FORK_OF = (kernel: Data, owner: string) =>
  kernel.ownerOrigin?.(owner) ?? "local";

export function definitionSnapshots(kernel: Data): Data[] {
  const out: Data[] = [];
  for (const { id, ownerId, item } of kernel.effects.entries()) {
    const legacy = toLegacyFx(item, GLSL_PRE);
    out.push({
      id,
      kind: "effect",
      version: 0,
      origin: FORK_OF(kernel, ownerId),
      renderer: null,
      glsl: wrapped(legacy.frag, item.frag),
      params: params(item.params),
      flags: flags(item),
    });
  }
  for (const { id, ownerId, item } of kernel.transitions.entries()) {
    const legacy = toLegacyTransition(item, GLSL_PRE);
    out.push({
      id,
      kind: "transition",
      version: 0,
      origin: FORK_OF(kernel, ownerId),
      renderer: null,
      glsl: wrapped(legacy.frag, item.frag),
      params: params(item.params),
      flags: flags(item),
    });
  }
  for (const { id, ownerId, item } of kernel.layerTypes.entries()) {
    const fragment = item.renderer?.kind === "fragment";
    out.push({
      id,
      kind: "layer",
      version: item.version ?? 1,
      origin: FORK_OF(kernel, ownerId),
      renderer: item.renderer,
      glsl: fragment
        ? wrapped(
            extensionLayerFragment(item, SHADER_HEADER),
            item.renderer.fragment,
          )
        : null,
      params: params(item.params),
      flags: flags(item),
    });
  }
  return out;
}
