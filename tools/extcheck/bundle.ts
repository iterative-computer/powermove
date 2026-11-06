// Build tool only: the disposable fixture scripts. The realm is embedded in the
// native runner and extensions go through the production native compiler.
import path from "node:path";
import { writeFile } from "node:fs/promises";
import { initial } from "./initial";
import { CLOCK_SOURCE } from "./clock-source.mjs";
const [output] = process.argv.slice(2);
await writeFile(path.join(output!, "clock.js"), CLOCK_SOURCE);
await writeFile(path.join(output!, "initial.json"), JSON.stringify(initial()));
const driver = await Bun.build({
  entrypoints: [path.resolve(import.meta.dir, "native-trace.ts")],
  target: "browser",
  format: "iife",
});
if (!driver.success) throw Error("Native trace driver build failed");
await writeFile(
  path.join(output!, "trace.js"),
  await driver.outputs[0]!.text(),
);
