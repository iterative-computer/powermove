/*
 * Main's host clock in the engine integration harnesses, made the way the
 * app makes it (`engineClock` in main/engine/index.ts): the engines'
 * `CLOCK_UPTIME_RAW` (contract 9 `HostMicros`) through pm-bridge.
 *
 * libuv's `hostNowUs` counts the time the Mac slept and the engines' clock
 * does not, so after a sleep the two differ by every second slept and an
 * engine never appears to share main's clock (phase2-gate.md, H1). A
 * harness therefore needs the bridge addon: falling back to libuv's clock
 * is the defect the harnesses check for.
 */
import { createRequire } from 'node:module';

import { engineClock, hostNowUs } from '../index';

export type MainHostClock = {
  now: () => number;
  /** libuv's clock minus the engines' clock when the harness started (µs):
   *  about the time the Mac slept since it booted. */
  libuvMinusEngineUs: number;
};

/** Loads the pm-bridge addon at `POWERMOVE_PM_BRIDGE`; throws without it. */
export function mainHostClock(file: string | undefined = process.env['POWERMOVE_PM_BRIDGE']): MainHostClock {
  if (!file) throw new Error("POWERMOVE_PM_BRIDGE must name the pm-bridge addon (main's host clock)");
  const addon = createRequire(__filename)(file) as { nowUs?: () => number };
  if (typeof addon.nowUs !== 'function') throw new Error(`the pm-bridge addon at ${file} does not export nowUs`);
  const now = engineClock(addon as { nowUs(): number });
  return { now, libuvMinusEngineUs: Math.round(hostNowUs() - now()) };
}
