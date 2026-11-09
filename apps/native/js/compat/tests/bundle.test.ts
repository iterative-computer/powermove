import { expect, test } from "bun:test";
import { CompatRealm } from "../src";
import { FakeHost } from "./fake-host";
test("browser bundle executes with only ECMAScript globals and explicit host functions", async () => {
  const build = await Bun.build({
    entrypoints: [new URL("../src/index.ts", import.meta.url).pathname],
    target: "browser",
    format: "iife",
    minify: false,
  });
  expect(build.success).toBe(true);
  const source = await build.outputs[0].text();
  expect(source).not.toMatch(
    /(?:(?<![.\w])require\(\s*["']|from\s*["'](?:node:|bun:)|process\.|Bun\.|node_modules\/happy-dom)/,
  );
  // Build the same production entry as an IIFE export, then evaluate without a DOM.
  const result = await Bun.build({
    entrypoints: [new URL("../src/index.ts", import.meta.url).pathname],
    target: "browser",
    format: "esm",
  });
  const esm = await result.outputs[0].text();
  const module = new Function(
    esm.replace(
      /export\s*\{([^}]+)\};?\s*$/,
      (_match, names: string) =>
        "return {" + names.replace(/(\w+)\s+as\s+(\w+)/g, "$2:$1") + "};",
    ),
  )();
  const host = new FakeHost();
  const realm = new module.CompatRealm({
    id: "bundle",
    generation: 1,
    initial: host.publication(),
    host,
  });
  expect(realm.globals.document).toBeUndefined();
  realm.history.do("bundle edit", () => {
    realm.PM.proj.name = "from bundle";
  });
  expect(host.state.document.name).toBe("from bundle");
});
test("model factories preserve tagged authored defaults, unknown fields and refreshed clone ids", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "factory",
      generation: 1,
      host,
      initial: host.publication(),
      capabilities: {
        ease: { handles: () => ({ eo: [0.33, 0], ei: [0.67, 1] }) },
      },
    });
  const PM = realm.PM,
    p = PM.mkProject({ name: "Factory" });
  expect(p).toMatchObject({
    colorDepth: 16,
    w: 1920,
    h: 1080,
    fps: 30,
    dur: 10,
    name: "Factory",
    params: {},
  });
  PM.proj = p;
  expect(PM.KF(1, 2)).toMatchObject({
    t: 1,
    v: 2,
    eo: [0.33, 0],
    ei: [0.67, 1],
    hold: false,
  });
  const layer = PM.mkLayer("shape", {
    name: "Shape",
    d: { unknown: { authored: true } },
  });
  expect(layer.d).toMatchObject({
    w: 480,
    h: 480,
    radius: 24,
    unknown: { authored: true },
  });
  layer.p["position.x"].kf.push(PM.KF(1, 42));
  PM.proj.layers.push(layer);
  const cloned = PM.cloneLayer(layer);
  expect(cloned.id).not.toBe(layer.id);
  expect(cloned.p["position.x"].kf[0].i).not.toBe(
    layer.p["position.x"].kf[0].i,
  );
  expect(cloned.d.unknown).toEqual({ authored: true });
  expect(PM.deserialize(PM.serialize())).toEqual(PM.proj);
});
test("frozen powermove exports populate the shared compiler module table without a DOM", () => {
  const host = new FakeHost(),
    svelte = {};
  const realm = new CompatRealm({
    id: "modules",
    generation: 1,
    host,
    initial: host.publication(),
    runtimeModules: { svelte },
  });
  const runtime = realm.globals.__powermove_runtime;
  expect(runtime.svelte).toBe(svelte);
  expect(runtime.powermove.isProperty({ v: 1, kf: [], expr: null })).toBe(true);
  expect(runtime.powermove.expressionDiagnostic("1 + 2")).toBeNull();
  expect(runtime.powermove.MATTE_MODES).toContain("alpha");
  expect(typeof runtime.powermove.mountOverlayOnBody).toBe("function");
  expect(typeof runtime.powermove.temporalKeys).toBe("function");
});
