/** Original synthetic native service double, used by both source and candidate. */
import type { Data } from "../../src/types";
export function probePM() {
  const trace: any[] = [],
    marker = { synthetic: true };
  const PM: Data = {
    time: 1,
    playing: false,
    quality: 1,
    perf: marker,
    previewResolution: "half",
    sel: { layers: ["a"], keys: [], chan: null },
    proj: { layers: [] },
    TL: { keySelectionActive: false },
    expressionErrors: new WeakMap(),
    CH: marker,
    BLENDS: marker,
    TYPE_META: marker,
    MASK_SHAPES: marker,
    Fonts: marker,
  };
  const define = (path: string, result: any) => {
    let object = PM;
    const parts = path.split(".");
    for (const part of parts.slice(0, -1)) object = object[part] ??= {};
    object[parts.at(-1)!] = function (...args: any[]) {
      trace.push([path, args]);
      return result;
    };
  };
  for (const path of [
    "ev",
    "evP",
    "findProp",
    "hasKeyAt",
    "setKey",
    "setKeyOn",
    "resolveContent",
    "P",
    "KF",
    "mkLayer",
    "mkMask",
    "mkProject",
    "cloneLayer",
    "normalizeFill",
    "layerDefinition",
    "L",
    "byName",
    "firstSel",
    "groupBounds",
    "moveToGroup",
    "commandForAsset",
    "assets.get",
    "GL.bounds",
    "GL.pick",
    "raster",
    "textCaretLayout",
    "renderFrameTo",
    "UIState.getKeyHandles",
    "UIState.setKeyHandles",
    "UIState.getReveal",
    "UIState.setReveal",
    "UIState.getShaderMeta",
    "UIState.setShaderMeta",
    "MediaTiming.startPatch",
    "MediaTiming.endPatch",
  ])
    define(path, marker);
  for (const path of [
    "active",
    "wouldCycle",
    "MediaTiming.isTimed",
    "Audio.drawWaveform",
    "GL.init",
    "GL.resize",
    "UIState.getLayerCollapsed",
    "UIState.setLayerCollapsed",
    "UIState.getGroupCollapsed",
    "UIState.setGroupCollapsed",
    "UIState.getFxOpen",
    "UIState.setFxOpen",
  ])
    define(path, true);
  for (const path of [
    "allProps",
    "worldMatrix",
    "localMatrix",
    "transformParentMatrix",
    "mul",
    "resolveSelectedKeys",
    "groupAncestors",
    "transformRoots",
    "expandGroups",
    "hex2rgb",
  ])
    define(path, [1, 2]);
  for (const path of [
    "animVersion",
    "MediaTiming.rate",
    "MediaTiming.earliestStart",
    "UIState.timelineVersion",
    "round",
    "clamp",
    "lerp",
    "snapF",
  ])
    define(path, 2);
  for (const path of [
    "tc",
    "uid",
    "rgb2hex",
    "Ease.nameOf",
    "Export.snapshot",
    "GL.compileError",
    "assetKind",
  ])
    define(path, "synthetic");
  for (const path of [
    "removeKey",
    "applyEaseTo",
    "touch",
    "selectLayers",
    "invalidate",
    "normalizeStack",
    "setTime",
    "play",
    "pause",
    "toggle",
    "step",
    "hist.selection",
    "bus.emit",
  ])
    define(path, undefined);
  define("curComp", PM.proj);
  define("groupSpan", { from: 0, dur: 2 });
  define("parseTc", null);
  define("importFiles", Promise.resolve(undefined));
  define("assets.add", Promise.resolve(marker));
  PM.Ease.PRESETS = marker;
  PM.GL.previewViewport = marker;
  PM.GL.gl = marker;
  return { PM, trace, marker };
}
