import { bridge } from "../../kernel/bridge";
import {
  generatedModelSchema,
  recipeSchema,
  RECIPE_PARAMETERS,
  type ModelRecipe,
  type BlenderOutput,
} from "../../../../shared/blender";
import { createObject, parseScene } from "./schema";
import { sceneCommands } from "./operations";
import { importedMaterial } from "./materials";
import { layer3DRole } from "./layers";
import { compCenter, modelScale, PX_PER_UNIT } from "./space";
import type { EditMeta, EditResult } from "../types/commands";
const clone = (v: any) => JSON.parse(JSON.stringify(v));
export const MODEL_RECIPES = [
  {
    id: "rocket",
    label: "Rocket",
    parameters: { length: 2.8, radius: 0.6, nose: 1, fins: 4, finSize: 0.8 },
  },
  {
    id: "staircase",
    label: "Staircase",
    parameters: {
      steps: 12,
      radius: 1.5,
      rise: 0.22,
      turn: 30,
      width: 1.2,
      depth: 0.5,
    },
  },
  {
    id: "text",
    label: "3D Text",
    parameters: { text: "Powermove", size: 1, depth: 0.12, bevel: 0.02 },
  },
] as const;
export const MODEL_FIELDS: Record<
  string,
  { label: string; min?: number; max?: number; step?: number; kind?: string }
> = {
  length: { label: "Body length", min: 0.1, max: 50, step: 0.1 },
  radius: { label: "Radius", min: 0.05, max: 30, step: 0.05 },
  nose: { label: "Nose length", min: 0.05, max: 20, step: 0.1 },
  fins: { label: "Fins", min: 0, max: 16, step: 1 },
  finSize: { label: "Fin size", min: 0.05, max: 10, step: 0.05 },
  steps: { label: "Steps", min: 1, max: 128, step: 1 },
  rise: { label: "Rise", min: 0.01, max: 5, step: 0.01 },
  turn: { label: "Turn", min: -180, max: 180, step: 1 },
  width: { label: "Width", min: 0.05, max: 10, step: 0.1 },
  depth: { label: "Depth", min: 0, max: 50, step: 0.01 },
  size: { label: "Size", min: 0.01, max: 50, step: 0.1 },
  bevel: { label: "Bevel", min: 0, max: 1, step: 0.01 },
  text: { label: "Text", kind: "text" },
  segments: { label: "Segments", min: 3, max: 128, step: 1 },
};
export const modelFieldsFor = (kind: string) =>
  Object.fromEntries(
    Object.entries(RECIPE_PARAMETERS[kind] || {}).map(([key, field]) => [
      key,
      {
        ...MODEL_FIELDS[key],
        min: field.min,
        max: field.max,
        step: field.integer ? 1 : MODEL_FIELDS[key]?.step,
      },
    ]),
  );
/* Recipe parts are laid out in model units (y up) relative to their group;
   layers hold composition pixels (y down, z away), so offsets scale and flip. */
const PART_FACTOR: Record<string, number> = {
  x: PX_PER_UNIT, y: -PX_PER_UNIT, z: -PX_PER_UNIT, rx: 1, ry: -1, rz: -1, sx: 1, sy: 1, sz: 1,
};
const partPixels = (base: Record<string, number>) =>
  Object.fromEntries(Object.entries(base).map(([key, value]) => [key, value * (PART_FACTOR[key] ?? 1)]));
