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

/**
 * The mode with "Use New Engine" on: the interactive engine follows the
 * mirror's patch stream (plan section 6), so the mirror is on at least in
 * the plain `on` mode; a test mode the profile chose stays.
 */
export function withNewEngine(mode: EngineDocMode, newEngine: boolean): EngineDocMode {
  return newEngine && !mode.evaluator ? parseMode('on') : mode;
}

/**
 * The development modes (dual answers, the auditor, the tracking Proxy) run
 * the legacy evaluator next to pm-wasm on purpose. They are for developers
 * and test builds only: a build chooses them at build time
 * (`VITE_POWERMOVE_NATIVE_EVALUATOR`), and a profile's stored override can
 * only turn the plain mirror on or off outside a development build.
 */
export function profileOverride(stored: string | null, development: boolean): EngineDocMode | null {
  if (stored == null) return null;
  const mode = parseMode(stored);
  if (development || !(mode.audit || mode.dual || mode.proxy)) return mode;
  return parseMode('on');
}

/** The renderer's mode: the stored override, else the build default. */
export function rendererMode(): EngineDocMode {
  let stored: string | null = null;
  try { stored = globalThis.localStorage?.getItem(STORAGE_KEY) ?? null; } catch { /* storage unavailable */ }
  const env = (import.meta as any).env ?? {};
  return profileOverride(stored, env.DEV === true || env.MODE === 'test') ?? parseMode(env.VITE_POWERMOVE_NATIVE_EVALUATOR ?? 'off');
}


/** Worker runtime environment is a developer override only. Production is
 * selected solely by Use New Engine, including the flag-off legacy path. */
export function workerMode(value: string | undefined, newEngine: boolean, development: boolean): EngineDocMode {
  return development ? withNewEngine(parseMode(value ?? 'on'), newEngine) : parseMode(newEngine ? 'on' : 'off');
}
