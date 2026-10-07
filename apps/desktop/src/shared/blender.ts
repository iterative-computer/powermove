import { z } from "zod";

/** Reused materials retain composition timing even before a layer's in point. */
export function materialKeyframeMinimum(path: string): number {
  return /^(?:shader|m)\.[^.]+$|^slots\.[^.]+\.(?:shader\.)?[^.]+$/.test(path)
    ? -100000000
    : 0;
}

export const identifier = z.string().regex(/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/);
const assetId = z.string().min(1).max(160);
const finite = z.number().finite();
export const shaderValue = z.union([
  finite.min(-100000).max(100000),
  z.boolean(),
  z.string().regex(/^#[\da-f]{6}$/i),
  z.array(finite.min(-100000).max(100000)).min(2).max(4),
]);
export const shaderChannel = z
  .object({
    v: shaderValue,
    kf: z
      .array(
        z
          .object({ t: finite.min(-100000000).max(100000000), v: shaderValue })
          .passthrough(),
      )
      .max(2400)
      .default([]),
    expr: z.string().max(2000).nullable().default(null),
  })
  .passthrough();
const socket = z.union([
  z.string().min(1).max(160),
  z.number().int().min(0).max(128),
]);
export const SHADER_NODES = [
  "ShaderNodeBsdfPrincipled",
  "ShaderNodeOutputMaterial",
  "ShaderNodeTexCoord",
  "ShaderNodeMapping",
  "ShaderNodeTexNoise",
  "ShaderNodeTexVoronoi",
  "ShaderNodeTexWave",
  "ShaderNodeTexChecker",
  "ShaderNodeTexGradient",
  "ShaderNodeTexImage",
  "ShaderNodeBump",
  "ShaderNodeNormalMap",
  "ShaderNodeMath",
  "ShaderNodeVectorMath",
  "ShaderNodeMixRGB",
  "ShaderNodeMixShader",
  "ShaderNodeBsdfGlass",
  "ShaderNodeBsdfTransparent",
  "ShaderNodeEmission",
  "ShaderNodeValue",
  "ShaderNodeRGB",
  "ShaderNodeSeparateXYZ",
  "ShaderNodeCombineXYZ",
  "ShaderNodeHueSaturation",
  "ShaderNodeInvert",
  "ShaderNodeFresnel",
  "ShaderNodeLayerWeight",
  "ShaderNodeValToRGB",
  "ShaderNodeMapRange",
  "ShaderNodeClamp",
  "ShaderNodeVectorRotate",
] as const;
const nodeProperties = z
  .object({
    operation: z.string().max(64).optional(),
    blend_type: z.string().max(64).optional(),
    noise_dimensions: z.enum(["1D", "2D", "3D", "4D"]).optional(),
    voronoi_dimensions: z.enum(["1D", "2D", "3D", "4D"]).optional(),
    distance: z.string().max(64).optional(),
    feature: z.string().max(64).optional(),
    wave_type: z.enum(["BANDS", "RINGS"]).optional(),
    bands_direction: z.enum(["X", "Y", "Z", "DIAGONAL"]).optional(),
    rings_direction: z.enum(["X", "Y", "Z", "SPHERICAL"]).optional(),
    gradient_type: z.string().max(64).optional(),
    interpolation: z.enum(["Linear", "Closest", "Cubic", "Smart"]).optional(),
    extension: z.enum(["REPEAT", "EXTEND", "CLIP", "MIRROR"]).optional(),
    vector_type: z.enum(["POINT", "TEXTURE", "VECTOR", "NORMAL"]).optional(),
    space: z
      .enum(["TANGENT", "OBJECT", "WORLD", "BLENDER_OBJECT", "BLENDER_WORLD"])
      .optional(),
    distribution: z.string().max(64).optional(),
    use_clamp: z.boolean().optional(),
  })
  .strict();
export const shaderGraphSchema = z
  .object({
    nodes: z
      .array(
        z
          .object({
            id: identifier,
            type: z.enum(SHADER_NODES),
            inputs: z
              .record(z.string().min(1).max(160), shaderValue)
              .default({}),
            properties: nodeProperties.default({}),
            image: assetId.optional(),
            colorSpace: z.enum(["sRGB", "Non-Color"]).optional(),
            ramp: z
              .object({
                interpolation: z
                  .enum(["LINEAR", "EASE", "CONSTANT", "CARDINAL", "B_SPLINE"])
                  .default("LINEAR"),
                stops: z
                  .array(
                    z
                      .object({
                        position: finite.min(0).max(1),
                        color: z.string().regex(/^#[\da-f]{6}$/i),
                      })
                      .strict(),
                  )
                  .min(2)
                  .max(32),
              })
              .strict()
              .optional(),
          })
          .strict(),
      )
      .min(1)
      .max(128),
    links: z
      .array(
        z
          .object({
            from: identifier,
            output: socket,
            to: identifier,
            input: socket,
          })
          .strict(),
      )
      .max(256),
  })
  .strict()
  .superRefine((graph, ctx) => {
    const nodes = new Map(graph.nodes.map((n) => [n.id, n]));
    if (nodes.size !== graph.nodes.length)
      ctx.addIssue({
        code: "custom",
        message: "Shader node IDs must be unique",
      });
    if (!graph.nodes.some((n) => n.type === "ShaderNodeOutputMaterial"))
      ctx.addIssue({
        code: "custom",
        message: "A material needs an output node",
      });
    const edges = new Map<string, string[]>(),
      targets = new Set<string>();
    for (const link of graph.links) {
      if (!nodes.has(link.from) || !nodes.has(link.to))
        ctx.addIssue({
          code: "custom",
          message: "Shader links must address existing nodes",
        });
      const target = `${link.to}:${link.input}`;
      if (targets.has(target))
        ctx.addIssue({
          code: "custom",
          message: "A shader input can have only one incoming link",
        });
      targets.add(target);
      edges.set(link.from, [...(edges.get(link.from) || []), link.to]);
    }
    const visiting = new Set<string>(),
      seen = new Set<string>();
    const visit = (id: string): boolean => {
      if (visiting.has(id)) return false;
      if (seen.has(id)) return true;
      visiting.add(id);
      for (const next of edges.get(id) || []) if (!visit(next)) return false;
      visiting.delete(id);
      seen.add(id);
      return true;
    };
    if (graph.nodes.some((n) => !visit(n.id)))
      ctx.addIssue({
        code: "custom",
        message: "Shader links cannot contain cycles",
      });
  });
export const shaderInputSchema = z
  .object({
    id: identifier,
    label: z.string().min(1).max(160),
    kind: z.enum(["number", "color", "toggle"]),
    node: z.string().min(1).max(160),
    socket,
    min: finite.optional(),
    max: finite.optional(),
    step: finite.positive().optional(),
  })
  .strict();
export const blenderMaterialSchema = z
  .object({
    id: identifier,
    name: z.string().min(1).max(160),
    preset: z
      .enum(["surface", "noise", "checker", "metal", "glass", "emission"])
      .optional(),
    source: z
      .object({ assetId, material: z.string().min(1).max(160) })
      .strict()
      .optional(),
    graph: shaderGraphSchema.optional(),
    inputs: z.array(shaderInputSchema).max(64).default([]),
    p: z.record(identifier, shaderChannel).default({}),
  })
  .strict()
  .superRefine((shader, ctx) => {
    const ids = new Set<string>();
    for (const field of shader.inputs) {
      if (ids.has(field.id))
        ctx.addIssue({
          code: "custom",
          message: "Material input IDs must be unique",
        });
      ids.add(field.id);
      if (shader.graph && !shader.graph.nodes.some((n) => n.id === field.node))
        ctx.addIssue({
          code: "custom",
          message: "Exposed input must address a shader node",
        });
      const prop = shader.p[field.id];
      if (!prop) {
        ctx.addIssue({
          code: "custom",
          message: `Missing material input ${field.id}`,
        });
        continue;
      }
      const valid = (v: unknown) =>
        field.kind === "color"
          ? typeof v === "string" && /^#[\da-f]{6}$/i.test(v)
          : field.kind === "toggle"
            ? typeof v === "boolean"
            : typeof v === "number" &&
              Number.isFinite(v) &&
              (field.min === undefined || v >= field.min) &&
              (field.max === undefined || v <= field.max);
      if (![prop.v, ...prop.kf.map((k) => k.v)].every(valid))
        ctx.addIssue({
          code: "custom",
          message: `Invalid material input ${field.label}`,
        });
    }
    if (Object.keys(shader.p).some((id) => !ids.has(id)))
      ctx.addIssue({
        code: "custom",
        message: "Unknown exposed material input",
      });
    if (!shader.graph && !shader.source)
      ctx.addIssue({
        code: "custom",
        message: "Material requires a graph or Blender source",
      });
  });
export type BlenderMaterial = z.output<typeof blenderMaterialSchema>;
export type ShaderGraph = z.output<typeof shaderGraphSchema>;
export const render3DSchema = z
  .object({
    enabled: z.boolean().default(false),
    engine: z.enum(["eevee", "cycles"]).default("eevee"),
    samples: z.number().int().min(1).max(4096).default(64),
    previewSamples: z.number().int().min(1).max(512).default(16),
    previewScale: z.number().min(0.1).max(1).default(0.5),
    denoise: z.boolean().default(true),
    device: z.enum(["auto", "cpu", "gpu"]).default("auto"),
  })
  .strict();
export type Render3DSettings = z.output<typeof render3DSchema>;
export const blenderSourceSchema = z
  .object({ assetId, object: z.string().min(1).max(160) })
  .strict();
export const modifierSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("bevel"),
      width: finite.min(0).max(100),
      segments: z.number().int().min(1).max(12).default(3),
    })
    .strict(),
  z
    .object({
      type: z.literal("subdivision"),
      levels: z.number().int().min(0).max(4).default(1),
    })
    .strict(),
  z
    .object({
      type: z.literal("solidify"),
      thickness: finite.min(-100).max(100),
    })
    .strict(),
  z
    .object({
      type: z.literal("array"),
      count: z.number().int().min(1).max(128),
      offset: z.tuple([finite, finite, finite]),
    })
    .strict(),
  z
    .object({
      type: z.literal("mirror"),
      axes: z.tuple([z.boolean(), z.boolean(), z.boolean()]),
    })
    .strict(),
]);
export const RECIPE_PARAMETERS: Record<
  string,
  Record<
    string,
    { value: string | number; min?: number; max?: number; integer?: boolean }
  >
