/** Live PM facades, preserving the tag's argument/default/return adapters. */
import { HostError } from "./channel";
import { EditGesture } from "./gesture";
import * as space from "./frozen/reference/legacy/core/space-3d";
import * as factory from "./frozen/host/pm-facades";
import type { Data, GestureAPI } from "./types";

export function mergeDescriptors(...sources: (Data | undefined)[]): Data {
  const descriptors: PropertyDescriptorMap = {};
  for (const source of sources)
    if (source)
      Object.assign(descriptors, Object.getOwnPropertyDescriptors(source));
  return Object.defineProperties({}, descriptors);
}
/** Unlike an early-boot Electron PM, an absent native operation must not appear
 * to succeed. The proxy is private to the adapters; host.pm remains the exact PM.
 */
function requiredPM(PM: Data): Data {
  const methods = new Set(
    "ev evP active findProp allProps hasKeyAt setKey setKeyOn removeKey applyEaseTo wouldCycle resolveContent animVersion touch worldMatrix localMatrix transformParentMatrix mul groupAncestors transformRoots groupBounds groupSpan expandGroups normalizeStack moveToGroup firstSel selectLayers resolveSelectedKeys invalidate setTime play pause toggle step importFiles commandForAsset assetKind textCaretLayout renderFrameTo round lerp snapF tc parseTc hex2rgb rgb2hex".split(
      " ",
    ),
  );
  const subsystems: Record<string, string[]> = {
    GL: ["bounds", "pick", "init", "resize", "compileError"],
    MediaTiming: ["isTimed", "rate", "earliestStart", "startPatch", "endPatch"],
    Audio: ["drawWaveform"],
    assets: ["get", "add"],
    UIState: [
      "timelineVersion",
      "getLayerCollapsed",
      "setLayerCollapsed",
      "getGroupCollapsed",
      "setGroupCollapsed",
      "getKeyHandles",
      "setKeyHandles",
      "getFxOpen",
      "setFxOpen",
      "getReveal",
      "setReveal",
      "getShaderMeta",
      "setShaderMeta",
    ],
    Ease: ["nameOf"],
    Layout: ["refresh"],
  };
  const unavailable = (path: string): never => {
    throw new HostError("Unavailable", `PM.${path} capability is not attached`);
  };
  const missingFonts = new Proxy({}, { get: () => unavailable("Fonts") });
  return new Proxy(PM, {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver);
      if (key === "Fonts" && value === undefined) return missingFonts;
      if (typeof key === "string" && subsystems[key])
        return new Proxy(value ?? {}, {
          get(object, member) {
            const next = object[member];
            if (next === undefined && subsystems[key].includes(String(member)))
              return unavailable(`${key}.${String(member)}`);
            return typeof next === "function" ? next.bind(object) : next;
          },
          set(object, member, next) {
            if (!value) return unavailable(key);
            object[member] = next;
            return true;
          },
        });
      if (value === undefined && methods.has(String(key)))
        return unavailable(String(key));
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}
export function makePMFacades(PM: Data, gesture: GestureAPI): Data {
  const strict = requiredPM(PM);
  class BoundGesture extends EditGesture {
    constructor(binding: any) {
      super(gesture, binding);
    }
  }
  const call =
    (name: string) =>
    (...args: any[]) => {
      if (typeof PM[name] !== "function")
        throw new HostError(
          "Unavailable",
          `PM.${name} capability is not attached`,
        );
      return PM[name](...args);
    };
  const space3d = factory.makeSpace3D(strict);
  // N3: never turn an unavailable evaluator into a plausible identity matrix.
  for (const key of [
    "local3D",
    "parent3D",
    "world3D",
    "is3DLayer",
    "perspectiveAmount",
    "planeMatrix",
    "planeContains",
  ])
    space3d[key] = (...args: any[]) => (space as Data)[key](strict, ...args);
  return {
    anim: factory.makeAnim(strict),
    model: factory.makeModel(strict),
    selection: factory.makeSelection(strict),
    groups: factory.makeGroups(strict),
    transport: factory.makeTransport(strict),
    media: factory.makeMedia(strict),
    render: factory.makeRender(strict),
    uiState: factory.makeUIState(strict),
    util: factory.makeUtil(strict),
    ease: factory.makeEase(strict),
    space3d,
    dnd: factory.makeDnd(strict),
    workspace: factory.makeWorkspace(strict),
    project: {
      select: call("selectLayers"),
      time: () => Number(PM.time ?? 0),
      setTime: call("setTime"),
      play: call("play"),
      pause: call("pause"),
      playing: () => !!PM.playing,
    },
    ui: {
      ...Object.fromEntries(
        [
          "toast",
          "confirm",
          "openExternal",
          "menu",
          "modal",
          "icon",
          "drag",
          "closeMenus",
          "showLayerMenu",
          "showParentMenu",
          "beginParentPick",
          "openShaderEditor",
        ].map((name) => [name, call(name)]),
      ),
      gesture: BoundGesture,
    },
  };
}
