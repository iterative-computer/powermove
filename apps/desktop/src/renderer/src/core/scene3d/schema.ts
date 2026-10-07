import { z } from 'zod';
import { blenderMaterialSchema,blenderSourceSchema,identifier } from '../../../../shared/blender';
import { layer3DProperties } from './layers';

/** The scene is project data, never executable code or a live Three.js object. */
export const SCENE3D_DEFINITION = 'powermove.3d.scene';
const id = z.string().regex(/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/);
const color = z.string().regex(/^#[0-9a-f]{6}$/i);
const num = (min = -100_000, max = 100_000) => z.number().finite().min(min).max(max);
const channel = (value: z.ZodType, minTime = 0) => z.union([value, z.object({
  v: value, kf: z.array(z.object({ t: num(minTime), v: value }).passthrough()).max(2400),
  expr: z.string().max(2000).nullable().optional()
}).passthrough()]).transform((raw: any) => typeof raw === 'object' && raw !== null && 'v' in raw
  ? { ...raw, expr: raw.expr ?? null } : { v: raw, kf: [], expr: null });
const p = (fields: Record<string, [z.ZodType, unknown]>, minTime = 0) => z.object(Object.fromEntries(
  Object.entries(fields).map(([key, [type, fallback]]) => [key, channel(type, minTime).default(() => ({ v: fallback, kf: [], expr: null }))])
)).strict();
const transform = {
  x: [num(), 0], y: [num(), 0], z: [num(), 0],
  rx: [num(-360_000, 360_000), 0], ry: [num(-360_000, 360_000), 0], rz: [num(-360_000, 360_000), 0],
  sx: [num(-10000, 10000), 1], sy: [num(-10000, 10000), 1], sz: [num(-10000, 10000), 1],
  visible: [z.boolean(), true]
} satisfies Record<string, [z.ZodType, unknown]>;
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
    groups:z.array(z.object({start:z.number().int().nonnegative(),count:z.number().int().nonnegative(),materialIndex:z.number().int().min(0).max(63)}).strict()).max(64).optional(),
    normals: z.array(num(-1, 1)).max(300_000).optional(), uvs: z.array(num()).max(200_000).optional()
  }).strict().superRefine((m, ctx) => {
    const count = m.positions.length / 3;
    if (!Number.isInteger(count)) ctx.addIssue({ code: 'custom', message: 'Positions must contain xyz triples' });
    if (m.indices ? m.indices.length % 3 || m.indices.some(i => i >= count) : count % 3) ctx.addIssue({ code:'custom', message:'Faces must contain valid triangle indices' });
    if (m.normals && m.normals.length !== m.positions.length) ctx.addIssue({ code:'custom', message:'Each vertex needs one normal' });
    if(m.groups?.some(g=>g.start+g.count>(m.indices?.length ?? count)||g.start%3||g.count%3))ctx.addIssue({code:'custom',message:'Invalid material face ranges'});
    if (m.uvs && m.uvs.length !== count * 2) ctx.addIssue({ code:'custom', message:'Each vertex needs one UV pair' });
  }) }).strict(),
  z.object({ lathe: points2, segments: z.number().int().min(3).max(128).optional() }).strict(),
  z.object({ extrude: points2.refine(v => v.length >= 3), depth: num(.0001,10000), bevel: num(0,100).optional() }).strict()
]);
export const materialSchema = z.object({
  shader:blenderMaterialSchema.optional(),
  placement:z.object({scaleX:num(.001,1000).default(1),scaleY:num(.001,1000).default(1),offsetX:num().default(0),offsetY:num().default(0),rotation:num(-360000,360000).default(0)}).strict().optional(),
  p: p({ color: [color,'#C7C4FF'], roughness: [num(0,1),.4], metalness: [num(0,1),.1],
    emissive: [color,'#000000'], emissiveIntensity: [num(0,100),1], opacity: [num(0,1),1] }, -100000000).default({} as any),
  maps: z.object(Object.fromEntries(['color','normal','roughness','metalness','emissive','ao'].map(k => [k,z.string().min(1).max(160).optional()]))).strict().default({}),
  doubleSided: z.boolean().default(true)
}).strict();
export const objectSchema = z.object({
  id, name: z.string().min(1).max(160).default('Object'), parent: id.nullable().default(null),
  source: sourceSchema, p: p(transform).default({} as any), material: materialSchema.default({} as any),
  useSourceMaterials: z.boolean().default(false),
  blender:blenderSourceSchema.optional(),
  generation:z.object({groupId:identifier,partId:identifier}).strict().optional(),
  slots:z.array(z.object({id:identifier,name:z.string().min(1).max(160),material:materialSchema}).strict()).max(64).default([]),
  castShadow: z.boolean().default(true), receiveShadow: z.boolean().default(true)
}).strict();
export const lightSchema = z.object({
  id, name: z.string().min(1).max(160).default('Light'), type: z.enum(['sun','point','spot','area']).default('sun'),
  p: p({ x:[num(),-3], y:[num(),5], z:[num(),4], targetX:[num(),0], targetY:[num(),0], targetZ:[num(),0],
    color:[color,'#FFFFFF'], intensity:[num(0,10000),3], distance:[num(0,100000),0], decay:[num(0,4),2],
    angle:[num(1,89),45], penumbra:[num(0,1),.4], width:[num(.001,10000),1], height:[num(.001,10000),1], visible:[z.boolean(),true] }).default({} as any),
  castShadow: z.boolean().default(true)
}).strict();
export const cameraSchema = z.object({
  projection: z.enum(['perspective','orthographic']).default('perspective'),
  p: p({ x:[num(),3], y:[num(),2], z:[num(),6], targetX:[num(),0], targetY:[num(),0], targetZ:[num(),0],
    fov:[num(1,150),45], zoom:[num(.001,1000),1], near:[num(.0001,10000),.01], far:[num(.01,100000),1000] }).default({} as any)
}).strict();
export const environmentSchema = z.object({
  p: p({ ambient:[num(0,100),.5], ambientColor:[color,'#FFFFFF'], exposure:[num(.01,20),1] }).default({} as any),
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
  scene.objects = scene.objects.map(o => ({...o,slots:(o.slots || []).map(slot=>({...slot,material:materialSchema.parse({...slot.material,p:materialSchema.shape.p.parse(slot.material?.p || {})})})),p:p(transform).parse(o.p || {}),material:materialSchema.parse({ ...o.material,p:materialSchema.shape.p.parse(o.material?.p || {}) })}));
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
  for (const o of scene.objects || []) { collect(o,`o.${o.id}`,o.name);collect(o.material,`o.${o.id}.m`,`${o.name} material`); }
  for (const l of scene.lights || []) collect(l,`light.${l.id}`,l.name);
  collect(scene.camera,'camera','Camera');collect(scene.environment,'environment','Scene');
  return out;
}

export function validateSceneChannel(path:string,property:unknown):void {
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
  return [...new Set(scene.objects.flatMap(o => [ ...('assetId' in o.source ? [o.source.assetId] : []), ...Object.values(o.material.maps) ].filter((x): x is string => !!x)))];
}
