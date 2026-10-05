import {
  blenderMaterialSchema,
  type BlenderMaterial,
} from "../../../../shared/blender";
export const MATERIAL_PRESETS = [
  { id: "surface", label: "Surface" },
  { id: "noise", label: "Noise" },
  { id: "checker", label: "Checker" },
  { id: "metal", label: "Brushed metal" },
  { id: "glass", label: "Glass" },
  { id: "emission", label: "Glow" },
] as const;
export function materialPreset(
  preset: BlenderMaterial["preset"] = "surface",
  id = `material_${crypto.randomUUID().replaceAll("-", "")}`,
  name?: string,
): BlenderMaterial {
  const node = {
    id: "surface",
    type: "ShaderNodeBsdfPrincipled" as const,
    inputs: {
      "Base Color": "#FF6B30",
      Roughness: 0.32,
      Metallic: preset === "metal" ? 0.9 : 0.15,
    },
    properties: {},
  };
  const nodes: any[] = [
    node,
    {
      id: "output",
      type: "ShaderNodeOutputMaterial",
      inputs: {},
      properties: {},
    },
  ];
  const links: any[] = [
    { from: "surface", output: "BSDF", to: "output", input: "Surface" },
  ];
  const inputs: any[] = [
    {
      id: "color",
      label: "Color",
      kind: "color",
      node: "surface",
      socket: "Base Color",
    },
    {
      id: "roughness",
      label: "Roughness",
      kind: "number",
      node: "surface",
      socket: "Roughness",
      min: 0,
      max: 1,
      step: 0.01,
    },
    {
      id: "metalness",
      label: "Metallic",
      kind: "number",
      node: "surface",
      socket: "Metallic",
      min: 0,
      max: 1,
      step: 0.01,
    },
  ];
  const values: any = {
    color: "#FF6B30",
    roughness: 0.32,
    metalness: preset === "metal" ? 0.9 : 0.15,
  };
  if (preset === "noise" || preset === "checker") {
    nodes.push({
      id: "texture",
      type: preset === "noise" ? "ShaderNodeTexNoise" : "ShaderNodeTexChecker",
      inputs: { Scale: 5 },
      properties: {},
    });
    nodes.push({
      id: "coordinates",
      type: "ShaderNodeTexCoord",
      inputs: {},
      properties: {},
    });
    links.push({
      from: "coordinates",
      output: "Generated",
      to: "texture",
      input: "Vector",
    });
    if (preset === "checker") {
      nodes.at(-2).inputs.Color1 = "#FF6B30";
      nodes.at(-2).inputs.Color2 = "#13383B";
      links.push({
        from: "texture",
        output: "Color",
        to: "surface",
        input: "Base Color",
      });
      inputs[0].node = "texture";
      inputs[0].socket = "Color1";
      inputs.push({
        id: "color2",
        label: "Second color",
        kind: "color",
        node: "texture",
        socket: "Color2",
      });
      values.color2 = "#13383B";
    } else {
      nodes.push({
        id: "bump",
        type: "ShaderNodeBump",
        inputs: { Strength: 0.25, Distance: 0.08 },
        properties: {},
      });
      links.push(
        { from: "texture", output: "Fac", to: "bump", input: "Height" },
        { from: "bump", output: "Normal", to: "surface", input: "Normal" },
      );
      inputs.push({
        id: "strength",
        label: "Bump strength",
        kind: "number",
        node: "bump",
        socket: "Strength",
        min: 0,
        max: 1,
        step: 0.01,
      });
      values.strength = 0.25;
    }
    inputs.push({
      id: "scale",
      label: "Pattern scale",
      kind: "number",
      node: "texture",
      socket: "Scale",
      min: 0.01,
      max: 1000,
      step: 0.1,
    });
    values.scale = 5;
  }
  if (preset === "glass") {
    node.inputs["Base Color"] = "#D1E9FF";
    (node.inputs as any)["Transmission Weight"] = 1;
    (node.inputs as any).IOR = 1.45;
    values.color = "#D1E9FF";
    values.roughness = 0.1;
    inputs.push(
      {
        id: "transmission",
        label: "Transmission",
        kind: "number",
        node: "surface",
        socket: "Transmission Weight",
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        id: "ior",
        label: "IOR",
        kind: "number",
        node: "surface",
        socket: "IOR",
        min: 1,
        max: 3,
        step: 0.01,
      },
    );
    values.transmission = 1;
    values.ior = 1.45;
  }
  if (preset === "emission") {
    (node.inputs as any)["Emission Color"] = "#FF6B30";
    (node.inputs as any)["Emission Strength"] = 3;
    inputs.push(
      {
        id: "emission",
        label: "Glow color",
        kind: "color",
        node: "surface",
        socket: "Emission Color",
      },
      {
        id: "intensity",
        label: "Glow strength",
        kind: "number",
        node: "surface",
        socket: "Emission Strength",
        min: 0,
        max: 100,
        step: 0.1,
      },
    );
    values.emission = "#FF6B30";
    values.intensity = 3;
  }
  return blenderMaterialSchema.parse({
    id,
    name:
      name ||
      MATERIAL_PRESETS.find((p) => p.id === preset)?.label ||
      "Material",
    preset,
    graph: { nodes, links },
    inputs,
    p: Object.fromEntries(
      Object.entries(values).map(([k, v]) => [k, { v, kf: [], expr: null }]),
    ),
  });
}
/** Source materials expose node group inputs and retain the complete Blender graph. */
export function importedMaterial(
  raw: any,
  sourceAssetId: string,
  id: string,
): BlenderMaterial {
  return blenderMaterialSchema.parse({
    id,
    name: raw.name,
    source: { assetId: sourceAssetId, material: raw.name },
    inputs: raw.inputs.map(({ value, ...input }: any) => input),
    p: Object.fromEntries(
      raw.inputs.map((input: any) => [
        input.id,
        { v: input.value, kf: [], expr: null },
      ]),
    ),
  });
}
export function materialPreview(material: any): any {
  const result = { ...material, p: { ...material.p } },
    shader = material.shader;
  for (const [key, value] of Object.entries<any>(shader?.p || {})) {
    const input = shader.inputs.find((f: any) => f.id === key),
      slot = input?.socket;
    const target =
      slot === "Base Color" || key === "color"
        ? "color"
        : slot === "Roughness" || key === "roughness"
          ? "roughness"
          : slot === "Metallic" || key === "metalness"
            ? "metalness"
            : slot === "Emission Color" || key === "emission"
              ? "emissive"
              : slot === "Emission Strength" || key === "intensity"
                ? "emissiveIntensity"
                : slot === "Alpha"
                  ? "opacity"
                  : null;
    if (target) result.p[target] = value;
  }
  return result;
}

