/* The only renderer module allowed to read the preload's global bridge. */
import type { PowermoveBridge } from '../../../shared/ipc';

let captured: PowermoveBridge | undefined;
let sealed = false;
const KEY = 'powermove';

/** Capture once at boot, then remove the raw global before extensions run. */
export function captureBridge(scope: object = globalThis): PowermoveBridge | undefined {
  if (sealed) return captured;
  const value = (scope as { powermove?: PowermoveBridge }).powermove;
  if (value && typeof value === 'object') captured = value;
  try {
    Reflect.deleteProperty(scope, KEY);
    Object.defineProperty(scope, KEY, { value: undefined, configurable: false, enumerable: false, writable: false });
  } catch (error) {
    console.error('[kernel] could not take the host bridge off window', error);
  }
  sealed = true;
  return captured;
}

/** The web host builds its bridge in this realm before extensions run. */
export function provideBridge(value: PowermoveBridge): void {
  if (captured) throw new Error('The host bridge is already installed.');
  captured = value;
}

/** After boot this reads only the captured bridge. The preboot path serves unit fixtures. */
export function bridge(): PowermoveBridge | undefined {
  return sealed ? captured : captured ?? (globalThis as { powermove?: PowermoveBridge }).powermove;
}

/** Test-only seam for fixtures that install a host bridge after module load. */
export function installBridgeForTests(value: PowermoveBridge | undefined): void {
  if (!import.meta.env?.VITEST) return;
  captured = value;
}

export function resetBridgeForTests(): void {
  if (!import.meta.env?.VITEST) return;
  captured = undefined;
  sealed = false;
}
