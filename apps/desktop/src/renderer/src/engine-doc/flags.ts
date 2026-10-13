/*
 * The "native evaluator" flag (docs/rust-engine.md, section 26, Phase 1: "2D
 * synchronous queries and geometry-dependent edits switch to pm-wasm behind a
 * flag").
 *
 * Values, from a build-time default (`VITE_POWERMOVE_NATIVE_EVALUATOR`, or
 * `POWERMOVE_NATIVE_EVALUATOR` in the headless host) overridden by
 * `localStorage['powermove:native-evaluator']`:
 *
 * | Value | Mirror and queries | Auditor | Dual answers | Tracking Proxy |
 * |---|---|---|---|---|
 * | `off` (default) | no | no | no | no |
 * | `on` | yes | no | no | no |
 * | `audit` | yes | yes | yes | no |
 * | `audit-proxy` | yes | yes | yes | yes (violations reported) |
 * | `strict` | yes | yes | yes | yes (violations throw) |
 *
 * Every build keeps the barrier's `doc.write` / `doc.touched` and the
 * structural diff of `touch()`. The tracking Proxy is opt-in (`audit-proxy`,
 * `strict`) for development and tests: it is a second identity for every
 * document object, and code that compares a reference held across tasks with
 * a later read sees two objects (docs/engine/write-path.md lists the e2e tests
 * that do), so it never runs in the plain `on` mode.
 */
import type { ViolationMode } from './proxy';

export interface EngineDocMode {
  evaluator: boolean;
  audit: boolean;
  dual: boolean;
  proxy: false | ViolationMode;
  /** The flag's value as given. */
  name: string;
}

export const OFF: EngineDocMode = { evaluator: false, audit: false, dual: false, proxy: false, name: 'off' };

export const STORAGE_KEY = 'powermove:native-evaluator';

export function parseMode(value: unknown): EngineDocMode {
  const name = typeof value === 'string' ? value.trim().toLowerCase() : '';
  switch (name) {
    case 'on': case '1': case 'true':
      return { evaluator: true, audit: false, dual: false, proxy: false, name: 'on' };
    case 'audit':
      return { evaluator: true, audit: true, dual: true, proxy: false, name };
    case 'audit-proxy':
      return { evaluator: true, audit: true, dual: true, proxy: 'report', name };
    case 'strict':
      return { evaluator: true, audit: true, dual: true, proxy: 'throw', name };
    default:
      return OFF;
  }
}

/** The renderer's mode: the stored override, else the build default. */
export function rendererMode(): EngineDocMode {
  let stored: string | null = null;
  try { stored = globalThis.localStorage?.getItem(STORAGE_KEY) ?? null; } catch { /* storage unavailable */ }
  const env = (import.meta as any).env ?? {};
  const value = stored ?? env.VITE_POWERMOVE_NATIVE_EVALUATOR ?? 'off';
  return parseMode(value);
}