export function materialSlots(
  layer: any,
): Array<{ id: string; name: string; material: any; prefix: string }> {
  const object = layer?.d?.data?.object;
  if (!object) return [];
  return object.slots?.length
    ? object.slots.map((slot: any) => ({ ...slot, prefix: `slots.${slot.id}` }))
    : [{ id: "base", name: "Surface", material: object.material, prefix: "m" }];
}
export function shaderPeers(
  PM: any,
  layer: any,
  path: string,
): Array<{ layer: any; path: string; prop: any }> {
  const match = /^(shader|slots\.([^.]+)\.shader)\.([^.]+)$/.exec(path);
  if (!match) return [];
  const material = match[2]
    ? layer.d.data.object.slots.find((s: any) => s.id === match[2])?.material
    : layer.d.data.object.material;
  const shader = material?.shader;
  if (!shader) return [];
  const layers = PM.ProjectIndex?.allLayers?.() || PM.proj.layers;
  return layers
    .flatMap((candidate: any) =>
      materialSlots(candidate)
        .filter((slot) => slot.material.shader?.id === shader.id)
        .map((slot) => ({
          layer: candidate,
          path:
            slot.id === "base"
              ? `shader.${match[3]}`
              : `${slot.prefix}.shader.${match[3]}`,
          prop: slot.material.shader.p[match[3]!],
        })),
    )
    .filter((peer: any) => peer.prop);
}
/** Material reuse keeps composition timing while every surface owns its key IDs. */
export function assignedMaterial(
  PM: any,
  material: any,
  source: any,
  target: any,
  previous: any,
) {
  const next = JSON.parse(JSON.stringify(material)),
    offset = (source?.from || 0) - (target?.from || 0);
  const same = previous?.shader?.id === next.shader?.id;
  for (const [prefix, channels] of [
    ["p", next.p],
    ["shader", next.shader?.p],
  ] as const) {
    for (const [field, prop] of Object.entries<any>(channels || {})) {
      const prior =
        prefix === "shader"
          ? previous?.shader?.p?.[field]
          : previous?.p?.[field];
      for (const key of prop.kf || []) {
        key.t += offset;
        key.i = same
          ? prior?.kf?.find((old: any) => Math.abs(old.t - key.t) < 1e-8)?.i ||
            PM.uid("k")
          : PM.uid("k");
      }
    }
  }
  return next;
}
export function listMaterials(PM: any) {
  const list = new Map<
    string,
    { id: string; name: string; users: number; material: any }
  >();
  for (const layer of PM.curComp?.().layers || PM.proj.layers)
    for (const slot of materialSlots(layer)) {
      const shader = slot.material.shader;
      if (!shader) continue;
      const current = list.get(shader.id);
      if (current) current.users++;
      else
        list.set(shader.id, {
          id: shader.id,
          name: shader.name,
          users: 1,
          material: slot.material,
        });
    }
  return [...list.values()];
}
