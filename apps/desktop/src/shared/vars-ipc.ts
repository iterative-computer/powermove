/*
 * Extension variables: the values a user enters for an extension that
 * declares `vars` in its manifest (apiVersion 2). Values live in the app
 * profile (`<userData>/env/<envKey>.env`), never in the extension folder, so
 * nothing the user types is ever published or shared with the source.
 *
 * Store 1.0 has per-extension values only. Global values, consent and
 * Settings › Variables are Store 1.1.
 *
 * Every channel here is renderer-reachable and therefore hostile input: main
 * validates each payload with zod and checks the sender (src/main/env/ipc.ts).
 */

export const VARS_IPC = {
  /** Which declared keys have a value. Never returns values. */
  status: 'vars:status',
  /** Store one value, then re-resolve the extension's health. */
  set: 'vars:set',
  /** Forget one value, then re-resolve the extension's health. */
  delete: 'vars:delete',
  /** Main shows a native dialog with the value masked and a Copy button handled in main. */
  reveal: 'vars:reveal',
  /**
   * Resolved values for delivery to `api.vars` just before activation.
   *
   * This is the ONE channel that hands plaintext to the renderer, by design:
   * extensions share one renderer realm, so any running extension can call it
   * for any id and read what it returns (store plan §2.2b). The boundary is
   * the profile, not the extension, until extension isolation lands.
   */
  values: 'vars:values',
  /**
   * Main-internal: asked inside `ext:remove` when the extension has stored
   * values ("Keep" / "Delete"). Not exposed through the preload.
   */
  removeAsk: 'vars:remove-ask'
} as const;

export interface VarsIdRequest {
  id: string;
}

export interface VarsKeyRequest {
  id: string;
  key: string;
}

export interface VarsSetRequest {
  id: string;
  key: string;
  /** An empty value removes the key. */
  value: string;
}

export type VarsResolutionStatus = 'ok' | 'needs-setup';

export interface VarsKeyStatus {
  key: string;
  label: string;
  hint?: string;
  secret: boolean;
  required: boolean;
  /** A usable value is stored. */
  set: boolean;
  /** A value is stored but this Mac can no longer decrypt it. */
  undecryptable: boolean;
}

export interface VarsStatus {
  keys: VarsKeyStatus[];
  status: VarsResolutionStatus;
}

/** Longest value accepted for one key. */
export const VARS_VALUE_MAX_CHARS = 8192;

export interface VarsBridge {
  status(req: VarsIdRequest): Promise<VarsStatus>;
  set(req: VarsSetRequest): Promise<VarsStatus>;
  delete(req: VarsKeyRequest): Promise<VarsStatus>;
  reveal(req: VarsKeyRequest): Promise<void>;
  /** See VARS_IPC.values: readable by every extension in the shared realm. */
  values(req: VarsIdRequest): Promise<Record<string, string>>;
}
