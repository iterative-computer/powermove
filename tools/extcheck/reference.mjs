// Node only. Frozen Electron + the existing trace-recorder overlay, never the owner's profile.
import { readFile, writeFile, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { prepareReferenceExtension } from "./reference-profile.mjs";
import { admitFixtureWorkspace } from "./fixture-aliases.mjs";
import { referenceRevision } from "./references.mjs";
import { CLOCK_FIXTURE } from "./clock-source.mjs";
import { withBootClock } from "./startup-clock.mjs";

delete process.env.ELECTRON_RUN_AS_NODE;
delete process.env.POWERMOVE_E2E_LIVE;
const input = [];
for await (const chunk of process.stdin) input.push(chunk);
const request = JSON.parse(Buffer.concat(input).toString());
const ref = path.resolve(process.argv[2]);
const out = request.output;
const profile = path.join(out, "profile");
let session;
const status = {
  status: "blocked",
  reason: "reference-not-ready",
  case: request.case,
  input_sha256: request.input_sha256,
  script_sha256: request.script_sha256,
};
try {
  const store = request.case.startsWith("store-") ? request.store : undefined;
  if (request.case.startsWith("store-") && !store)
    throw Error("Verified Store identity required");
  status.revision = execFileSync("git", ["-C", ref, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();
  if (status.revision !== referenceRevision(request.reference_revision))
    throw Error("Wrong reference");
  const hash = createHash("sha256");
  for (const file of ["core.ts", "overlay/runtime.ts", "overlay/transform.mjs"])
    hash.update(
      await readFile(new URL("../trace-recorder/" + file, import.meta.url)),
    );
  const fingerprint = hash.digest("hex");
  const manifest = JSON.parse(
    await readFile(path.join(request.extension, "manifest.json"), "utf8"),
  );
  await prepareReferenceExtension(
    profile,
    request.extension,
    manifest.id,
    store,
  );
  const { launchApp } = await import(
    pathToFileURL(path.join(ref, "apps/desktop/e2e/helpers/app.trace.mjs"))
  );
  session = await launchApp({
    userData: profile,
    env: {
      POWERMOVE_TRACE_RECORDER: "1",
      POWERMOVE_DISABLE_RUNTIME_UPDATES: "1",
      POWERMOVE_NEW_ENGINE: "0",
    },
  });
  await session.page.setViewportSize({ width: 1024, height: 768 });
  const clock = await session.page.evaluate(
    () => window.__extcheckClock?.fixture,
  );
  if (
    JSON.stringify(clock) !== JSON.stringify(CLOCK_FIXTURE) ||
    JSON.stringify(request.clock_fixture) !== JSON.stringify(CLOCK_FIXTURE)
  ) {
    status.reason = "reference-clock-fixture-unavailable";
    throw Error(
      "Prepare and rebuild the reference with the current shared clock",
    );
  }
  status.clock_fixture = clock;
  const installed = await session.page.evaluate(
    () => window.__PMTrace?.buildFingerprint,
  );
  if (installed !== fingerprint)
    throw Error("Missing/stale trace instrumentation");
  await withBootClock(session.page, async () => {
    await session.page.evaluate(async () => {
      await window.PM.Kernel.loader.builtinsReady;
    });
    // builtinsReady intentionally resolves before user-extension discovery/compilation.
    await session.page
      .waitForFunction(
        (id) => {
          const loader = window.PM.Kernel.loader;
          return (
            loader.activeIds().includes(id) ||
            loader
              .records()
              .some(
                (record) =>
                  record.id === id &&
                  ["error", "blocked", "needs-trust", "needs-setup"].includes(
                    record.health?.state,
                  ),
              )
          );
        },
        manifest.id,
        { timeout: 15000 },
      )
      .catch(() => {});
  });
  const active = await session.page.evaluate(
    (id) => window.PM.Kernel.loader.activeIds().includes(id),
    manifest.id,
  );
  if (store) {
    const trust = await session.page.evaluate(
      (id) =>
        window.PM.Kernel.loader.records().find((record) => record.id === id)
          ?.trust,
      manifest.id,
    );
    if (trust !== "store")
      throw Error("Store reference did not enter its isolated facade");
    const realms = session.page
      .frames()
      .filter((frame) => frame.url().startsWith("app://x-"));
    const fixtures = await Promise.all(
      realms.map((frame) =>
        frame.evaluate(() => globalThis.__extcheckClock?.fixture),
      ),
    );
    if (
      !fixtures.length ||
      fixtures.some(
        (value) => JSON.stringify(value) !== JSON.stringify(CLOCK_FIXTURE),
      )
    ) {
      status.reason = "reference-clock-fixture-unavailable";
      throw Error(
        "Store runtime/view does not have the shared comparison clock",
      );
    }
    status.store_clock_realms = fixtures.length;
    status.store_isolated = true;
  }
  if (!active) {
    console.error(
      await session.page.evaluate(
        (id) => window.PM.Kernel.loader.records().filter((r) => r.id === id),
        manifest.id,
      ),
    );
    status.reason = "reference-extension-not-active";
    throw Error("Extension not active on frozen reference");
  }
  status.reason = "reference-capture-failed";
  await session.page.evaluate(
    ({ request, revision, fingerprint, apiVersion }) => {
      const PM = window.PM;
      const project = PM.mkProject({
        name: "Synthetic extension check",
        w: 320,
        h: 180,
        fps: 30,
        dur: 2,
      });
      project.created = 0;
      PM.replaceProject(project);
      PM.hist.clear();
      PM.ProjectsScreen?.hide();
      window.__PMTrace.start({
        scenario: request.case,
        synthetic: true,
        role: request.case.startsWith("store-") ? "store" : "trusted",
        apiVersions: [apiVersion],
        reference: { revision },
        adapter: fingerprint,
        driver: "extcheck-electron",
        input: "scripted-interactions",
        eventScope: "synchronous-calls",
        input_sha256: request.input_sha256,
        script_sha256: request.script_sha256,
      });
    },
    {
      request,
      revision: status.revision,
      fingerprint,
      apiVersion: manifest.apiVersion ?? 1,
    },
  );
  await session.page.evaluate(admitFixtureWorkspace);
  await session.page.evaluate(() =>
    window.__PMTrace.point("driver.clock", window.__extcheckClock.snapshot()),
  );
  const interactions = [];
  for (let index = 0; index < request.script.steps.length; index++) {
    const step = request.script.steps[index];
    await session.page.evaluate(
      ({ index, step }) =>
        window.__PMTrace.point("driver.step", { index, step }),
      { index, step },
    );
    let result = null;
    if (["reload", "dispose", "seek", "panel", "command"].includes(step.op)) {
      result = await withBootClock(session.page, () =>
        session.page.evaluate(
          async ({ step, id }) => {
            const PM = window.PM,
              trace = window.__PMTrace;
            const api = PM.Kernel.api("synthetic-extcheck-driver");
            if (step.op === "reload") {
              await trace.call("lifecycle.reload", [id], () =>
                PM.Kernel.loader.reload(id),
              );
              return { active: PM.Kernel.loader.activeIds().includes(id) };
            }
            if (step.op === "dispose") {
              await trace.call("lifecycle.dispose", [id], () =>
                api.extensions.setEnabled(id, false),
              );
              await PM.Kernel.loader.whenIdle?.();
              return { active: PM.Kernel.loader.activeIds().includes(id) };
            }
            if (step.op === "seek") {
              PM.pause();
              trace.call("PM.setTime", [step.time], () =>
                PM.setTime(step.time),
              );
              return { time: PM.time };
            }
            if (step.op === "panel") {
              trace.call("panels.open", [step.id], () =>
                api.panels.open(step.id),
              );
              return { open: api.panels.isOpen(step.id) };
            }
            return trace.call(
              "commands.run",
              [step.id, ...(step.args ?? [])],
              () => api.commands.run(step.id, ...(step.args ?? [])),
            );
          },
          { step, id: manifest.id },
        ),
      );
    } else if (step.op === "advance") {
      await session.page.evaluate(
        (ms) => window.__extcheckClock.advance(ms),
        step.ms,
      );
    } else if (step.op === "click") {
      await session.page.locator(step.selector).click();
    } else if (step.op === "fill") {
      await session.page.locator(step.selector).fill(step.value);
    } else if (step.op === "key") {
      await session.page.keyboard.press(step.key);
    } else if (step.op === "assert") {
      const locator = session.page.locator(step.selector);
      const actual =
        step.property === "value"
          ? await locator.inputValue()
          : await locator.textContent();
      result = { pass: actual === step.equals, actual };
    } else if (step.op === "screenshot") {
      // Script-owned animation frames, followed by actual host paint.
      // No pixels, render-time values or trace rows are rewritten.
      await session.page.evaluate(async () => {
        await window.__extcheckClock.settle(2);
        await window.__extcheckClock.nativePaint();
      });
      await session.page.screenshot({
        path: path.join(out, step.label + ".png"),
        animations: "disabled",
      });
    } else {
      status.reason = "unsupported-script-step";
      throw Error("Unsupported step");
    }
    interactions.push({ op: step.op, result });
    await session.page.evaluate(
      ({ index, result }) => {
        window.__PMTrace.point("driver.result", { index, result });
        window.__PMTrace.checkpoint("step-" + index);
      },
      { index, result },
    );
  }
  const trace = await session.page.evaluate(() => window.__PMTrace.finish());
  await writeFile(path.join(out, "trace.jsonl"), trace);
  await writeFile(
    path.join(out, "interactions.json"),
    JSON.stringify(interactions),
  );
  Object.assign(status, { status: "captured", revision: status.revision });
  delete status.reason;
} catch (error) {
  // Source, strings, names and stacks are private. Parent sees fixed reason codes only.
  console.error(error);
} finally {
  if (session) {
    const child = session.app.process();
    let timer;
    try {
      await Promise.race([
        session.close(),
        new Promise((_, reject) => {
          timer = setTimeout(() => {
            child.kill("SIGKILL");
            reject(Error("Close timeout"));
          }, 5000);
        }),
      ]);
    } catch {
      child.kill("SIGKILL");
    } finally {
      clearTimeout(timer);
    }
  }
  await rm(profile, { recursive: true, force: true });
  await writeFile(path.join(out, "capture.json"), JSON.stringify(status));
}
