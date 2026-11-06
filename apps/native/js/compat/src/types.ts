/** Wire values preserve unknown project fields; no model migration occurs here. */
export type Data = Record<string, any>;
export type Path = (string | number)[];
export interface Patch {
  path: Path;
  exists: boolean;
  value?: any;
}
export interface Disposable {
  dispose(): void;
}
export interface ServicesAPI {
  register<T>(name: string, implementation: T): Disposable;
  get<T>(name: string): T | null;
}
export type EditCommand = Data & { type: string };
export interface EditResult {
  ok: boolean;
  message: string;
  data?: Data;
}
export interface EditMeta extends Data {
  label?: string;
  origin?: string;
  baseRevision?: number;
}
export interface EditAPI {
  apply(input: EditCommand | EditCommand[], meta?: EditMeta): EditResult;
  dispatch(input: EditCommand): EditResult;
  begin(label: string, meta?: EditMeta): void;
  commit(label?: string): EditResult;
  cancel(): boolean;
  mutate(label: string, action: unknown, meta?: EditMeta): EditResult;
}
export interface HistoryAPI {
  begin(label: string): string | null;
  commit(label?: string): boolean;
  cancel(): void;
  do<T>(label: string, fn: () => T): T;
  external(
    label: string,
    undo: unknown,
    redo: unknown,
    options?: Data,
  ): string | null;
  selection(before: Data, after: Data): string | null;
  undo(): boolean;
  redo(): boolean;
}
export interface GestureAPI {
  edit: EditAPI;
  history: HistoryAPI;
}
export interface Publication {
  revision: number;
  /** Whole document on a full sync; omitted when `ops` or nothing changed it. */
  document?: Data;
  ops?: Data[];
  /** Omitted unless they moved. */
  selection?: Data;
  workspace?: Data;
  facts: {
    historyBusy: boolean;
    canUndo: boolean;
    canRedo: boolean;
    editLive: boolean;
    label?: string | null;
  };
  events?: { name: string; value?: any }[];
  notice?: string;
}
export interface Request {
  realm: string;
  generation: number;
  txid: number;
  method: string;
  args: Data;
}
export interface Continuation {
  handle: string;
  phase: "undo" | "redo" | "cleanup";
  token: string;
}
export interface Reply {
  value?: any;
  publication?: Publication;
  continuation?: Continuation;
  /** Same-realm cleanup mailbox drained before returning this acknowledgement. */
  retired?: string[];
  error?: { code: string; message: string };
}
/** Sync functions run on the realm worker; the UI thread never calls/waits on JS. */
export interface KernelHost {
  call(request: Request): Reply;
  query(method: string, args: Data): any;
  /** Schedule the realm task boundary; must run on this realm's owning thread. */
  scheduleFlush(flush: () => void): void;
  reportError(error: unknown): void;
  pinGeneration(handle: string): void;
  unpinGeneration(handle: string): void;
}
