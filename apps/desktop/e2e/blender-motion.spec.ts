import path from "node:path";
import { readFile } from "node:fs/promises";
import { expect, test } from "./helpers/app";

test("Blender models, editable materials, regeneration and engine renders work in the native app", async ({
  session,
}) => {
  test.setTimeout(180000);
  await session.openEditor();
  const { page } = session;
  const status = await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.Kernel.api("blender-proof").scene3d.blenderStatus();
  });
  test.skip(
    !status.available,
    "Install Blender to run the real rendering integration",
  );
  const created = await page.evaluate(async () => {
    const PM = (window as any).PM,
      api = PM.Kernel.api("blender-proof");
    PM.setTime(0);
    PM.Edit.apply({
      type: "set_composition",
      patch: { width: 192, height: 128 },
    });
    const before = PM.hist.list().length,
      result = await api.scene3d.model({
        operation: "create_model",
        recipe: { kind: "rocket", parameters: { fins: 2 }, modifiers: [] },
      });
    if (!result.ok) throw new Error(result.message);
    const id = result.data.result.id;
    PM.selectLayers([id]);
    PM.invalidate();
    return {
      id,
      before,
      count: PM.hist.list().length,
      parts: PM.L(id).d.modeling.parts,
    };
  });
  expect(created.count).toBe(created.before + 1);
  expect(Object.keys(created.parts)).toHaveLength(5);
  await expect(
    page.getByRole("button", { name: "Regenerate", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Body length property", exact: true }),
  ).toBeVisible();
  const body = created.parts.body.layerId;
  const updated = await page.evaluate(
    async ({ id, body }) => {
      const PM = (window as any).PM,
        api = PM.Kernel.api("blender-proof");
      const selected = PM.L(body);
      const material = JSON.parse(
        JSON.stringify(selected.d.data.object.slots[0].material),
      );
      material.shader = api.scene3d.createMaterial("noise");
      const edited = api.scene3d.setMaterial(body, "slot0", material);
      if (!edited.ok) throw new Error(edited.message);
      PM.Edit.apply({
        type: "replace_keyframes",
        target: body,
        path: "rotation.y",
        keyframes: [
          { time: 0, value: 20 },
          { time: 1, value: 80 },
        ],
      });
      PM.Edit.apply({
        type: "set_property",
        target: id,
        path: "model.length",
        value: 3.8,
      });
      const shader = JSON.stringify(PM.L(body).d.data.object.slots[0].material),
        keys = JSON.stringify(PM.L(body).p["rotation.y"].kf),
        before = PM.hist.list().length;
      const result = await api.scene3d.model({
        operation: "regenerate_model",
        target: id,
      });
      if (!result.ok) throw new Error(result.message);
      return {
        sameId: PM.L(id).d.modeling.parts.body.layerId === body,
        shader:
          JSON.stringify(PM.L(body).d.data.object.slots[0].material) === shader,
        keys: JSON.stringify(PM.L(body).p["rotation.y"].kf) === keys,
        undo: PM.hist.list().length - before,
      };
    },
    { id: created.id, body },
  );
  expect(updated).toEqual({ sameId: true, shader: true, keys: true, undo: 1 });
  await page.evaluate((body) => {
    const PM = (window as any).PM;
    PM.selectLayers([body]);
    PM.invalidate();
  }, body);
  await expect(
    page.getByRole("group", { name: "Pattern scale property", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("radio", { name: "Solid", exact: true }),
  ).toBeVisible();
  for (const theme of ["dark", "light"]) {
    await page.evaluate((theme) => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      PM.invalidate();
    }, theme);
    await page.screenshot({ path: `/tmp/powermove-blender-${theme}.png` });
  }
  // Inspect the completed worker's own engine, rather than trusting UI settings.
  await session.app.evaluate(({ ipcMain }) => {
    const ipc = ipcMain as any,
      original = ipc._invokeHandlers.get("blender:run"),
      proof: any[] = [];
    (globalThis as any).__blenderEngineProof = proof;
    ipc.removeHandler("blender:run");
    ipc.handle("blender:run", async (event: any, job: any) => {
      const result = await original(event, job);
      if (job.operation === "render")
        proof.push({ requested: job.snapshot.settings.engine, actual: result.manifest.renderEngine });
      return result;
    });
  });
  for (const engine of ["eevee", "cycles"]) {
    const render = await page.evaluate(
      async ({ engine, body }) => {
        const PM = (window as any).PM,
          api = PM.Kernel.api("blender-proof");
        const result = api.scene3d.setRendering({
          enabled: true,
          engine,
          samples: 4,
          previewSamples: 4,
          previewScale: 1,
          device: "cpu",
        });
        if (!result.ok) throw new Error(result.message);
        // Only one object is rendered here, making the engine check economical.
        for (const layer of PM.proj.layers)
          if (layer.d?.data?.object && layer.id !== body) layer.on = false;
        PM.touch();
        // Public preview API exercises real IPC and PNG decoding; export captures
        // independently use the render-frame preparation path in the next check.
        api.scene3d.setPreviewMode("draft");
        const draft = [
          ...PM.GL.renderToPixels(0, 192, 128, {
            transparent: true,
            mblur: false,
            editorViewport: true,
          }),
        ];
        api.scene3d.setPreviewMode("rendered");
        await api.scene3d.renderFrame();
        const pixels = [
          ...PM.GL.renderToPixels(0, 192, 128, {
            transparent: true,
            mblur: false,
            editorViewport: true,
          }),
        ];
        const difference = pixels.reduce(
          (sum: number, v: number, i: number) => sum + Math.abs(v - draft[i]),
          0,
        );
        const snapshot = await PM.Export.snapshotAsync(0, 192);
        const exported = [
          ...PM.GL.renderToPixels(0, 192, 128, {
            transparent: true,
            mblur: false,
          }),
        ];
        return {
          difference,
          snapshot: snapshot.startsWith("data:image/jpeg"),
          exported: exported.some(
            (v: number, i: number) => i % 4 === 3 && v > 0,
          ),
          state: api.scene3d.previewState(),
          nonempty: pixels.some((v: number, i: number) => i % 4 === 3 && v > 0),
          transparent: pixels
            .filter((_: number, i: number) => i % 4 === 3)
            .some((v: number) => v === 0),
        };
      },
      { engine, body },
    );
    expect(render.state.error).toBeNull();
    expect(render.nonempty).toBe(true);
    expect(render.transparent).toBe(true);
    expect(render.difference).toBeGreaterThan(1000);
    expect(render.snapshot).toBe(true);
    expect(render.exported).toBe(true);
    const proof = await session.app.evaluate(() => (globalThis as any).__blenderEngineProof);
    const jobs = proof.filter((job: any) => job.requested === engine);
    expect(jobs.length).toBeGreaterThan(0);
    for (const job of jobs)
      if (engine === "cycles") expect(job.actual).toBe("CYCLES");
      else expect(job.actual).toMatch(/^BLENDER_EEVEE/);
  }
  const source = await page.evaluate(async () => {
    const PM = (window as any).PM,
      asset = Object.values(PM.proj.assets).find(
        (a: any) => a.format === "blend",
      ) as any,
      blob = await PM.MediaStore.get(asset);
    const api = PM.Kernel.api("blender-proof");
    const result = await api.scene3d.model({
      operation: "import_blend",
      sourceAssetId: asset.id,
      name: "Reimported",
    });
    if (!result.ok) throw new Error(result.message);
    return {
      bytes: blob.size,
      parts: Object.keys(PM.L(result.data.result.id).d.modeling.parts).length,
    };
  });
  expect(source.bytes).toBeGreaterThan(1000);
  expect(source.parts).toBe(5);
  const imported = await page.evaluate(async () => {
    const PM = (window as any).PM,
      api = PM.Kernel.api("blender-proof");
    api.scene3d.setRendering({ engine: "eevee", samples: 4 });
    await PM.Export.snapshotAsync(0, 192);
    return {
      missing: PM.UIState?.shaderMeta?.get?.(PM.L(PM.sel.layers[0]))?.missing,
      error: api.scene3d.previewState().error,
    };
  });
  expect(imported.error).toBeNull();
  const reloaded = await page.evaluate(async () => {
    const PM = (window as any).PM,
      serialized = JSON.parse(PM.serialize());
    PM.proj = PM.hydrateProject(serialized.proj);
    PM.ProjectIndex.invalidate();
    PM.assets.clear();
    await PM.assets.restoreProject(PM.proj);
    PM.touch();
    const api = PM.Kernel.api("blender-proof"),
      snapshot = await PM.Export.snapshotAsync(0, 192);
    return {
      engine: api.scene3d.getRendering().engine,
      models: PM.proj.layers.filter((l: any) => l.d?.modeling).length,
      source: !!Object.values(PM.proj.assets).find(
        (a: any) => a.format === "blend",
      ),
      image: snapshot.startsWith("data:image/jpeg"),
    };
  });
  expect(reloaded).toMatchObject({
    engine: "eevee",
    models: 2,
    source: true,
    image: true,
  });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test("an agent builds and animates a textured Blender model, captures the real camera and undoes its run", async ({
  session,
}) => {
  test.setTimeout(180000);
  await session.openEditor();
  const { page } = session;
  const status = await page.evaluate(() =>
    (window as any).PM.Kernel.api(
      "blender-agent-proof",
    ).scene3d.blenderStatus(),
  );
  test.skip(
    !status.available,
    "Install Blender for the real engine integration",
  );
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM,
      api = PM.Kernel.api("blender-agent-proof");
    PM.Edit.apply({
      type: "set_composition",
      patch: { width: 160, height: 120 },
    });
    PM.setTime(0);
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 16;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#FF4400";
    ctx.fillRect(0, 0, 16, 16);
    ctx.fillStyle = "#FFFFFF";
    ctx.fillRect(0, 0, 8, 8);
    ctx.fillRect(8, 8, 8, 8);
    const blob = await new Promise<Blob>((r) => canvas.toBlob((b) => r(b!)));
    const asset = await PM.assets.add(
      new File([blob], "checks.png", { type: "image/png" }),
      { silent: true },
    );
    const before = JSON.stringify(PM.proj.layers),
      baseRevision = PM.proj.revision;
    const call = async (tool: string, args: any = {}) => {
      const result = await PM.AgentHarness.test.handleLiveAgentTool({
        runId: "blender-agent-e2e",
        callId: crypto.randomUUID(),
        tool,
        arguments: args,
        baseRevision,
      });
      if (!result.ok) throw new Error(JSON.stringify(result));
      return result;
    };
    await call("get_project_state");
    const created = await call("edit_3d", {
      operation: "create_model",
      name: "Vase",
      recipe: {
        kind: "lathe",
        parameters: { segments: 32 },
        profile: [
          [0, -1],
          [0.8, -1],
          [0.8, -0.8],
          [0.45, 0],
          [0.6, 0.7],
          [0.5, 1],
        ],
        modifiers: [{ type: "bevel", width: 0.02, segments: 2 }],
      },
    });
    const group = JSON.parse(created.content[0].text).layer.id,
      object = PM.L(PM.L(group).d.modeling.parts.model.layerId);
    const shader = {
      id: "agent_paint",
      name: "Agent paint",
      graph: {
        nodes: [
          {
            id: "surface",
            type: "ShaderNodeBsdfPrincipled",
            inputs: { "Base Color": "#FFFFFF", Metallic: 0.2 },
          },
          { id: "output", type: "ShaderNodeOutputMaterial" },
        ],
        links: [
          { from: "surface", output: "BSDF", to: "output", input: "Surface" },
        ],
      },
      inputs: [
        {
          id: "roughness",
          label: "Roughness",
          kind: "number",
          node: "surface",
          socket: "Roughness",
          min: 0,
          max: 1,
        },
      ],
      p: { roughness: { v: 0.2, kf: [], expr: null } },
    };
    await call("get_3d_scene");
    await call("edit_3d", {
      operation: "update_object",
      target: object.id,
      patch: {
        slots: [
          {
            ...object.d.data.object.slots[0],
            material: {
              p: { color: "#FFFFFF" },
              maps: { color: asset.id },
              shader,
            },
          },
        ],
      },
    });
    await call("apply_commands", {
      commands: [
        {
          type: "replace_keyframes",
          target: object.id,
          path: "rotation.y",
          keyframes: [
            { time: 0, value: 0 },
            { time: 1, value: 60 },
          ],
        },
        {
          type: "replace_keyframes",
          target: object.id,
          path: "slots.slot0.shader.roughness",
          keyframes: [
            { time: 0, value: 0.1 },
            { time: 1, value: 0.8 },
          ],
        },
      ],
    });
    await call("edit_3d", {
      operation: "add_light",
      light: {
        id: "studio_key",
        type: "sun",
        p: { x: 3, y: 5, z: 4, intensity: 3 },
      },
    });
    await call("edit_3d",{operation:"add_light",light:{id:"studio_fill",type:"point",p:{x:-2,y:3,z:2,intensity:20,distance:12}}});
    await call("edit_3d", { operation: "add_camera" });
    await call("edit_3d", {
      operation: "set_rendering",
      settings: {
        enabled: true,
        engine: "eevee",
        samples: 4,
        previewSamples: 4,
        device: "cpu",
      },
    });
    const frames = await call("render_frames", { times: [0, 1], width: 160 });
    const images = frames.content.filter((c: any) => c.type === "image");
    const cameraBefore = await PM.Export.snapshotAsync(0, 160);
    api.scene3d.frame(false);
    const cameraAfter = await PM.Export.snapshotAsync(0, 160);
    const state = JSON.parse((await call("get_3d_scene")).content[0].text);
    await call("__finish_run", { commit: true });
    PM.hist.undo();
    return {
      frames: images.length,
      animated: images[0].data !== images[1].data,
      editor: api.scene3d.getView(),
      independent: cameraBefore === cameraAfter,
      restored: JSON.stringify(PM.proj.layers) === before,
      paths: state.channels.map((c: any) => c.path),
      model: state.modelGroups[0].model.recipe.kind,
    };
  });
  expect(result).toMatchObject({
    frames: 2,
    animated: true,
    editor: "editor",
    independent: true,
    restored: true,
    model: "lathe",
  });
  expect(result.paths).toContain("slots.slot0.shader.roughness");
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test("Blender motion blur exports transparent groups and nested compositions without falling back to Draft", async ({
  session,
}) => {
  test.setTimeout(180000);
  await session.openEditor();
  const { page } = session;
  const available = await page.evaluate(() =>
    (window as any).PM.Kernel.api("blur-proof").scene3d.blenderStatus(),
  );
  test.skip(!available.available, "Install Blender for the real export check");
  await page.evaluate(() => {
    const PM = (window as any).PM,
      api = PM.Kernel.api("blur-proof");
    PM.Edit.apply({
      type: "set_composition",
      patch: { width: 128, height: 96, fps: 12, shutter: 1 },
    });
    const added = api.scene3d.edit({
      operation: "add_object",
      object: {
        source: { primitive: "box" },
        material: { p: { color: "#FF6622" } },
      },
    });
    if (!added.ok) throw Error(added.message);
    const id = added.data.result.id;
    PM.Edit.apply({
      type: "group_layers",
      targets: [id],
      name: "Animated model",
    });
    const group = PM.proj.layers.find((l: any) => l.type === "group");
    const edit = PM.Edit.apply([
      { type: "set_layer", target: group.id, patch: { motionBlur: true } },
      {
        type: "replace_keyframes",
        target: group.id,
        path: "position.x",
        keyframes: [
          { time: 0, value: -3 },
          { time: 1, value: 3 },
        ],
      },
    ]);
    if (!edit.ok) throw Error(edit.message);
    api.scene3d.setRendering({
      enabled: true,
      engine: "eevee",
      samples: 1,
      previewSamples: 1,
      previewScale: 1,
    });
    PM.setTime(0.5);
  });
  const files: Buffer[] = [];
  for (const [name, alpha, mblur] of [
    ["sharp", true, false],
    ["blur", true, true],
    ["opaque", false, true],
  ] as const) {
    const file = path.join(session.userData, `${name}.png`);
    await session.app.evaluate(({ dialog }, file) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
    }, file);
    expect(
      await page.evaluate(
        ({ alpha, mblur }) =>
          (window as any).PM.Export.run({
            format: "still",
            scale: 1,
            alpha,
            mblur,
            audio: false,
          }),
        { alpha, mblur },
      ),
    ).toEqual({ cancelled: false });
    const bytes = await readFile(file);
    expect(bytes.subarray(1, 4).toString()).toBe("PNG");
    expect(bytes.readUInt32BE(16)).toBe(128);
    files.push(bytes);
  }
  expect(files[0]!.equals(files[1]!)).toBe(false);
  const nested = await page.evaluate(async () => {
    const PM = (window as any).PM,
      api = PM.Kernel.api("blur-proof"),
      group = PM.proj.layers.find((l: any) => l.type === "group");
    const id = PM.Comps.precompose([group.id], { name: "Nested model" });
    PM.touch();
    api.scene3d.setPreviewMode("rendered");
    await api.scene3d.renderFrame();
    const preview = PM.GL.renderToPixels(0.5, 128, 96, {
      transparent: true,
      mblur: false,
      editorViewport: true,
    });
    return {
      engine: PM.proj.comps[id].render3d.engine,
      visible: [...preview].some(
        (v: number, i: number) => i % 4 === 3 && v > 0,
      ),
      error: api.scene3d.previewState().error,
    };
  });
  expect(nested).toEqual({ engine: "eevee", visible: true, error: null });
  const file = path.join(session.userData, "nested.png");
  await session.app.evaluate(({ dialog }, file) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
  }, file);
  expect(
    await page.evaluate(() =>
      (window as any).PM.Export.run({
        format: "still",
        scale: 1,
        alpha: true,
        mblur: true,
        audio: false,
      }),
    ),
  ).toEqual({ cancelled: false });
  expect((await readFile(file)).length).toBeGreaterThan(1000);
  expect(
    await page.evaluate(() => {
      try {
        (window as any).PM.GL.renderToPixels(0.75, 128, 96, {
          transparent: true,
          mblur: false,
        });
        return false;
      } catch (error) {
        return String(error).includes("not prepared");
      }
    }),
  ).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
