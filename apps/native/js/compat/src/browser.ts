/** Explicit global publication: Bun 1.3.14 has no --global-name build option. */
import * as compat from "./index";
Object.defineProperty(globalThis, "PowermoveCompat", {
  value: compat,
  configurable: true,
});
