// Add the comparison clock only to an owned disposable frozen reference.
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CLOCK_SOURCE, CLOCK_FIXTURE } from "./clock-source.mjs";

export function clockHTML(original) {
  const anchor = "<head>";
  if (original.split(anchor).length !== 2)
    throw Error("Unsupported reference HTML");
  const source = `${CLOCK_SOURCE}\nglobalThis.__extcheckClock.fixture=${JSON.stringify(CLOCK_FIXTURE)};`;
  return original.replace(
    anchor,
    `${anchor}\n<!-- extcheck-clock:start -->\n<script>${source.replace(/<\/script/gi, "<\\/script")}</script>\n<!-- extcheck-clock:end -->`,
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
      `  let clockPending = false;\n  let clockFailure;\n  const clockPump = setInterval(async () => {\n    if (clockPending) return;\n    clockPending = true;\n    try { const error = await page.evaluate(async () => {\n      try { await globalThis.__extcheckClock?.advance(16); return null; }\n      catch (error) { return String(error); }\n    }); clockFailure ??= error; }\n    catch {} finally { clockPending = false; }\n  }, 10);\n  try {\n${begin}`,
    )
    .replace(
      end,
      `${end}\n  } finally { clearInterval(clockPump);\n    while (clockPending) await new Promise(resolve => setTimeout(resolve, 1));\n    if (clockFailure) throw Error(clockFailure);\n  }`,
    );
}

export async function prepare(reference) {
  const revision = execFileSync("git", ["-C", reference, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();
  if (revision !== "07ed5fa17ee640659cb075f9cad1af2095011d60")
    throw Error("Wrong reference revision");
  const htmlPath = "apps/desktop/src/renderer/index.html";
  const original = execFileSync(
    "git",
    ["-C", reference, "show", `HEAD:${htmlPath}`],
    { encoding: "utf8" },
  );
  const current = await readFile(path.join(reference, htmlPath), "utf8");
  // Remove just our marked block when upgrading the fixture; refuse every
  // other source change. Never alter the installed app or a profile.
  const clean = current.replace(
    /\n<!-- extcheck-clock:start -->\n[\s\S]*?\n<!-- extcheck-clock:end -->/,
    "",
  );
  if (clean !== original) throw Error("Unrelated reference HTML edit");
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
  await writeFile(path.join(reference, htmlPath), clockHTML(original));
  await writeFile(helperPath, helper);
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await prepare(path.resolve(process.argv[2]));
  console.log(
    "Prepared frozen trace overlay and shared comparison clock; rebuild reference before capture",
  );
}
