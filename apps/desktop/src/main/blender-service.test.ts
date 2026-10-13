import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, writeFile, chmod, rm, readdir } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import {
  BlenderWorker,
  validateBlenderJob,
  blenderCandidates,
  discoverBlender,
} from "./blender-service";
import { recipeSchema, render3DSchema } from "../shared/blender";
const roots: string[] = [],
  workers: BlenderWorker[] = [];
afterEach(async () => {
  workers.splice(0).forEach((w) => w.dispose());
  await Promise.all(
    roots.splice(0).map((p) => rm(p, { recursive: true, force: true })),
  );
});
const generation = (id = "model") => ({
  id,
  operation: "generate" as const,
  recipe: recipeSchema.parse({ kind: "rocket" }),
  assets: [],
});
const snapshot = () => ({
  width: 32,
  height: 32,
  settings: render3DSchema.parse({}),
  camera: {
    matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 5, 1],
    fov: 45,
    zoom: 1,
    near: 0.01,
    far: 100,
  },
  lights: [],
  objects: [
    {
      id: "mesh",
      meshes: [
        {
          positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
          uvs: [0, 0, 1, 0, 0, 1],
          material: {},
        },
      ],
    },
  ],
});
async function fake() {
  const dir = await mkdtemp(path.join(os.tmpdir(), "powermove-worker-test-"));
  roots.push(dir);
  const executable = path.join(dir, "fake-blender");
  await writeFile(
    executable,
    `#!/usr/bin/env python3
import sys,json,os,time
for line in sys.stdin:
    envelope=json.loads(line)
    req=json.load(open(envelope['path']))
    if req['operation']=='import':time.sleep(.3)
    json.dump({'files':[{'id':'source','name':'source.blend'}],'pid':os.getpid()},open(os.path.join(req['dir'],'manifest.json'),'w'))
    open(os.path.join(req['dir'],'source.blend'),'wb').write(b'worker-output')
    print('PM_BLENDER:'+json.dumps({'id':req['id'],'ok':True}),flush=True)
`,
  );
  await chmod(executable, 0o700);
  const worker = new BlenderWorker(executable, dir, 3000);
  workers.push(worker);
  return { worker, dir };
}
describe("bounded Blender protocol", () => {
  it("rejects path traversal, duplicate assets and executable recipes before starting", () => {
    expect(() =>
      validateBlenderJob({ ...generation(), id: "../model" }),
    ).toThrow();
    const asset = { id: "asset", name: "asset.blend", data: new Uint8Array() };
    expect(() =>
      validateBlenderJob({ ...generation(), assets: [asset, asset] }),
    ).toThrow();
    expect(() =>
      validateBlenderJob({ ...generation(), script: "import os" } as any),
    ).toThrow();
    expect(() =>
      recipeSchema.parse({
        kind: "rocket",
        modifiers: [
          { type: "subdivision", levels: 4 },
          { type: "array", count: 128, offset: [1, 0, 0] },
        ],
      }),
    ).toThrow(/budget/);
    expect(() =>
      recipeSchema.parse({
        kind: "mesh",
        mesh: { positions: [0, 0, 0, 1, 0, 0, 0, 1, 0], indices: [0, 1, 2, 0] },
      }),
    ).toThrow();
  });
  it("validates render cameras, geometry and referenced textures", () => {
    expect(
      validateBlenderJob({
        id: "render",
        operation: "render",
        assets: [],
        snapshot: snapshot(),
      }),
    ).toBeDefined();
    for (const bad of [
      { ...snapshot(), width: 9000 },
      {
        ...snapshot(),
        objects: [...snapshot().objects, ...snapshot().objects],
      },
      {
        ...snapshot(),
        lights: [
          {
            id: "sun",
            type: "sun",
            position: [Infinity, 0, 0],
            target: [0, 0, 0],
            color: "#ffffff",
            intensity: 1,
          },
        ],
      },
      {
        ...snapshot(),
        environment: {
          ambient: Infinity,
          ambientColor: "#ffffff",
          exposure: 1,
        },
      },
      { ...snapshot(), camera: { ...snapshot().camera, far: 0 } },
      {
        ...snapshot(),
        objects: [{ id: "mesh", meshes: [{ positions: [Infinity], uvs: [] }] }],
      },
    ])
      expect(() =>
        validateBlenderJob({
          id: "render",
          operation: "render",
          assets: [],
          snapshot: bad,
        }),
      ).toThrow();
    const textured = snapshot();
    textured.objects[0]!.meshes[0]!.material = {
      maps: { normal: "missing" },
    } as any;
    expect(() =>
      validateBlenderJob({
        id: "render",
        operation: "render",
        assets: [],
        snapshot: textured,
      }),
    ).toThrow(/texture/);
  });
  it("serializes jobs in one warm process and cleans every job directory", async () => {
    const { worker, dir } = await fake();
    const [a, b] = await Promise.all([
      worker.run(generation("first")),
      worker.run(generation("second")),
    ]);
    expect(a.manifest.pid).toBe(b.manifest.pid);
    expect(new TextDecoder().decode(a.files[0]!.data)).toBe("worker-output");
    const child = (await readdir(dir)).find((n) =>
      n.startsWith("powermove-blender-"),
    )!;
    expect(await readdir(path.join(dir, child))).toEqual(["worker.py"]);
  });
  it("cancels active and queued work, then permits a fresh worker", async () => {
    const { worker } = await fake();
    const first = worker.run({
      id: "slow",
      operation: "import",
      sourceAssetId: "asset",
      assets: [{ id: "asset", name: "asset.blend", data: new Uint8Array() }],
    });
    const second = worker.run(generation("queued"));
    const results = Promise.allSettled([first, second]);
    await new Promise((r) => setTimeout(r, 80));
    worker.cancel();
    expect((await results).map((r) => r.status)).toEqual([
      "rejected",
      "rejected",
    ]);
    expect((await worker.run(generation("retry"))).files).toHaveLength(1);
  });
  it("reports an unavailable executable without a shell fallback", async () => {
    expect(
      (await discoverBlender("/nonexistent/powermove-blender")).available,
    ).toBe(false);
  });
});

