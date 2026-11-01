/** Review-only transition-release handoff. All paths are owned by main. */
export const TR_IPC = 'transition:request';
export type ProxyOf = { fingerprint: string | null; sourcePath: string | null };
export type TRPayload = {
  key: string; size: number; type: string; revision?: string;
  /** IDB origin, object store, primary key and Blob path (logical JSON). */
  locator?: string; metadata?: unknown; proxyOf?: ProxyOf;
};
export type TRStatus = {
  state: 'pending' | 'copying' | 'paused' | 'complete';
  done: number; total: number; mediaComplete: boolean; localStorageComplete: boolean;
  secretsComplete: boolean; cutoverReady: boolean;
};
export type TRCommand =
  | { action: 'status' }
  | { action: 'secrets' }
  | { action: 'start'; inventory: string[] }
  | { action: 'localStorage'; values: Record<string, { raw: string; json?: unknown }> }
  | { action: 'reuse'; payload: TRPayload; hash: string; pass: string }
  | { action: 'begin'; payload: TRPayload; pass?: string }
  | { action: 'chunk'; token: string; offset: number; data: Uint8Array }
  | { action: 'finish'; token: string; hash: string }
  | { action: 'abort'; token: string }
  | { action: 'record'; pass: string; locator: string; metadata: unknown; payloads: string[] }
  | { action: 'complete'; pass: string; inventory: string[] };
export interface TransitionBridge {
  request(command: TRCommand): Promise<any>;
}
