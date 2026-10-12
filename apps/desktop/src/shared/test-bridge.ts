/*
 * The e2e test seam for the host bridge.
 *
 * The renderer takes `window.powermove` off the page at boot
 * (renderer/src/kernel/capture-bridge.ts) so no code in the page reaches raw
 * IPC. Hidden test launches (POWERMOVE_BACKGROUND_TEST=1 with an isolated
 * profile, main/background-testing.ts) start editor windows with
 * TEST_BRIDGE_SWITCH; only then does the preload also publish the same bridge
 * as TEST_BRIDGE_GLOBAL, which the kernel leaves alone, so Playwright specs
 * can call host APIs (store flush, media, extensions, providers). A normal
 * launch never passes the switch, so the global never exists for users or
 * their extensions.
 */
export const TEST_BRIDGE_SWITCH = '--powermove-test-bridge';
export const TEST_BRIDGE_GLOBAL = '__powermoveTestBridge';

/** Publishes `bridge` under TEST_BRIDGE_GLOBAL when the process was started
 *  with TEST_BRIDGE_SWITCH. Returns whether it did. */
export function publishTestBridge<T>(
  argv: readonly string[],
  expose: (key: string, value: T) => void,
  bridge: T
): boolean {
  if (!argv.includes(TEST_BRIDGE_SWITCH)) return false;
  expose(TEST_BRIDGE_GLOBAL, bridge);
  return true;
}
