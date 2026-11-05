import assert from "node:assert/strict";
import vm from "node:vm";
import { installClockFixture } from "./clock-fixture.mjs";
const create = () => {
  const context = vm.createContext({ performance: {}, crypto: {}, events: [] });
  return {
    context,
    clock: vm.runInContext(`(${installClockFixture.toString()})()`, context),
  };
};
const a = create(),
  b = create();
assert.equal(
  vm.runInContext(
    "[Date.now(),new Date().getTime(),Math.random(),crypto.randomUUID()].join()",
    a.context,
  ),
  vm.runInContext(
    "[Date.now(),new Date().getTime(),Math.random(),crypto.randomUUID()].join()",
    b.context,
  ),
);
vm.runInContext(
  `
  setTimeout(()=>{events.push(['timer',performance.now()]);queueMicrotask(()=>events.push(['microtask',performance.now()]))},8);
  const repeat=setInterval(()=>{events.push(['interval',performance.now()]);clearInterval(repeat)},8);
  requestAnimationFrame(t=>{events.push(['frame',t]);requestAnimationFrame(t=>events.push(['next-frame',t]))});
`,
  Object.assign(a.context, { queueMicrotask }),
);
await a.clock.advance(16);
assert.deepEqual(
  JSON.parse(vm.runInContext("JSON.stringify(events)", a.context)),
  [
    ["timer", 60008],
    ["microtask", 60008],
    ["interval", 60008],
    ["frame", 60016],
  ],
);
await a.clock.advance(16);
assert.equal(vm.runInContext("events.at(-1)[1]", a.context), 60032);
assert.equal(vm.runInContext("new Date(0).getTime()", a.context), 0);
assert.equal(
  vm.runInContext('Date.parse("1970-01-01T00:00:00Z")', a.context),
  0,
);
assert.equal(vm.runInContext("new Date() instanceof Date", a.context), true);
assert.equal(
  vm.runInContext(
    'new Date().constructor === Date && Date.name === "Date" && Date.length === 7',
    a.context,
  ),
  true,
);
vm.runInContext(
  "setTimeout(function(){events.push(this===globalThis)},0)",
  a.context,
);
await a.clock.advance(0);
assert.equal(vm.runInContext("events.at(-1)", a.context), true);
await assert.rejects(a.clock.advance(-1));
console.log(
  "PASS: comparison time, entropy, timer/microtask order, cancellation and frame progression",
);

const frozen = vm.createContext({
  performance: Object.freeze({ now: () => -1 }),
  crypto: {},
});
const frozenClock = vm.runInContext(
  `(${installClockFixture.toString()})()`,
  frozen,
);
assert.equal(vm.runInContext("performance.now()", frozen), 60000);
await frozenClock.advance(8);
assert.equal(vm.runInContext("performance.now()", frozen), 60008);
assert.throws(() =>
  vm.runInContext("crypto.getRandomValues(new Float32Array(1))", frozen),
);
assert.throws(() =>
  vm.runInContext(
    "crypto.getRandomValues(new DataView(new ArrayBuffer(8)))",
    frozen,
  ),
);
assert.throws(() =>
  vm.runInContext("crypto.getRandomValues(new Uint8Array(65537))", frozen),
);
const samples = Array.from({ length: 3 }, () => {
  const { context } = create();
  return vm.runInContext(
    "JSON.stringify([Array.from(crypto.getRandomValues(new Uint16Array(4))),crypto.randomUUID(),Math.random()])",
    context,
  );
});
assert.equal(new Set(samples).size, 1);
console.log(
  "PASS: frozen V8 performance and three identical entropy recordings",
);
