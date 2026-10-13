/*
 * "Use New Engine" (plan section 26, Phase 2: everything behind the flag).
 *
 * Off by default. The store key is the persistent switch; the
 * POWERMOVE_NEW_ENGINE environment variable (1 or 0) overrides it for
 * development and tests. The value is read once at launch: turning it on
 * or off takes effect after a relaunch, so a running app never mixes
 * engines. With the flag off, main starts no engine process, registers no
 * engine port, and the app behaves exactly as before.
 */
export const NEW_ENGINE_STORE_KEY = 'useNewEngine';
export const NEW_ENGINE_ENV = 'POWERMOVE_NEW_ENGINE';

export function newEngineEnabled(env: Record<string, string | undefined>, stored: unknown): boolean {
  const override = env[NEW_ENGINE_ENV];
  if (override === '1' || override === 'true') return true;
  if (override === '0' || override === 'false') return false;
  return stored === true;
}
