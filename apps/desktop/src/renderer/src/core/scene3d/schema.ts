import { validateExtrusionChannel } from './extrusion-data';
import { z } from 'zod';
import { layer3DProperties } from './layers';
import { MATERIAL_SLOTS, objectMapSets, mapImageId } from './material-maps';
import { GROUP_PART_KEY, PART_KEY, PART_PATH } from './model-parts';

/** The scene is project data, never executable code or a live Three.js object. */
export const SCENE3D_DEFINITION = 'powermove.3d.scene';
const id = z.string().regex(/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/);
const color = z.string().regex(/^#[0-9a-f]{6}$/i);
const num = (min = -100_000, max = 100_000) => z.number().finite().min(min).max(max);
const channel = (value: z.ZodType) => z.union([value, z.object({
  v: value, kf: z.array(z.object({ t: num(0), v: value }).passthrough()).max(2400),
  expr: z.string().max(2000).nullable().optional()
}).passthrough()]).transform((raw: any) => typeof raw === 'object' && raw !== null && 'v' in raw
  ? { ...raw, expr: raw.expr ?? null } : { v: raw, kf: [], expr: null });
const p = (fields: Record<string, [z.ZodType, unknown]>) => z.object(Object.fromEntries(
  Object.entries(fields).map(([key, [type, fallback]]) => [key, channel(type).default(() => ({ v: fallback, kf: [], expr: null }))])
)).strict();
const transform = {
  x: [num(), 0], y: [num(), 0], z: [num(), 0],
  rx: [num(-360_000, 360_000), 0], ry: [num(-360_000, 360_000), 0], rz: [num(-360_000, 360_000), 0],
  sx: [num(-10000, 10000), 1], sy: [num(-10000, 10000), 1], sz: [num(-10000, 10000), 1],
  visible: [z.boolean(), true]
} satisfies Record<string, [z.ZodType, unknown]>;
const objectTransform = p(transform);
const points2 = z.array(z.tuple([num(), num()])).min(2).max(1024);
const primitiveParams = z.object({
  width: num(.0001, 10000).optional(), height: num(.0001, 10000).optional(), depth: num(.0001, 10000).optional(),
  radius: num(.0001, 10000).optional(), radiusTop: num(0, 10000).optional(), radiusBottom: num(0, 10000).optional(),
  tube: num(.0001, 10000).optional(), segments: z.number().int().min(3).max(128).optional(),
  detail: z.number().int().min(0).max(5).optional()
}).strict();
export const sourceSchema = z.union([
  z.object({ primitive: z.enum(['box','sphere','plane','cylinder','cone','torus','capsule','icosahedron']), parameters: primitiveParams.optional() }).strict(),
  z.object({ assetId: z.string().min(1).max(160) }).strict(),
  z.object({ mesh: z.object({
    positions: z.array(num()).min(9).max(300_000),
    indices: z.array(z.number().int().nonnegative()).min(3).max(300_000).optional(),
    normals: z.array(num(-1, 1)).max(300_000).optional(), uvs: z.array(num()).max(200_000).optional()
  }).strict().superRefine((m, ctx) => {
    const count = m.positions.length / 3;
    if (!Number.isInteger(count)) ctx.addIssue({ code: 'custom', message: 'Positions must contain xyz triples' });
    if (m.indices ? m.indices.length % 3 || m.indices.some(i => i >= count) : count % 3) ctx.addIssue({ code:'custom', message:'Faces must contain valid triangle indices' });
    if (m.normals && m.normals.length !== m.positions.length) ctx.addIssue({ code:'custom', message:'Each vertex needs one normal' });
    if (m.uvs && m.uvs.length !== count * 2) ctx.addIssue({ code:'custom', message:'Each vertex needs one UV pair' });
  }) }).strict(),
  z.object({ lathe: points2, segments: z.number().int().min(3).max(128).optional() }).strict(),
  z.object({ extrude: points2.refine(v => v.length >= 3), depth: num(.0001,10000), bevel: num(0,100).optional() }).strict()
]);
export const materialMapSchema = z.union([z.string().min(1).max(160),z.object({compositionId:z.string().min(1).max(160)}).strict(),z.object({layerId:z.string().min(1).max(160)}).strict()]);
const mapsSchema=z.object(Object.fromEntries(MATERIAL_SLOTS.map(k=>[k,materialMapSchema.optional()]))).strict().superRefine((maps,ctx)=>{
  for(const [slot,map] of Object.entries(maps))if(typeof map==='object' && slot!=='color' && slot!=='emissive')
    ctx.addIssue({code:'custom',message:'Composition and layer textures support base color and emissive slots',path:[slot]});
});
export const materialSchema = z.object({
  p: p({ color: [color,'#C7C4FF'], roughness: [num(.02,1),.4], metalness: [num(0,1),.1],
    emissive: [color,'#000000'], emissiveIntensity: [num(0,100),1], opacity: [num(0,1),1] }).default({} as any),
  maps: mapsSchema.default({}),
  doubleSided: z.boolean().default(true)
}).strict();
/** Sparse overrides preserve imported material values and all old documents. */
export const intrinsicPropertiesSchema = z.object({
  opacity: channel(num(0,100)).optional(), roughness: channel(num(0,1)).optional(), metallic: channel(num(0,1)).optional(),
  emissionColor: channel(color).optional(), emissionIntensity: channel(num(0,100)).optional(), ambientResponse: channel(num(0,100)).optional()
}).strict();
/** Sparse part offsets; absent parts keep the model file's own pose. */
export const partTransformSchema = p({ x:[num(),0], y:[num(),0], z:[num(),0],
  rx:[num(-360_000,360_000),0], ry:[num(-360_000,360_000),0], rz:[num(-360_000,360_000),0],
  scale:[num(0,10000),100], visible:[z.boolean(),true] });
export const partSchema = z.object({ name: z.string().min(1).max(160).default('Part'), p: partTransformSchema.default({} as any),
  members: z.array(z.string().regex(PART_KEY)).min(1).max(4096).optional(), parent: z.string().regex(PART_KEY).nullable().optional(),
  pivot: z.tuple([num(),num(),num()]).optional(), matrix: z.array(num(-1e12,1e12)).length(16).optional()
}).strict();
export const objectSchema = z.object({
  id, name: z.string().min(1).max(160).default('Object'), parent: id.nullable().default(null),
  source: sourceSchema, p: objectTransform.default({} as any), material: materialSchema.default({} as any),
  useSourceMaterials: z.boolean().default(false),
  materialProperties: intrinsicPropertiesSchema.default({}),
  sourceMaterialProperties: z.record(z.string().regex(/^(0|[1-9][0-9]{0,4})$/),intrinsicPropertiesSchema).default({}),
  sourceMaterials: z.record(z.string().regex(/^(0|[1-9][0-9]{0,4})$/),mapsSchema).default({}),
  parts: z.record(z.string().regex(PART_KEY),partSchema).default({}),
  castShadow: z.boolean().default(true), receiveShadow: z.boolean().default(true)
}).strict();
export const lightSchema = z.object({
  id, name: z.string().min(1).max(160).default('Light'), type: z.enum(['sun','point','spot']).default('sun'),
  p: p({ x:[num(),-3], y:[num(),5], z:[num(),4], targetX:[num(),0], targetY:[num(),0], targetZ:[num(),0],
    color:[color,'#FFFFFF'], intensity:[num(0,10000),3], distance:[num(0,100000),0], decay:[num(0,4),2],
    angle:[num(1,89),45], penumbra:[num(0,1),.4], visible:[z.boolean(),true] }).default({} as any),
  castShadow: z.boolean().default(true)
}).strict();
export const cameraSchema = z.object({
  projection: z.enum(['perspective','orthographic']).default('perspective'),
  p: p({ x:[num(),3], y:[num(),2], z:[num(),6], targetX:[num(),0], targetY:[num(),0], targetZ:[num(),0],
    fov:[num(1,150),45], zoom:[num(.001,1000),1], near:[num(.0001,10000),.01], far:[num(.01,100000),1000] }).default({} as any)
}).strict();
export const environmentSchema = z.object({
  p: p({ ambient:[num(0,100),.5], ambientColor:[color,'#FFFFFF'], exposure:[num(.01,20),1], intensity:[num(0,100),1], rotation:[num(-360_000,360_000),0] }).default({} as any),
  map: z.string().min(1).max(160).nullable().default(null),
  background: color.nullable().default(null), shadows: z.boolean().default(true)
}).strict();
const schema = z.object({
  version: z.literal(1).default(1), objects: z.array(objectSchema).max(512).default([]),
  lights: z.array(lightSchema).max(16).default([]), camera: cameraSchema.default({} as any),
  environment: environmentSchema.default({} as any)
}).strict();
export type Scene3D = z.output<typeof schema>;
export type SceneObject = z.output<typeof objectSchema>;
export type SceneLight = z.output<typeof lightSchema>;
export type SceneSource = z.input<typeof sourceSchema>;
export type SceneChannel = {v: any; kf: any[]; expr: string|null};

export function parseScene(raw: unknown): Scene3D {
  // Bound the entire document before expensive validation/allocation.
  if ((JSON.stringify(raw)?.length ?? 0) > 8_000_000) throw new Error('3D scene exceeds the 8 MB geometry budget');
  const scene = schema.parse(raw);
  // Zod's default bypasses transforms; parse nested defaults explicitly.
  scene.objects = scene.objects.map(o => ({...o,p:objectTransform.parse(o.p || {}),
    parts:Object.fromEntries(Object.entries(o.parts || {}).map(([key,part])=>[key,{...part,p:partTransformSchema.parse(part.p || {})}])),material:materialSchema.parse({ ...o.material,p:materialSchema.shape.p.parse(o.material?.p || {}) })}));
  scene.camera = cameraSchema.parse({...scene.camera,p:cameraSchema.shape.p.parse(scene.camera.p || {})});
  scene.environment = environmentSchema.parse({...scene.environment,p:environmentSchema.shape.p.parse(scene.environment.p || {})});
  scene.lights = scene.lights.map(l => lightSchema.parse({...l,p:lightSchema.shape.p.parse(l.p || {})}));
  const ids = new Set<string>(['camera']);
  let vertices = 0;
  for (const node of [...scene.objects,...scene.lights]) {
    if (ids.has(node.id)) throw new Error(`Duplicate 3D id: ${node.id}`);
    ids.add(node.id);
  }
  const byId = new Map(scene.objects.map(o => [o.id,o]));
  for (const object of scene.objects) {
    const owners = new Map<string,string>();
    for (const [key, part] of Object.entries(object.parts)) {
      if (GROUP_PART_KEY.test(key) !== !!part.members || part.members && !part.pivot) throw new Error('Invalid model part group');
      for (const member of part.members || []) {
        if (owners.has(member) || member === key || GROUP_PART_KEY.test(member) && !object.parts[member]?.members) throw new Error('Invalid model part group membership');
        owners.set(member,key);
      }
    }
    for (const [key, part] of Object.entries(object.parts)) if (part.members) {
      const seen = new Set([key]); let parent = owners.get(key) ?? part.parent;
      while (parent) {
        if (seen.has(parent)) throw new Error('Model part groups contain a cycle'); seen.add(parent);
        if(seen.size>64)throw new Error('Model part groups are too deeply nested');
        if (GROUP_PART_KEY.test(parent) && !object.parts[parent]?.members) throw new Error('Missing model part group parent');
        parent = owners.get(parent) ?? object.parts[parent]?.parent;
      }
    }
    const chain = new Set([object.id]);
    let parent = object.parent;
    while (parent) {
      if (chain.has(parent)) throw new Error('3D object parenting contains a cycle');
      chain.add(parent);
      const ancestor = byId.get(parent);
      if (!ancestor) throw new Error(`Missing 3D parent: ${parent}`);
      parent = ancestor.parent;
    }
    if ('primitive' in object.source) {
      const source=object.source,s=source.parameters?.segments || 48;
      vertices+=source.primitive==='box'?24:source.primitive==='plane'?4:source.primitive==='icosahedron'?60*4**(source.parameters?.detail ?? 2)
        :source.primitive==='sphere'||source.primitive==='torus'?(s+1)*(Math.max(8,Math.floor(s/2))+1):(s+1)*12;
    }
    if ('extrude' in object.source) vertices+=object.source.extrude.length*(object.source.bevel?60:12);
    if ('mesh' in object.source) vertices += object.source.mesh.positions.length / 3;
    if ('lathe' in object.source) {
      if (object.source.lathe.some(([r])=>r<0)) throw new Error('Lathe radii cannot be negative');
      vertices += object.source.lathe.length * (object.source.segments || 48);
    }
  }
  if (vertices > 500_000) throw new Error('3D scene exceeds the geometry budget');
  const camera = scene.camera.p as Record<string, SceneChannel>;
  if (camera.far!.v <= camera.near!.v) throw new Error('Camera far plane must be beyond its near plane');
  return scene;
}

export function createObject(id: string, source: SceneSource = {primitive:'box'}, name = 'Object'): SceneObject {
  return parseScene({objects:[{id,source,name}]}).objects[0]!;
}
export function createLight(id: string, type: SceneLight['type'] = 'sun', name = 'Light'): SceneLight {
  return parseScene({lights:[{id,type,name}]}).lights[0]!;
}
export function createScene(): Scene3D {
  return parseScene({ lights:[{ id:'key', name:'Sun', type:'sun' }] });
}

export function sceneProperties(layer: any): any[] {
  const scene = layer.d?.data?.scene;
  if (!scene) return layer3DProperties(layer);
  const out: any[] = [];
  const collect = (node: any,prefix: string,name: string) => {
    for (const [key,prop] of Object.entries<any>(node?.p || {})) if (prop && Array.isArray(prop.kf))
      out.push({key:`${prefix}.${key}`,prop,label:key,group:name});
  };
  for (const o of scene.objects || []) { collect({p:o.materialProperties},`o.${o.id}.mi`,`${o.name} material`);for(const [index,p] of Object.entries(o.sourceMaterialProperties || {}))collect({p},`o.${o.id}.mi.${index}`,`${o.name} material ${index}`); collect(o,`o.${o.id}`,o.name);collect(o.material,`o.${o.id}.m`,`${o.name} material`); }
  for (const l of scene.lights || []) collect(l,`light.${l.id}`,l.name);
  collect(scene.camera,'camera','Camera');collect(scene.environment,'environment','Scene');
  return out;
}

export function validateSceneChannel(path:string,property:unknown):void {
  if(validateExtrusionChannel(path,property))return;
  const part=PART_PATH.exec(path.replace(/^o\.[^.]+\./,''));
  if(part){partTransformSchema.shape[part[2] as keyof typeof partTransformSchema.shape]!.parse(property);return;}
  const intrinsic=/^(?:o\.[^.]+\.)?mi\.(?:[0-9]+\.)?([^.]+)$/.exec(path);
  if(intrinsic){const field=intrinsicPropertiesSchema.shape[intrinsic[1]! as keyof typeof intrinsicPropertiesSchema.shape];if(!field)throw new Error(`Unknown material property: ${path}`);field.unwrap().parse(property);return;}
  if(/^m\.[^.]+$/.test(path)) { const field=materialSchema.shape.p.unwrap().shape[path.slice(2)]; if(!field)throw new Error(`Unknown 3D channel: ${path}`); field.parse(property); return; }
  if(/^(light|camera|environment)\.[^.]+$/.test(path)){
    const [prefix,key]=path.split('.');
    const fields=prefix==='light'?lightSchema.shape.p.unwrap():prefix==='camera'?cameraSchema.shape.p.unwrap():environmentSchema.shape.p.unwrap();
    const field=fields.shape[key!];if(!field)throw new Error(`Unknown 3D channel: ${path}`);field.parse(property);return;
  }
  const match=/^(?:o\.([^.]+)\.(m\.)?|light\.([^.]+)\.|camera\.|environment\.)(.+)$/.exec(path);
  if(!match)return;
  const fields=path.startsWith('o.') ? match[2]?materialSchema.shape.p.unwrap():objectSchema.shape.p.unwrap()
    :path.startsWith('light.')?lightSchema.shape.p.unwrap():path.startsWith('camera.')?cameraSchema.shape.p.unwrap():environmentSchema.shape.p.unwrap();
  const schema=fields.shape[match[4]!];
  if(!schema)throw new Error(`Unknown 3D channel: ${path}`);
  schema.parse(property);
}

export function sceneAssetIds(scene: Scene3D): string[] {
  return [...new Set([...(scene.environment.map?[scene.environment.map]:[]),...scene.objects.flatMap(o => [ ...('assetId' in o.source ? [o.source.assetId] : []), ...objectMapSets(o).flatMap(maps=>Object.values(maps).map(mapImageId)) ].filter((x): x is string => !!x))])];
}
