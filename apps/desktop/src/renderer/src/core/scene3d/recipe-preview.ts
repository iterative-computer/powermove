import * as THREE from "three";
import type { ModelRecipe } from "../../../../shared/blender";
import type { SceneSource } from "./schema";
import { evaluatedRecipe } from "./modeling";
const cache = new Map<
  string,
  Map<string, { source: SceneSource; base: Record<string, number> }>
>();
/** Cheap parametric geometry stays interactive; Blender applies modifiers in rendered mode. */
export function recipeParts(recipe: ModelRecipe) {
  const signature = JSON.stringify(recipe);
  if (cache.has(signature)) return cache.get(signature)!;
  const p = recipe.parameters,
    parts = new Map<
      string,
      { source: SceneSource; base: Record<string, number> }
    >();
  const n = (key: string, value: number) =>
    typeof p[key] === "number" ? (p[key] as number) : value;
  const add = (
    id: string,
    source: SceneSource,
    base: Record<string, number> = {},
  ) =>
    parts.set(id, {
      source,
      base: {
        x: 0,
        y: 0,
        z: 0,
        rx: 0,
        ry: 0,
        rz: 0,
        sx: 1,
        sy: 1,
        sz: 1,
        ...base,
      },
    });
  if (recipe.kind === "rocket") {
    const radius = n("radius", 0.6),
      length = n("length", 2.8),
      nose = n("nose", 1),
      fins = Math.floor(n("fins", 4)),
      fin = n("finSize", 0.8);
    add("body", {
      primitive: "cylinder",
      parameters: { radius, height: length },
    });
    add(
      "nose",
      { primitive: "cone", parameters: { radius, height: nose } },
      { y: length / 2 + nose / 2 },
    );
    add(
      "engine",
      {
        primitive: "cylinder",
        parameters: {
          radiusTop: radius * 0.75,
          radiusBottom: radius * 0.55,
          height: radius * 0.7,
        },
      },
      { y: -length / 2 - radius * 0.35 },
    );
    const vertices = [
      radius * 0.8,
      -length * 0.15,
      -0.06,
      radius + fin,
      -length * 0.55,
      -0.06,
      radius * 0.8,
      -length * 0.5,
      -0.06,
      radius * 0.8,
      -length * 0.15,
      0.06,
      radius + fin,
      -length * 0.55,
      0.06,
      radius * 0.8,
      -length * 0.5,
      0.06,
    ];
    const indices = [
      0, 2, 1, 3, 4, 5, 0, 1, 4, 0, 4, 3, 1, 2, 5, 1, 5, 4, 2, 0, 3, 2, 3, 5,
    ];
    for (let i = 0; i < fins; i++) {
      const angle = (i * Math.PI * 2) / Math.max(1, fins),
        positions = vertices.map((v, index) =>
          index % 3 === 1
            ? v
            : index % 3 === 0
              ? v * Math.cos(angle) + vertices[index + 2]! * Math.sin(angle)
              : -vertices[index - 2]! * Math.sin(angle) + v * Math.cos(angle),
        );
      add(`fin_${String(i).padStart(2, "0")}`, {
        mesh: { positions, indices },
      });
    }
  } else if (recipe.kind === "staircase") {
    const count = Math.floor(n("steps", 12)),
      radius = n("radius", 1.5),
      rise = n("rise", 0.22),
      turn = n("turn", 30),
      width = n("width", 1.2),
      depth = n("depth", 0.5);
    for (let i = 0; i < count; i++) {
      const angle = (i * turn * Math.PI) / 180;
      const g = new THREE.BoxGeometry(width, rise * 0.6, depth).rotateY(-angle);
      add(
        `step_${String(i).padStart(3, "0")}`,
        {
          mesh: {
            positions: [...g.attributes.position!.array],
            normals: [...g.attributes.normal!.array],
            uvs: [...g.attributes.uv!.array],
            indices: [...g.index!.array],
          },
        },
        {
          x: radius * Math.cos(angle),
          y: i * rise,
          z: radius * Math.sin(angle),
        },
      );
      g.dispose();
    }
  } else if (recipe.kind === "lathe" && recipe.profile)
    add("model", {
      lathe: recipe.profile,
      segments: Math.floor(n("segments", 48)),
    });
  else if (recipe.kind === "extrude" && recipe.profile) {
    const depth = n("depth", 0.2),
      g = new THREE.ExtrudeGeometry(
        new THREE.Shape(recipe.profile.map((p) => new THREE.Vector2(...p))),
        { depth, steps: 1, bevelEnabled: false },
      ).translate(0, 0, -depth / 2);
    add("model", {
      mesh: {
        positions: [...g.attributes.position!.array],
        normals: [...g.attributes.normal!.array],
        uvs: [...g.attributes.uv!.array],
      },
    });
    g.dispose();
  }
  cache.set(signature, parts);
  if (cache.size > 64) cache.delete(cache.keys().next().value!);
  return parts;
}
export function animatedPart(PM: any, layer: any, time: number) {
  const generation = layer.d?.data?.object?.generation;
  if (!generation) return null;
  const group = PM.L(generation.groupId);
  if (!group?.d?.modeling) return null;
  const recipe = evaluatedRecipe(PM, group, time),
    original = group.d.modeling.recipe;
  if (
    JSON.stringify(recipe.parameters) === JSON.stringify(original.parameters) &&
    JSON.stringify(recipe.profile) === JSON.stringify(original.profile)
  )
    return null;
  const part = recipeParts(recipe).get(generation.partId),
    prior = group.d.modeling.parts[generation.partId];
  return part && prior ? { ...part, prior: prior.base, recipe } : null;
}
