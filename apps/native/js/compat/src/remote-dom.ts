import type { Data, Disposable } from "./types";
import type { RecorderCanvas } from "./canvas";

export type NodeId = string;
export type DOMOperation =
  | {
      op: "create";
      id: NodeId;
      kind: "element" | "text" | "comment";
      name: string;
      namespace?: string;
    }
  | { op: "insert"; parent: NodeId; child: NodeId; before: NodeId | null }
  | { op: "remove"; id: NodeId }
  | { op: "attribute"; id: NodeId; name: string; value: string | null }
  | { op: "property"; id: NodeId; name: string; value: any }
  | { op: "text"; id: NodeId; value: string }
  | { op: "canvas"; id: NodeId; surface: string };
export interface DOMBatch {
  realm: string;
  generation: number;
  sequence: number;
  operations: DOMOperation[];
}
export interface DOMEvent {
  realm: string;
  generation: number;
  sequence: number;
  target: NodeId;
  path: NodeId[];
  type: string;
  fields: Data;
  /** WebKit must hold cancellable defaults until this token is resolved. */
  defaultToken?: string;
}
export interface DOMEventResult {
  defaultPrevented: boolean;
  propagationStopped: boolean;
}
/** Implemented by the remote-DOM lane. No WebKit or layout simulation in this package. */
export interface RemoteDOMBridge {
  /** Supplies DOM globals backed by the bridge; all node identities belong to this realm. */
  connect(context: {
    realm: string;
    generation: number;
    createCanvas(width?: number, height?: number): RecorderCanvas;
    task<T>(run: () => T): T;
  }): { globals: Data; dispose(): void };
  submit(batch: DOMBatch): void;
  /** A synchronous layout/style read must flush all batches and name the acknowledged sequence. */
  read(node: NodeId, property: string, afterSequence: number): any;
  invoke(node: NodeId, method: string, args: any[], afterSequence: number): any;
  subscribe(listener: (event: DOMEvent) => DOMEventResult): Disposable;
  resolveDefault(token: string, result: DOMEventResult): void;
}