const transformPath: Record<string, string> = {
  x: "position.x",
  y: "position.y",
  z: "position.z",
  rx: "rotation.x",
  ry: "rotation.y",
  rz: "rotation",
  sx: "scale.x",
  sy: "scale.y",
  sz: "scale.z",
};
export function evaluatedRecipe(
  PM: any,
  group: any,
  time = PM.time,
): ModelRecipe {
  const model = generatedModelSchema.parse(group.d.modeling),
    parameters = { ...model.recipe.parameters };
  for (const [key, prop] of Object.entries(model.p))
    parameters[key] = PM.evP
      ? PM.evP(group, prop, time, `model.${key}`)
      : (prop.v as any);
  return recipeSchema.parse({ ...model.recipe, parameters });
}
export function generatedCommands(
  PM: any,
  result: BlenderOutput,
  sourceAssetId: string,
  recipe: ModelRecipe | undefined,
  target?: string,
  refreshMaterials = false,
): { commands: any[]; groupId: string } {
  const existing = target ? PM.L(target) : null;
  if (
    target &&
    (!existing || existing.type !== "group" || !existing.d?.modeling)
  )
    throw new Error("Choose a generated model group");
  if (
    existing &&
    (existing.lock ||
      (PM.groupAncestors?.(existing) || []).some((g: any) => g.lock))
  )
    throw new Error("Unlock the model group first");
  const groupId = existing?.id || PM.uid("model");
  const commands: any[] = [];
  const old = existing?.d.modeling;
  if (!existing) {
    commands.push({
      type: "add_layer",
      id: groupId,
      layerType: "group",
      name: recipe
        ? MODEL_RECIPES.find((r) => r.id === recipe.kind)?.label || "3D Model"
        : "Blender Model",
      properties: {
        "position.x": compCenter(PM.curComp?.() || PM.proj).x,
        "position.y": compCenter(PM.curComp?.() || PM.proj).y,
        "position.z": 0,
        "rotation.x": 0,
        "rotation.y": 0,
        rotation: 0,
        "scale.x": 100 * modelScale(PM.curComp?.() || PM.proj),
        "scale.y": 100 * modelScale(PM.curComp?.() || PM.proj),
        "scale.z": 100 * modelScale(PM.curComp?.() || PM.proj),
        "anchor.x": 0,
        "anchor.y": 0,
        "anchor.z": 0,
      },
      select: true,
    });
    commands.push({
      type: "set_layer",
      target: groupId,
      patch: { threeD: true, collapsed: true },
    });
  }
  const parts: any = {},
    seen = new Set<string>(),
    materialIds = new Map<string, string>();
  const stablePart = (name: string) => {
    let hash = 2166136261;
    for (const char of name)
      hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
    return `part_${(hash >>> 0).toString(36)}`;
  };
  for (let index = 0; index < result.manifest.parts.length; index++) {
    const part = result.manifest.parts[index],
      key = part.key;
    // Imported Blender names are arbitrary; deterministic slot IDs avoid unsafe channel paths.
    const partId = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(key)
      ? key
      : stablePart(key);
    if (seen.has(partId)) throw new Error("Generated part IDs must be unique");
    seen.add(partId);
    const prior = old?.parts?.[partId],
      layer = prior ? PM.L(prior.layerId) : null,
      id = layer?.id || PM.uid("model");
    const slots = (part.materials || []).map((raw: any) => {
      let materialId = materialIds.get(raw.name);
      if (!materialId) {
        materialId = `material_${crypto.randomUUID().replaceAll("-", "")}`;
        materialIds.set(raw.name, materialId);
      }
      return {
        id: raw.id,
        name: raw.name,
        material: {
          p: raw.p,
          shader: importedMaterial(raw, sourceAssetId, materialId),
        },
      };
    });
    const object = createObject(id, { mesh: part.mesh }, part.name);
    object.blender = { assetId: sourceAssetId, object: part.blender.object };
    object.generation = { groupId, partId };
    object.slots = parseScene({
      objects: [{ ...object, slots }],
    }).objects[0]!.slots;
    if (object.slots[0]) object.material = clone(object.slots[0].material);
    if (layer) {
      if (
        layer.lock ||
        (PM.groupAncestors?.(layer) || []).some((g: any) => g.lock)
      )
        throw new Error(`Unlock ${layer.name} before regenerating`);
      const content = {
        ...clone(layer.d.data),
        object: {
          ...clone(layer.d.data.object),
          source: object.source,
          blender: object.blender,
          generation: object.generation,
        },
      };
      if (refreshMaterials) {
        content.object.slots = object.slots.map((next) => {
          const prior = (layer.d.data.object.slots || []).find(
            (slot: any) => slot.name === next.name,
          );
          if (!prior) return next;
          if (
            !prior.material.shader?.source ||
            prior.material.shader.source.material !== next.name
          )
            return { ...next, id: prior.id, material: clone(prior.material) };
          const shader = clone(next.material.shader),
            oldShader = prior.material.shader,
            used = new Set<string>();
          const p: Record<string, any> = {};
          shader.inputs = shader.inputs.map((field: any, index: number) => {
            const oldField = oldShader.inputs.find(
              (item: any) =>
                item.node === field.node &&
                item.socket === field.socket &&
                item.kind === field.kind,
            );
            let id = oldField?.id || field.id;
            if (used.has(id)) id = `input_new${index}`;
            used.add(id);
            const oldProp = oldField ? oldShader.p[oldField.id] : null,
              path = `slots.${prior.id}.shader.${oldField?.id}`;
            p[id] =
              oldProp &&
              (oldProp.kf.length || oldProp.expr || layer.locked_intent?.[path])
                ? clone(oldProp)
                : shader.p[field.id];
            return { ...field, id };
          });
          shader.p = p;
          return {
            ...next,
            id: prior.id,
            material: { ...clone(prior.material), shader },
          };
        });
        if (content.object.slots[0]) {
          content.object.material = clone(content.object.slots[0].material);
          // Slots own animation identities; the fallback material is an independent copy.
          for (const prop of [
            ...Object.values<any>(content.object.material.p || {}),
            ...Object.values<any>(content.object.material.shader?.p || {}),
          ])
            for (const key of prop.kf || []) key.i = PM.uid("kf");
        }
      }
      // Geometry changes around the existing layer. Human materials, transforms and keys survive.
      commands.push({
        type: "set_content",
        target: id,
        patch: { data: content },
      });
      for (const [key, path] of Object.entries(transformPath)) {
        const prop = layer.p[path],
          factor = key.startsWith("s") ? 100 : (PART_FACTOR[key] ?? 1),
          delta =
            (part.base[key] -
              (prior.base[key] ?? (key.startsWith("s") ? 1 : 0))) *
            factor;
        if (delta && prop)
          commands.push({
            type: "offset_property",
            target: id,
            path,
            delta,
            preserveHandEdits: false,
          });
      }
      if (old.retired?.includes(partId))
        commands.push({
          type: "set_layer",
          target: id,
          patch: { visible: true },
        });
    } else {
      const additions = sceneCommands(PM, {
        operation: "add_object",
        object: { ...object, parent: groupId, p: partPixels(part.base) },
      }).commands;
      for (const command of additions)
        if (command.type === "add_layer")
          command.index =
            (existing ? PM.proj.layers.indexOf(existing) : 0) + index + 1;
      commands.push(...additions);
    }
    parts[partId] = { layerId: id, base: part.base };
  }
  const retired: string[] = [];
  for (const [partId, part] of Object.entries<any>(old?.parts || {}))
    if (!seen.has(partId)) {
      parts[partId] = part;
      retired.push(partId);
      const layer = PM.L(part.layerId);
      if (layer) {
        if (layer.lock)
          throw new Error(`Unlock ${layer.name} before regenerating`);
        commands.push({
          type: "set_layer",
          target: layer.id,
          patch: { visible: false },
        });
      }
    }
  const nextRecipe = refreshMaterials
    ? recipeSchema.parse({ kind: "mesh" })
    : recipe || old?.recipe || { kind: "mesh", parameters: {}, modifiers: [] };
  const p = Object.fromEntries(
    Object.entries(nextRecipe.parameters)
      .map(([key, value]) => [
        key,
        old?.p?.[key]
          ? old.p[key].kf.length || old.p[key].expr
            ? old.p[key]
            : { ...old.p[key], v: value }
          : { v: value, kf: [], expr: null },
      ])
      .filter(
        ([key, value]: any) =>
          typeof value.v !== "string" &&
          !RECIPE_PARAMETERS[nextRecipe.kind]?.[key]?.integer,
      ),
  );
  const model = generatedModelSchema.parse({
    version: 1,
    recipe: nextRecipe,
    p,
    parts,
    retired,
    sourceAssetId,
  });
  commands.push({
    type: "set_content",
    target: groupId,
    patch: { modeling: model },
  });
  return { commands, groupId };
}
async function saveSource(
  PM: any,
  file: BlenderOutput["files"][number],
  name?: string,
): Promise<string> {
  const id = PM.uid("asset"),
    blob = new Blob([new Uint8Array(file.data)], {
      type: "application/x-blender",
    });
  const label = name ? `${name.replace(/\.blend$/i, "")}.blend` : file.name;
  const asset = {
    id,
    name: label,
    kind: "model",
    format: "blend",
    storageKey: id,
    size: blob.size,
  };
  if (!(await PM.MediaStore.put(id, blob, asset)))
    throw new Error("Could not save the Blender source");
  PM.proj.assets[id] = asset;
  PM.assets?.map?.set(id, { ...asset, blob });
  return id;
}
export async function editModel(
  PM: any,
  args: {
    operation: "create_model" | "regenerate_model" | "import_blend";
    target?: string;
    recipe?: unknown;
    name?: string;
    file?: File;
    sourceAssetId?: string;
  },
  meta: EditMeta = {},
): Promise<EditResult> {
  const project = PM.proj,
    revision = PM._scene3dRevision || 0;
  let sourceId: string | undefined;
  try {
    const host = bridge()?.blender;
    if (!host) throw new Error("Blender modeling requires the desktop app");
    const target =
        args.target ||
        (args.operation === "regenerate_model" ? PM.sel.layers[0] : undefined),
      group = target ? PM.L(target) : null;
    const recipe =
      args.operation === "import_blend"
        ? undefined
        : recipeSchema.parse(
            args.recipe ||
              (group
                ? evaluatedRecipe(PM, group)
                : { kind: "rocket", parameters: {}, modifiers: [] }),
          );
    const assets: any[] = [];
    let sourceAssetId = args.sourceAssetId;
    if (args.file) {
      sourceAssetId = "importSource";
      assets.push({
        id: sourceAssetId,
        name: args.file.name,
        data: new Uint8Array(await args.file.arrayBuffer()),
      });
    } else if (sourceAssetId) {
      const asset = PM.proj.assets[sourceAssetId],
        blob = await PM.MediaStore.get(asset);
      if (!blob) throw new Error("Missing Blender source asset");
      assets.push({
        id: sourceAssetId,
        name: asset.name,
        data: new Uint8Array(await blob.arrayBuffer()),
      });
    }
    const result = await host.run({
      id: `job_${crypto.randomUUID().replaceAll("-", "")}`,
      operation: args.operation === "import_blend" ? "import" : "generate",
      recipe,
      sourceAssetId,
      assets,
    });
    if (PM.proj !== project || (PM._scene3dRevision || 0) !== revision)
      throw new Error(
        "The project changed while the model was generating. Retry to preserve those edits.",
      );
    const source = result.files.find((f) => f.id === "source");
    if (!source) throw new Error("Blender returned no source");
    sourceId = await saveSource(
      PM,
      source,
      args.name ||
        group?.name ||
        MODEL_RECIPES.find((r) => r.id === recipe?.kind)?.label ||
        args.file?.name,
    );
    if (PM.proj !== project || (PM._scene3dRevision || 0) !== revision)
      throw new Error(
        "The project changed while the model was saving. Retry to preserve those edits.",
      );
    const plan = generatedCommands(
      PM,
      result,
      sourceId,
      recipe,
      target,
      args.operation === "import_blend",
    );
    if (args.operation !== "regenerate_model" && !PM.proj.render3d)
      plan.commands.push({
        type: "set_composition",
        patch: { render3d: { enabled: true } },
      });
    if (args.name)
      plan.commands.push({
        type: "set_layer",
        target: plan.groupId,
        patch: { name: args.name },
      });
    const label =
      args.operation === "regenerate_model"
        ? "Regenerate model"
        : args.operation === "import_blend"
          ? "Import Blender model"
          : "Create 3D model";
    PM.hist.beginScoped(
      label,
      meta.historyGroup || null,
      meta.origin || "interface",
    );
    try {
      const applied = PM.Edit.apply(plan.commands, {
        label,
        origin: "interface",
        ...meta,
      });
      if (!applied.ok) throw new Error(applied.message);
      PM.selectLayers?.([plan.groupId]);
      PM.hist.commit(label);
      PM.bus?.emit?.("assets");
      void PM.Projects?.put?.(project)?.catch?.((error: unknown) =>
        PM.toast?.(`Could not checkpoint model media: ${String(error)}`),
      );
      applied.data.result = {
        id: plan.groupId,
        layerId: plan.groupId,
        parts: result.manifest.parts.length,
      };
      return applied;
    } catch (error) {
      PM.hist.rollback();
      throw error;
    }
  } catch (error) {
    if (sourceId) {
      delete project.assets[sourceId];
      PM.assets?.map?.delete(sourceId);
    }
    return {
      ok: false,
      message: error instanceof Error ? error.message : String(error),
    };
  }
}
