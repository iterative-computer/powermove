// Browser contract double only: no claim about WebKit decoding or painted pixels.
import { test, expect } from "bun:test";
import { createRequire } from "node:module";
const require = createRequire(
  new URL("../../../js/remote-dom/package.json", import.meta.url),
);
const { JSDOM } = require("jsdom");
const built = Bun.spawnSync([
  "bun",
  "build",
  new URL("page-entry.ts", import.meta.url).pathname,
  "--target=browser",
  "--format=iife",
]);
if (!built.success) throw Error(built.stderr.toString());
const shell = await Bun.file(
  new URL("../src/remote-dom.js", import.meta.url),
).text();
const session = { panel: "synthetic", generation: 1 };
const batch = {
  version: 1,
  session,
  seq: 1,
  caused_by: null,
  ops: [
    { op: "create", id: 6, kind: "element", name: "canvas", namespace: null },
    { op: "insert", parent: 4, id: 6, before: null },
  ],
};
async function page(events: unknown[]) {
  const dom = new JSDOM(
    "<!doctype html><html><head></head><body></body></html>",
    { runScripts: "outside-only" },
  );
  const calls: any[] = [],
    draws: any[] = [],
    resources: string[] = [];
  let polls = 0,
    decode: (() => void) | undefined;
  const decoded = new Promise<void>((resolve) => {
    decode = resolve;
  });
  dom.window.XMLHttpRequest = class {
    url = "";
    responseText = "";
    open(_method: string, url: string) {
      this.url = url;
    }
    send() {
      calls.push(JSON.parse(new URL(this.url).searchParams.get("q")!));
      this.responseText = JSON.stringify({ ok: true, value: null });
    }
  };
  dom.window.HTMLCanvasElement.prototype.getContext = function () {
    return {
      drawImage: (_bitmap: any, x: number, y: number) =>
        draws.push([this.width, this.height, x, y]),
    };
  };
  dom.window.createImageBitmap = async () => {
    await decoded;
    return {
      close() {
        calls.push({ kind: "bitmap-closed" });
      },
    };
  };
  dom.window.fetch = async (url: string) => {
    if (url.includes("canvas?")) {
      resources.push(url);
      return { ok: true, blob: async () => ({}) };
    }
    polls++;
    if (polls === 1)
      return {
        json: async () => ({ generation: 1, seq: events.length, events }),
      };
    return new Promise(() => {});
  };
  dom.window.eval(
    `const PM_HOST_CONFIG={base:'pmcompat://host/synthetic/',generation:1};${built.stdout.toString()}\n${shell}`,
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  return { dom, calls, draws, resources, decode: decode!, polls: () => polls };
}
test("native canvas resource is decoded before release or the next poll", async () => {
  const p = await page([
    { kind: "create", panel: 1 },
    { kind: "batch", panel: 1, batch },
    {
      kind: "canvas",
      panel: 1,
      node: 6,
      dom_sequence: 1,
      resource: 17,
      width: 2,
      height: 3,
    },
  ]);
  try {
    expect(p.resources).toEqual(["pmcompat://host/synthetic/canvas?q=17"]);
    expect(p.calls.some((c) => c.kind === "canvas_applied")).toBe(false);
    expect(p.polls()).toBe(1);
    p.decode();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(p.draws).toEqual([[2, 3, 0, 0]]);
    const release = p.calls.find((c) => c.kind === "canvas_applied");
    expect(release.payload).toEqual({ resource: 17 });
    expect(p.calls.findIndex((c) => c.kind === "bitmap-closed")).toBeLessThan(
      p.calls.indexOf(release),
    );
    expect(p.polls()).toBe(2);
  } finally {
    p.dom.window.close();
  }
});
test("a canvas preceding its DOM sequence fails visibly without a false acknowledgement", async () => {
  const p = await page([
    { kind: "create", panel: 1 },
    { kind: "batch", panel: 1, batch },
    {
      kind: "canvas",
      panel: 1,
      node: 6,
      dom_sequence: 2,
      resource: 17,
      width: 2,
      height: 3,
    },
  ]);
  try {
    expect(p.resources).toEqual([]);
    expect(p.calls.some((c) => c.kind === "failure")).toBe(true);
    expect(p.calls.some((c) => c.kind === "canvas_applied")).toBe(false);
    expect(p.draws).toEqual([]);
  } finally {
    p.dom.window.close();
  }
});
