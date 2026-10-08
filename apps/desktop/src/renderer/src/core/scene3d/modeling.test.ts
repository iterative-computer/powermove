import { describe, it, expect } from "vitest";
import { makePM } from "../../legacy/__tests__/make-pm";
import {
  LAYER3D_DEFINITIONS,
  compositionScene,
  layer3DAssetIds,
} from "./layers";
import { pasteLayers } from "../../legacy/ui/shortcuts";
import { generatedCommands } from "./modeling";
import { materialPreset, assignedMaterial } from "./materials";
import {
  recipeSchema,
  shaderGraphSchema,
  blenderMaterialSchema,
  render3DSchema,
} from "../../../../shared/blender";
import { editScene } from "./operations";
import { compositionRuntime } from "./service";
const mesh = {
  positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  uvs: [0, 0, 1, 0, 0, 1],
  groups: [{ start: 0, count: 3, materialIndex: 0 }],
};
function editor() {
  const PM = makePM(
    "core/easing",
    "core/model",
    "core/selection",
    "core/anim",
    "core/history",
    "core/editing",
  );
  PM.proj = PM.mkProject({ name: "Models", dur: 5, fps: 30 });
  PM.time = 0;
  PM.layerDefinition = (id: string) =>
    Object.values(LAYER3D_DEFINITIONS).includes(id as any)
      ? {
          id,
          label: "3D",
          version: 1,
          params: [],
          defaults: {},
          renderer: { kind: "layer3d" },
        }
      : null;
  return PM;
}
const output = (keys = ["body", "nose"], y = 2) => ({
  files: [],
  manifest: {
    parts: keys.map((key, index) => ({
      key,
      name: key,
      mesh,
      base: {
        x: 0,
        y: index ? y : 0,
        z: 0,
        rx: 0,
        ry: 0,
        rz: 0,
        sx: 1,
        sy: 1,
        sz: 1,
      },
      materials: [],
      blender: { object: key },
    })),
  },
});
const create = (PM: any) => {
  const plan = generatedCommands(
    PM,
    output(),
    "source",
    recipeSchema.parse({ kind: "rocket" }),
  );
  const result = PM.Edit.apply(plan.commands);
  expect(result, result.message).toMatchObject({ ok: true });
  return PM.L(plan.groupId);
};
describe("Blender model recipes and native editing", () => {
  it("reuses animated materials in composition time without duplicating keyframe IDs", () => {
    const PM = editor(),
      shader = materialPreset("surface", "shared");
    shader.p.roughness!.kf = [{ i: "source_key", t: 1, v: 0.6 }];
    const source = { from: 0 },
      target = { from: 3 },
      material = { shader, p: {} };
    const next = assignedMaterial(PM, material, source, target, null);
    expect(next.shader.p.roughness.kf[0].t).toBe(-2);
    expect(next.shader.p.roughness.kf[0].i).not.toBe("source_key");
    const changed = assignedMaterial(PM, material, source, target, next);
    expect(changed.shader.p.roughness.kf[0].i).toBe(
      next.shader.p.roughness.kf[0].i,
    );
    expect(shader.p.roughness!.kf[0]!.t).toBe(1);
  });
  it("creates native parts in one undo, centred in the composition", () => {
    const PM = editor(),
      count = PM.hist.list().length,
      group = create(PM);
    expect(group.threeD).toBe(true);
    // The model sits at the composition centre; its parts are placed around the group's origin.
    expect(group.p["position.x"].v).toBe(960);
    expect(group.p["anchor.x"].v).toBe(0);
    expect(group.collapsed).toBe(true);
    expect(PM.proj.layers).toHaveLength(3);
    expect(PM.hist.list().length).toBe(count + 1);
    expect(PM.L(group.d.modeling.parts.nose.layerId).parent).toBe(group.id);
    PM.hist.undo();
    expect(PM.proj.layers).toHaveLength(0);
  });
  it("regenerates stable parts without losing materials or animated overrides", () => {
    const PM = editor(),
      group = create(PM),
      nose = PM.L(group.d.modeling.parts.nose.layerId);
    nose.d.data.object.material.shader = materialPreset("noise", "paint");
    PM.Edit.apply({
      type: "replace_keyframes",
      target: nose.id,
      path: "position.y",
      keyframes: [
        { time: 0, value: 3, eo: [0.1, 0.2], ei: [0.7, 0.8] },
        { time: 1, value: 5 },
      ],
    });
    const shader = JSON.stringify(nose.d.data.object.material),
      before = JSON.stringify(PM.proj);
    const plan = generatedCommands(
      PM,
      output(["body", "nose"], 4),
      "newSource",
      recipeSchema.parse({ kind: "rocket", parameters: { length: 4 } }),
      group.id,
    );
    const result = PM.Edit.apply(plan.commands);
    expect(result, result.message).toMatchObject({ ok: true });
    const next = PM.L(nose.id);
    expect(next.d.data.object.material).toEqual(JSON.parse(shader));
    // The part moved 2 units up in Blender: 400 px toward the top of the composition.
    expect(next.p["position.y"].kf.map((k: any) => k.v)).toEqual([-397, -395]);
    expect(next.p["position.y"].kf[0].eo).toEqual([0.1, 0.2]);
    expect(PM.L(group.id).d.modeling.parts.nose.layerId).toBe(nose.id);
    PM.hist.undo();
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
  it("replaces Blender source graphs while preserving part IDs and keyed material overrides", () => {
    const PM = editor();
    const imported = (roughness: number) => {
      const result = output(["body"]);
      (result.manifest.parts[0]!.materials as any[]).push({
        id: "slot0",
        name: "Paint",
        p: {
          color: { v: "#ffffff", kf: [], expr: null },
          roughness: { v: roughness, kf: [], expr: null },
        },
        inputs: [
          {
            id: "roughness",
            node: "Principled BSDF",
            socket: "Roughness",
            label: "Roughness",
            kind: "number",
            min: 0,
            max: 1,
            value: roughness,
          },
        ],
      });
      return result;
    };
    const first = generatedCommands(
      PM,
      imported(0.2),
      "first",
      recipeSchema.parse({ kind: "rocket" }),
    );
    expect(PM.Edit.apply(first.commands).ok).toBe(true);
    const id = PM.L(first.groupId).d.modeling.parts.body.layerId;
    expect(
      PM.Edit.apply({
        type: "replace_keyframes",
        target: id,
        path: "slots.slot0.shader.roughness",
        keyframes: [
          { time: 0, value: 0.4 },
          { time: 1, value: 0.8 },
        ],
      }).ok,
    ).toBe(true);
    const keys = structuredClone(
        PM.L(id).d.data.object.slots[0].material.shader.p.roughness.kf,
      ),
      before = JSON.stringify(PM.proj);
    expect(
      PM.Edit.apply(
        generatedCommands(
          PM,
          imported(0.9),
          "second",
          undefined,
          first.groupId,
          true,
        ).commands,
      ).ok,
    ).toBe(true);
    const group = PM.L(first.groupId),
      material = PM.L(id).d.data.object.slots[0].material;
    expect(group.d.modeling.parts.body.layerId).toBe(id);
    expect(group.d.modeling.recipe.kind).toBe("mesh");
    expect(material.shader.source.assetId).toBe("second");
    expect(material.shader.p.roughness.kf).toEqual(keys);
    expect(PM.L(id).d.data.object.material.shader.source.assetId).toBe(
      "second",
    );
    PM.hist.undo();
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
  it("keeps removed parts recoverable and reuses their layer IDs on return", () => {
    const PM = editor(),
      group = create(PM),
      id = group.d.modeling.parts.nose.layerId;
    PM.L(id).p.rotation.v = 72;
    expect(
      PM.Edit.apply(
        generatedCommands(
          PM,
          output(["body"]),
          "s2",
          recipeSchema.parse({ kind: "rocket" }),
          group.id,
        ).commands,
      ).ok,
    ).toBe(true);
    expect(PM.L(id).on).toBe(false);
    expect(PM.L(group.id).d.modeling.retired).toContain("nose");
    expect(
      PM.Edit.apply(
        generatedCommands(
          PM,
          output(),
          "s3",
          recipeSchema.parse({ kind: "rocket" }),
          group.id,
        ).commands,
      ).ok,
    ).toBe(true);
    expect(PM.L(id).on).toBe(true);
    expect(PM.L(id).p.rotation.v).toBe(72);
    expect(PM.proj.layers).toHaveLength(3);
  });
  it("evaluates model and material channels through the existing timeline", () => {
    const PM = editor(),
      group = create(PM),
      id = group.d.modeling.parts.body.layerId;
    expect(
      PM.Edit.apply({
        type: "replace_keyframes",
        target: group.id,
        path: "model.length",
        keyframes: [
          { time: 0, value: 2 },
          { time: 2, value: 4 },
        ],
      }).ok,
    ).toBe(true);
    expect(
      PM.evP(group, PM.findProp(group, "model.length"), 1, "model.length"),
    ).toBeCloseTo(3);
    const world = compositionRuntime(PM, 1),
      geometry = (world.objects.get(id) as any).geometry;
    geometry.computeBoundingBox();
    expect(geometry.boundingBox.max.y - geometry.boundingBox.min.y).toBeCloseTo(
      3,
    );
    expect(
      PM.Edit.apply({
        type: "set_property",
        target: group.id,
        path: "model.length",
        value: 500,
      }).ok,
    ).toBe(false);
    expect(PM.findProp(group, "model.fins")).toBeNull();
  });
  it("duplicates construction groups without keeping links to the original parts", () => {
    const PM = editor(),
      group = create(PM),
      original = JSON.parse(JSON.stringify(PM.proj.layers));
    PM.selectLayers([group.id]);
    const result = pasteLayers(PM, () => original, {
      atPlayhead: false,
    }) as any;
    expect(result, result.message).toMatchObject({ ok: true });
    const copies: any[] = PM.selLayers();
    const copy = copies.find((l) => l.d?.modeling),
      body = copies.find(
        (l) => l.d?.data?.object?.generation?.partId === "body",
      );
    expect(copy.id).not.toBe(group.id);
    expect(copy.d.modeling.parts.body.layerId).toBe(body.id);
    expect(body.d.data.object.generation.groupId).toBe(copy.id);
    expect(body.parent).toBe(copy.id);
    PM.hist.undo();
    expect(PM.proj.layers).toEqual(original);
  });
  it("rejects regeneration of locked parts before changing anything", () => {
    const PM = editor(),
      group = create(PM);
    PM.L(group.d.modeling.parts.body.layerId).lock = true;
    const before = JSON.stringify(PM.proj);
    expect(() =>
      generatedCommands(
        PM,
        output(),
        "s2",
        recipeSchema.parse({ kind: "rocket" }),
        group.id,
      ),
    ).toThrow(/Unlock/);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
});
describe("actual Blender materials and render settings", () => {
  it("validates every preset and binds exposed controls to shader nodes", () => {
    for (const preset of [
      "surface",
      "noise",
      "checker",
      "metal",
      "glass",
      "emission",
    ] as const) {
      const material = materialPreset(preset, "material");
      expect(blenderMaterialSchema.parse(material)).toEqual(material);
      expect(
        material.graph!.nodes.some(
          (n) => n.type === "ShaderNodeOutputMaterial",
        ),
      ).toBe(true);
      for (const input of material.inputs)
        expect(material.p[input.id]).toBeDefined();
    }
  });
  it("rejects executable nodes, malformed graphs and out-of-range controls", () => {
    expect(() =>
      shaderGraphSchema.parse({
        nodes: [{ id: "python", type: "PythonScript" }],
        links: [],
      }),
    ).toThrow();
    const shader = materialPreset("noise", "paint");
    expect(() =>
      blenderMaterialSchema.parse({
        ...shader,
        p: { ...shader.p, scale: { v: -2, kf: [], expr: null } },
      }),
    ).toThrow();
    expect(() =>
      shaderGraphSchema.parse({
        ...shader.graph,
        links: [
          ...shader.graph!.links,
          { from: "output", output: "Surface", to: "surface", input: "Normal" },
        ],
      }),
    ).toThrow(/cycles/);
    expect(() =>
      shaderGraphSchema.parse({
        ...shader.graph,
        links: [
          {
            from: "missing",
            output: "Color",
            to: "surface",
            input: "Base Color",
          },
        ],
      }),
    ).toThrow();
  });
  it("shares edits atomically and protects locked users", () => {
    const PM = editor(),
      shader = materialPreset("surface", "shared");
    for (const id of ["a", "b"])
      expect(
        editScene(PM, {
          operation: "add_object",
          object: { id, source: { primitive: "box" }, material: { shader } },
        }).ok,
      ).toBe(true);
    const count = PM.hist.list().length;
    expect(
      PM.Edit.apply({
        type: "set_property",
        target: "a",
        path: "shader.roughness",
        value: 0.8,
      }).ok,
    ).toBe(true);
    expect(PM.findProp(PM.L("b"), "shader.roughness").v).toBe(0.8);
    expect(PM.hist.list().length).toBe(count + 1);
    PM.hist.undo();
    expect(PM.findProp(PM.L("a"), "shader.roughness").v).toBe(0.32);
    expect(PM.findProp(PM.L("b"), "shader.roughness").v).toBe(0.32);
    PM.L("b").lock = true;
    const before = JSON.stringify(PM.proj);
    expect(
      PM.Edit.apply({
        type: "set_property",
        target: "a",
        path: "shader.color",
        value: "#FF0000",
      }).ok,
    ).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
  it("keeps shared material keys unique and aligned to composition time", () => {
    const PM = editor(),
      shader = materialPreset("surface", "shared");
    for (const id of ["a", "b"])
      expect(
        editScene(PM, {
          operation: "add_object",
          object: { id, source: { primitive: "box" }, material: { shader } },
        }).ok,
      ).toBe(true);
    PM.L("b").from = 2;
    expect(
      PM.Edit.apply({
        type: "replace_keyframes",
        target: "a",
        path: "shader.roughness",
        keyframes: [
          { time: 0, value: 0.1 },
          { time: 4, value: 0.9 },
        ],
      }).ok,
    ).toBe(true);
    const a = PM.findProp(PM.L("a"), "shader.roughness"),
      b = PM.findProp(PM.L("b"), "shader.roughness");
    expect(a.kf.map((k: any) => k.i)).not.toEqual(b.kf.map((k: any) => k.i));
    expect(b.kf.map((k: any) => k.t)).toEqual([-2, 2]);
    expect(
      PM.Edit.apply({
        type: "replace_keyframes", target: "b", path: "shader.roughness",
        keyframes: [{ time: -2, value: 0.2 }, { time: 2, value: 0.8 }],
      }).ok,
    ).toBe(true);
    expect(b.kf.map((k: any) => k.t)).toEqual([-2, 2]);
    expect(a.kf.map((k: any) => k.t)).toEqual([0, 4]);
    expect(PM.Edit.apply({type: "set_property", target: "b", path: "shader.roughness", mode: "keyframe", time: 0, value: 0.3}).ok).toBe(true);
    expect(b.kf.map((k: any) => k.t)).toEqual([-2, 2]);
    expect(b.kf[0].v).toBe(0.3);
    expect(a.kf[0].v).toBe(0.3);
    expect(PM.Edit.apply({type: "replace_keyframes", target: "b", path: "m.roughness", keyframes: [{time: -2,value: 0.4}]}).ok).toBe(true);
    expect(PM.findProp(PM.L("b"), "m.roughness").kf[0].t).toBe(-2);
    expect(PM.evP(PM.L("a"), a, 3, "shader.roughness")).toBeCloseTo(
      PM.evP(PM.L("b"), b, 3, "shader.roughness"),
    );
    const before = JSON.stringify(PM.proj);
    expect(
      PM.Edit.apply({
        type: "set_easing",
        keyframes: [a.kf[0].i],
        curve: "ease",
      }).ok,
    ).toBe(true);
    expect(b.kf[0].outEase).toEqual(a.kf[0].outEase);
    PM.hist.undo();
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
  it("animates shader inputs per material slot and retains source assets", () => {
    const PM = editor();
    const shader = materialPreset("noise", "paint");
    shader.source = { assetId: "blendSource", material: "Paint" };
    delete shader.graph;
    shader.inputs = shader.inputs.map((i) => ({
      ...i,
      node: "Principled BSDF",
    }));
    expect(
      editScene(PM, {
        operation: "add_object",
        object: {
          id: "a",
          source: { mesh },
          slots: [{ id: "paint", name: "Paint", material: { shader } }],
        },
      }).ok,
    ).toBe(true);
    expect(
      PM.Edit.apply({
        type: "replace_keyframes",
        target: "a",
        path: "slots.paint.shader.roughness",
        keyframes: [
          { time: 0, value: 0.1 },
          { time: 2, value: 0.9 },
        ],
      }).ok,
    ).toBe(true);
    expect(
      compositionScene(PM, 1).objects[0]!.slots[0]!.material.shader!.p
        .roughness!.v,
    ).toBeCloseTo(0.5);
    expect(layer3DAssetIds(PM.L("a"))).toContain("blendSource");
    expect(PM.findProp(PM.L("a"), "m.mask.opacity")).toBeNull();
  });
  it("stores engine settings in the project with undo and validates limits", () => {
    const PM = editor(),
      before = JSON.stringify(PM.proj),
      settings = render3DSchema.parse({
        enabled: true,
        engine: "cycles",
        samples: 128,
      });
    expect(
      PM.Edit.apply({ type: "set_composition", patch: { render3d: settings } })
        .ok,
    ).toBe(true);
    expect(PM.proj.render3d.engine).toBe("cycles");
    PM.hist.undo();
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(
      PM.Edit.apply({
        type: "set_composition",
        patch: { render3d: { samples: 1e9 } },
      }).ok,
    ).toBe(false);
  });
  it("bounds recipes and excludes topology parameters from animation", () => {
    expect(recipeSchema.parse({ kind: "rocket" }).parameters).toMatchObject({
      length: 2.8,
      fins: 4,
    });
    expect(() =>
      recipeSchema.parse({ kind: "rocket", parameters: { fins: 2.5 } }),
    ).toThrow();
    expect(() =>
      recipeSchema.parse({ kind: "staircase", parameters: { steps: 500 } }),
    ).toThrow();
    expect(() =>
      recipeSchema.parse({ kind: "rocket", parameters: { execute: "python" } }),
    ).toThrow();
    expect(() =>
      recipeSchema.parse({
        kind: "lathe",
        profile: [
          [-1, 0],
          [2, 1],
        ],
      }),
    ).toThrow();
  });
});