describe("Blender discovery", () => {
  const folders: Record<string, string[]> = {
    "/Applications": ["Blender.app", "Safari.app", "Blender 4.2.app"],
    "/Users/me/Applications": ["Blender-4.5.14.app", "Blender-5.0.1.app", "Blender-4.5.2.app", "BlenderKit.app", "Blender-4.5.14.zip"],
  };
  const list = async (directory: string) => folders[directory] ?? [];

  it("finds versioned bundles after the configured path, newest first per folder", async () => {
    expect(await blenderCandidates({
      env: { POWERMOVE_BLENDER_PATH: "/opt/blender/Blender", PATH: "/usr/local/bin:/opt/homebrew/bin" },
      platform: "darwin", home: "/Users/me", list,
    })).toEqual([
      "/opt/blender/Blender",
      "/Applications/Blender.app/Contents/MacOS/Blender",
      "/Applications/Blender 4.2.app/Contents/MacOS/Blender",
      "/Users/me/Applications/Blender-5.0.1.app/Contents/MacOS/Blender",
      "/Users/me/Applications/Blender-4.5.14.app/Contents/MacOS/Blender",
      "/Users/me/Applications/Blender-4.5.2.app/Contents/MacOS/Blender",
      "/usr/local/bin/blender",
      "/opt/homebrew/bin/blender",
    ]);
  });

  it("finds every Blender install folder on Windows", async () => {
    const windows = async (directory: string) =>
      directory === "C:\\Program Files\\Blender Foundation" ? ["Blender 4.5", "Blender 5.2", "Docs"] : [];
    expect(await blenderCandidates({ env: {}, platform: "win32", home: "C:\\Users\\me", list: windows })).toEqual([
      "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe",
      "C:\\Program Files\\Blender Foundation\\Blender 4.5\\blender.exe",
    ]);
  });

  it("keeps looking past an older Blender for a supported one", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "powermove-blender-discovery-"));
    try {
      const fake = async (name: string, version: string) => {
        const file = path.join(directory, name);
        await writeFile(file, `#!/bin/sh\necho "Blender ${version}"\n`);
        await chmod(file, 0o755);
        return file;
      };
      const old = await fake("old", "4.2.3 LTS");
      const current = await fake("current", "4.5.14 LTS");
      expect(await discoverBlender(old)).toMatchObject({ available: false, error: "Blender 4.5 or newer is required" });
      expect(await discoverBlender(undefined, { candidates: [path.join(directory, "missing"), old, current] }))
        .toMatchObject({ available: true, path: current, version: "Blender 4.5.14 LTS" });
      expect(await discoverBlender(undefined, { candidates: [old] }))
        .toMatchObject({ available: false, path: old, version: "Blender 4.2.3 LTS" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