> = {
  rocket: {
    length: { value: 2.8, min: 0.1, max: 50 },
    radius: { value: 0.6, min: 0.05, max: 10 },
    nose: { value: 1, min: 0.05, max: 20 },
    fins: { value: 4, min: 0, max: 16, integer: true },
    finSize: { value: 0.8, min: 0.05, max: 10 },
  },
  staircase: {
    steps: { value: 12, min: 1, max: 128, integer: true },
    radius: { value: 1.5, min: 0.1, max: 30 },
    rise: { value: 0.22, min: 0.01, max: 5 },
    turn: { value: 30, min: -180, max: 180 },
    width: { value: 1.2, min: 0.05, max: 10 },
    depth: { value: 0.5, min: 0.05, max: 10 },
  },
  text: {
    text: { value: "Powermove" },
    size: { value: 1, min: 0.01, max: 50 },
    depth: { value: 0.12, min: 0, max: 5 },
    bevel: { value: 0.02, min: 0, max: 1 },
  },
  lathe: { segments: { value: 48, min: 3, max: 128, integer: true } },
  extrude: { depth: { value: 0.2, min: 0.001, max: 50 } },
  mesh: {},
};
export const recipeSchema = z
  .object({
    kind: z.enum(["rocket", "staircase", "text", "lathe", "extrude", "mesh"]),
    parameters: z
      .record(
        z.string().regex(/^[A-Za-z][A-Za-z0-9_]{0,63}$/),
        z.union([finite.min(-10000).max(10000), z.string().max(2000)]),
      )
      .default({}),
    profile: z
      .array(
        z.tuple([finite.min(-10000).max(10000), finite.min(-10000).max(10000)]),
      )
      .min(2)
      .max(1024)
      .optional(),
    mesh: z
      .object({
        positions: z.array(finite).min(9).max(300000),
        indices: z
          .array(z.number().int().nonnegative())
          .min(3)
          .max(300000)
          .optional(),
      })
      .strict()
      .optional(),
    modifiers: z.array(modifierSchema).max(12).default([]),
  })
  .strict()
  .superRefine((recipe, ctx) => {
    const fields = RECIPE_PARAMETERS[recipe.kind]!;
    for (const [key, value] of Object.entries(recipe.parameters)) {
      const field = fields[key];
      if (!field) {
        ctx.addIssue({
          code: "custom",
          message: `Unknown model parameter: ${key}`,
        });
        continue;
      }
      if (
        typeof value !== typeof field.value ||
        (typeof value === "number" &&
          ((field.min !== undefined && value < field.min) ||
            (field.max !== undefined && value > field.max) ||
            (field.integer && !Number.isInteger(value)))) ||
        (typeof value === "string" && !value.trim())
      )
        ctx.addIssue({
          code: "custom",
          message: `Invalid model parameter: ${key}`,
        });
    }
    if (
      recipe.kind === "lathe" &&
      (!recipe.profile || recipe.profile.some((p) => p[0] < 0))
    )
      ctx.addIssue({
        code: "custom",
        message: "Lathe requires a profile with nonnegative radii",
      });
    if (
      recipe.kind === "extrude" &&
      (!recipe.profile || recipe.profile.length < 3)
    )
      ctx.addIssue({
        code: "custom",
        message: "Extrusion requires three outline points",
      });
    if (
      recipe.kind === "mesh" &&
      recipe.mesh &&
      (recipe.mesh.positions.length % 3 ||
        (recipe.mesh.indices
          ? recipe.mesh.indices.length % 3
          : recipe.mesh.positions.length % 9) ||
        recipe.mesh.indices?.some(
          (i) => i >= recipe.mesh!.positions.length / 3,
        ))
    )
      ctx.addIssue({ code: "custom", message: "Invalid mesh indices" });
    // Reject explosive modifier combinations before bpy allocates their geometry.
    const params = {
      ...Object.fromEntries(
        Object.entries(fields).map(([key, f]) => [key, f.value]),
      ),
      ...recipe.parameters,
    };
    let triangles =
      recipe.kind === "rocket"
        ? 384
        : recipe.kind === "staircase"
          ? 12
          : recipe.kind === "text"
            ? String(params.text).length * 256
            : recipe.kind === "lathe"
              ? (recipe.profile?.length || 0) * Number(params.segments) * 2
              : recipe.kind === "extrude"
                ? (recipe.profile?.length || 0) * 8
                : recipe.mesh
                  ? (recipe.mesh.indices?.length ||
                      recipe.mesh.positions.length / 3) / 3
                  : 0;
    for (const mod of recipe.modifiers) {
      triangles *=
        mod.type === "array"
          ? mod.count
          : mod.type === "mirror"
            ? 2 ** mod.axes.filter(Boolean).length
            : mod.type === "subdivision"
              ? 4 ** mod.levels
              : mod.type === "solidify"
                ? 3
                : 1 + mod.segments * 4;
    }
    const parts =
      recipe.kind === "rocket"
        ? 3 + Number(params.fins)
        : recipe.kind === "staircase"
          ? Number(params.steps)
          : 1;
    if (triangles > 33333 || triangles * parts * 3 > 500000)
      ctx.addIssue({
        code: "custom",
        message:
          "Model and modifiers exceed the geometry budget. Reduce detail or copies.",
      });
  })
  .transform((recipe) => ({
    ...recipe,
    parameters: {
      ...Object.fromEntries(
        Object.entries(RECIPE_PARAMETERS[recipe.kind]!).map(([key, field]) => [
          key,
          field.value,
        ]),
      ),
      ...recipe.parameters,
    },
  }));
