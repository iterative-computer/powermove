/** Engine definition snapshots of contributed effects, transitions and shader
 * layers, wrapped by the reference's own generators with its own preambles.
 * The native renderer validates and translates them; a bad one fails alone.
 */
import {
  GLSL_PRE,
  SHADER_HEADER,
} from "./frozen/reference/legacy/gl/shader-prelude";
import { extensionLayerFragment } from "./frozen/host/extension-layers";
import { toLegacyFx, toLegacyTransition } from "./frozen/host/glsl";
import type { Data } from "./types";

/** The shipped app's transition preamble (`PM.TRANSITION_PRE`). */
const TRANSITION_PRE =
  "#version 300 es\nprecision highp float;\nin vec2 v_uv; in vec2 v_st;\nuniform sampler2D u_from, u_to;\nuniform float u_prog;\nuniform vec2 u_res, u_texel;\nuniform float u_time;\nout vec4 o;\nvec4 src(vec2 uv){ return texture(u_from, uv); }\nfloat luma(vec3 c){ return dot(c, vec3(.2126,.7152,.0722)); }\nvec3 hue2rgb(float h){ return clamp(abs(mod(h*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.); }\nvec3 rgb2hsv(vec3 c){\n  vec4 K=vec4(0.,-1./3.,2./3.,-1.); vec4 p=mix(vec4(c.bg,K.wz),vec4(c.gb,K.xy),step(c.b,c.g));\n  vec4 q=mix(vec4(p.xyw,c.r),vec4(c.r,p.yzx),step(p.x,c.r));\n  float d=q.x-min(q.w,q.y); float e=1e-10;\n  return vec3(abs(q.z+(q.w-q.y)/(6.*d+e)), d/(q.x+e), q.x);\n}\nvec3 hsv2rgb(vec3 c){ return c.z*mix(vec3(1.),clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.),c.y); }\nfloat hash(vec2 p){ return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }\nfloat noise(vec2 p){\n  vec2 i=floor(p), f=fract(p); f=f*f*(3.-2.*f);\n  return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);\n}\nfloat fbm(vec2 p){ float v=0.,a=.5; for(int i=0;i<5;i++){ v+=a*noise(p); p*=2.03; a*=.5; } return v; }\n";
/** The shipped app's `PM.FX[id]` and `PM.TRANSITIONS[id]`: a definition as
 * its legacy table held it, the shader complete with its preamble. */
export const legacyEffect = (item: Data): Data => toLegacyFx(item, GLSL_PRE);
export const legacyTransition = (item: Data): Data =>
  toLegacyTransition(item, TRANSITION_PRE);

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

/** One definition the generators cannot wrap is reported and left out; it never
 * takes the others' definitions with it. */
export function definitionSnapshots(
  kernel: Data,
  report: (id: string, error: unknown) => void = () => undefined,
): Data[] {
  const out: Data[] = [];
  const each = (registry: Data, build: (entry: Data) => Data) => {
    for (const entry of registry.entries()) {
      try {
        out.push({
          id: entry.id,
          origin: FORK_OF(kernel, entry.ownerId),
          ...build(entry),
        });
      } catch (error) {
        report(entry.id, error);
      }
    }
  };
  each(kernel.effects, ({ item }) => ({
    kind: "effect",
    version: 0,
    renderer: null,
    glsl: wrapped(toLegacyFx(item, GLSL_PRE).frag, item.frag),
    params: params(item.params),
    flags: flags(item),
  }));
  each(kernel.transitions, ({ item }) => ({
    kind: "transition",
    version: 0,
    renderer: null,
    glsl: wrapped(toLegacyTransition(item, GLSL_PRE).frag, item.frag),
    params: params(item.params),
    flags: flags(item),
  }));
  each(kernel.layerTypes, ({ item }) => ({
    kind: "layer",
    version: item.version ?? 1,
    renderer: item.renderer,
    glsl:
      item.renderer?.kind === "fragment"
        ? wrapped(
            extensionLayerFragment(item, SHADER_HEADER),
            item.renderer.fragment,
          )
        : null,
    params: params(item.params),
    flags: flags(item),
  }));
  return out;
}
