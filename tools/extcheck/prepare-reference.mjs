// Add the comparison clock only to an owned disposable frozen reference.
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { REFERENCES, referenceRevision } from "./references.mjs";
import { CLOCK_SOURCE, CLOCK_FIXTURE } from "./clock-source.mjs";

export const CLOCK_ASSET = `/* extcheck-clock: owned fixture */\n${CLOCK_SOURCE}\nglobalThis.__extcheckClock.fixture=${JSON.stringify(CLOCK_FIXTURE)};`;

export function clockHTML(original, src = "./host/extcheck-clock.js") {
  const anchor = "<head>";
  if (original.split(anchor).length !== 2)
    throw Error("Unsupported reference HTML");
  if (!["./host/extcheck-clock.js", "../host/extcheck-clock.js"].includes(src))
    throw Error("Unsupported fixture asset URL");
  // External, classic script: executes before modules under the shipped CSP.
  // Never weaken script-src or inject an inline Store script.
  return original.replace(
    anchor,
    `${anchor}\n<!-- extcheck-clock:start -->\n<script src="${src}"></script>\n<!-- extcheck-clock:end -->`,
  );
}

export function clockHelper(original) {
  // Pump only during startup, while the shipping helper awaits PM readiness.
  // The driver owns all progression after launchApp returns. Boot progression
  // remains visible in the trace: it is never reset or stripped for stability.
  const begin = "  await page.waitForURL(";
  const end = "  return { app, page };";
  if (original.split(begin).length !== 2 || original.split(end).length !== 2)
    throw Error("Unsupported generated trace helper");
  return original
    .replace(
      begin,
      `  let clockPending = false;\n  let clockFailure;\n  const clockPump = setInterval(async () => {\n    if (clockPending) return;\n    clockPending = true;\n    try { for (const frame of page.frames()) {\n      const error = await frame.evaluate(async () => {\n        try { await globalThis.__extcheckClock?.advance(16); return null; }\n        catch (error) { return String(error); }\n      }); clockFailure ??= error;\n    } }\n    catch {} finally { clockPending = false; }\n  }, 10);\n  try {\n${begin}`,
    )
    .replace(
      end,
      `${end}\n  } finally { clearInterval(clockPump);\n    while (clockPending) await new Promise(resolve => setTimeout(resolve, 1));\n    if (clockFailure) throw Error(clockFailure);\n  }`,
    );
}

export async function prepare(reference, expected = REFERENCES.current) {
  referenceRevision(expected);
  const revision = execFileSync("git", ["-C", reference, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();
  if (revision !== expected) throw Error("Wrong reference revision");
  const htmlEdits = [];
  for (const [htmlPath, src] of [
    ["apps/desktop/src/renderer/index.html", "./host/extcheck-clock.js"],
    [
      "apps/desktop/src/renderer/sandbox/ext-sandbox.html",
      "../host/extcheck-clock.js",
    ],
  ]) {
    const original = execFileSync(
      "git",
      ["-C", reference, "show", `HEAD:${htmlPath}`],
      { encoding: "utf8" },
    );
    const current = await readFile(path.join(reference, htmlPath), "utf8");
    const clean = current.replace(
      /\n<!-- extcheck-clock:start -->\n[\s\S]*?\n<!-- extcheck-clock:end -->/,
      "",
    );
    if (clean !== original) throw Error("Unrelated reference HTML edit");
    htmlEdits.push([path.join(reference, htmlPath), clockHTML(original, src)]);
  }
  const assetPath = path.join(
    reference,
    "apps/desktop/src/renderer/public/host/extcheck-clock.js",
  );
  try {
    const existing = await readFile(assetPath, "utf8");
    if (!existing.startsWith("/* extcheck-clock: owned fixture */\n"))
      throw Error("Unrelated reference clock asset");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  execFileSync(
    "bun",
    [
      fileURLToPath(new URL("../trace-recorder/apply.ts", import.meta.url)),
      reference,
    ],
    { stdio: "pipe" },
  );
  const helperPath = path.join(
    reference,
    "apps/desktop/e2e/helpers/app.trace.mjs",
  );
  const helper = clockHelper(await readFile(helperPath, "utf8"));
  for (const [htmlPath, html] of htmlEdits) await writeFile(htmlPath, html);
  await writeFile(assetPath, CLOCK_ASSET);
  await writeFile(helperPath, helper);
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await prepare(path.resolve(process.argv[2]), process.argv[3]);
  console.log(
    "Prepared frozen trace overlay and shared comparison clock; rebuild reference before capture",
  );
}
