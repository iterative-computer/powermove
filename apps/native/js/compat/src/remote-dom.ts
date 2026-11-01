import type { Data } from "./types";
import type { RecorderCanvas } from "./canvas";

/** Realm-local connection only. Transport uses remote-dom/src/protocol.ts v1.
 * The page never receives functions or PM/service values. Fresh sync layout and
 * conditional browser-default replay are not supplied by that async protocol.
 */
export interface RemoteDOMBridge {
  connect(context: {
    realm: string;
    generation: number;
    /** Reserved for the native canvas integration; not a DOM wire operation. */
    createCanvas(width?: number, height?: number): RecorderCanvas;
    task<T>(run: () => T): T;
  }): { globals: Data; dispose(): void };
}
