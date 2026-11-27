import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import { clockHTML, clockHelper, CLOCK_ASSET } from "./prepare-reference.mjs";
import { CLOCK_SOURCE, CLOCK_FIXTURE } from "./clock-source.mjs";

test("shared prelude executes in a frozen-performance host before module evaluation", async () => {
  const context = vm.createContext({
    performance: Object.freeze({ now: () => -1 }),
    crypto: {},
  });
  vm.runInContext(CLOCK_SOURCE, context);
  assert.equal(
    vm.runInContext(
      "Date.now()-performance.timeOrigin-performance.now()",
      context,
    ),
    0,
  );
  assert.equal(
    vm.runInContext("typeof __extcheckClock.advance", context),
    "function",
  );
});

test("HTML prepends exactly the shared fixture before application modules", () => {
  const original =
    '<html><head></head><body><script type="module" src="./src/main.ts"></script></body></html>';
  const prepared = clockHTML(original);
  assert(
    prepared.indexOf('src="./host/extcheck-clock.js"') <
      prepared.indexOf('type="module"'),
  );
  assert(!prepared.includes("<script>"));
  assert(CLOCK_ASSET.includes(CLOCK_SOURCE));
  const context = vm.createContext({ performance: {}, crypto: {} });
  vm.runInContext(CLOCK_ASSET, context);
  assert.equal(
    vm.runInContext("__extcheckClock.fixture.sha256", context),
    CLOCK_FIXTURE.sha256,
  );
  assert(CLOCK_ASSET.includes(JSON.stringify(CLOCK_FIXTURE)));
  assert(
    clockHTML(original, "../host/extcheck-clock.js").includes(
      'src="../host/extcheck-clock.js"',
    ),
  );
  assert.throws(() => clockHTML(original, "https://invalid.example/clock.js"));
  assert.equal(
    prepared.replace(
      /\n<!-- extcheck-clock:start -->\n[\s\S]*?\n<!-- extcheck-clock:end -->/,
      "",
    ),
    original,
  );
  assert.throws(() => clockHTML("<head><head>"));
});

const helper = `async function boot(page, app) {
  await page.waitForURL(() => true);
  await page.ready();
  return { app, page };
}`;

test("startup clock pump is closed when the shipping helper completes", async () => {
  let cleared = 0;
  const context = vm.createContext({
    setInterval: () => 7,
    clearInterval: (id) => {
      assert.equal(id, 7);
      cleared++;
    },
  });
  vm.runInContext(clockHelper(helper), context);
  await context.boot({ waitForURL: async () => {}, ready: async () => {} }, {});
  assert.equal(cleared, 1);
});

test("startup clock pump closes on readiness failure and preserves the error", async () => {
  let cleared = 0;
  const error = Error("Synthetic startup failure");
  const context = vm.createContext({
    setInterval: () => 7,
    clearInterval: () => cleared++,
  });
  vm.runInContext(clockHelper(helper), context);
  await assert.rejects(
    context.boot(
      {
        waitForURL: async () => {},
        ready: async () => {
          throw error;
        },
      },
      {},
    ),
    (value) => value === error,
  );
  assert.equal(cleared, 1);
});

test("unsupported helper shapes fail before a reference source edit", () => {
  assert.throws(() => clockHelper("async function boot() {}"));
});