export type ModelRecipe = z.output<typeof recipeSchema>;
export const generatedModelSchema = z
  .object({
    version: z.literal(1),
    recipe: recipeSchema,
    p: z.record(identifier, shaderChannel).default({}),
    parts: z.record(
      identifier,
      z
        .object({ layerId: identifier, base: z.record(z.string(), finite) })
        .strict(),
    ),
    retired: z.array(identifier).max(512).default([]),
    sourceAssetId: assetId.optional(),
  })
  .strict()
  .superRefine((model, ctx) => {
    const fields = RECIPE_PARAMETERS[model.recipe.kind]!;
    for (const [key, prop] of Object.entries(model.p)) {
      const field = fields[key];
      if (
        !field ||
        field.integer ||
        typeof field.value !== "number" ||
        ![prop.v, ...prop.kf.map((k) => k.v)].every(
          (v) =>
            typeof v === "number" &&
            (field.min === undefined || v >= field.min) &&
            (field.max === undefined || v <= field.max),
        )
      )
        ctx.addIssue({
          code: "custom",
          message: `Invalid model channel: ${key}`,
        });
    }
  });
export type GeneratedModel = z.output<typeof generatedModelSchema>;
export type BlenderAsset = { id: string; name: string; data: Uint8Array };
export type BlenderJob = {
  id: string;
  operation: "generate" | "import" | "render";
  recipe?: ModelRecipe;
  sourceAssetId?: string;
  snapshot?: unknown;
  assets: BlenderAsset[];
};
export type BlenderOutput = {
  files: Array<{ id: string; name: string; data: Uint8Array }>;
  manifest: any;
};
export type BlenderStatus = {
  available: boolean;
  version?: string;
  path?: string;
  error?: string;
};
