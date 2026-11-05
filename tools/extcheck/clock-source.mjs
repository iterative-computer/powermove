// Build/driver only. Exactly these bytes are evaluated before either host's code.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
const source = readFileSync(
  new URL("./clock-fixture.mjs", import.meta.url),
  "utf8",
);
const anchor = "export function installClockFixture";
if (source.split(anchor).length !== 2)
  throw Error("Unexpected clock fixture module");
export const CLOCK_SEED = 0x504d5032;
// Function.toString differs between Node and Bun. Hash/evaluate the original
// source bytes, with only the module export removed from this owned fixture.
export const CLOCK_SOURCE = `(()=>{${source.replace(anchor, "function installClockFixture")}\ninstallClockFixture(globalThis,${CLOCK_SEED});})()`;
export const CLOCK_FIXTURE = Object.freeze({
  version: 1,
  seed: CLOCK_SEED,
  sha256: createHash("sha256").update(CLOCK_SOURCE).digest("hex"),
});
