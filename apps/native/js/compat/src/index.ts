/** Pure ECMAScript entrypoint: embeddable in rusty_v8, with no Node or DOM imports. */
export { CompatRealm, EditGesture } from "./realm";
export type { RealmOptions } from "./realm";
export { HostChannel, HostError } from "./channel";
export { DocumentMirror } from "./mirror";
export { createServicesRegistry } from "./services";
export { RecorderCanvas } from "./canvas";
export type { RecorderHost, SurfaceVersion, NativeSurface } from "./canvas";
export type * from "./remote-dom";
export type * from "./types";
export {
  normalizeChord,
  chordOfEvent,
  chordMatches,
  chordModifierCount,
} from "./keychord";

export * as powermove from "./frozen/kernel/editor-helpers";
