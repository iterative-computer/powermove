/** Pure ECMAScript entrypoint: embeddable in rusty_v8, with no Node or DOM imports. */
export { CompatRealm, EditGesture } from "./realm";
export { CompatPanels, SECTIONS_PANEL } from "./panels";
export { connectControlState } from "./control-state";
export { definitionSnapshots } from "./definitions";
export { nativeTheme } from "./native-theme";
export type { RealmOptions } from "./realm";
export type { ExtensionOptions, ExtensionHandle } from "./extensions";
export { HostChannel, HostError } from "./channel";
export { DocumentMirror } from "./mirror";
export { createServicesRegistry } from "./services";
export { RecorderCanvas, NativeImage } from "./canvas";
export type { RecorderHost, SurfaceVersion, NativeSurface } from "./canvas";
export type * from "./remote-dom";
export type * from "./types";
export {
  normalizeChord,
  chordOfEvent,
  chordMatches,
  chordModifierCount,
} from "./keychord";

export * as powermove from "./frozen/reference/kernel/editor-helpers";
