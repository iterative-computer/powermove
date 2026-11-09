import { mapCanvas } from './texture-pixels';
import * as THREE from 'three';
import { packWorld,worldSun } from './environment';
import {expandEnvironmentGeometry,instanceMatrix} from './environment-geometry';
import { MAP_SIZE, extractScene, STUDIO_MATERIAL_FLOATS, STUDIO_TRIANGLE_FLOATS, type StudioScene } from './extract';
import { restoreImportedMaterial } from './imported-material';
import type { StudioView } from './gpu';
import type { StudioCameraDescription, StudioCameraOptions, StudioMaterial, StudioMesh, StudioSceneDescription, StudioTextureSource, StudioVec3 } from '../scene3d/studio-backend';

const vec = (v: ArrayLike<number>, at = 0): StudioVec3 => ({ x: v[at]!, y: v[at + 1]!, z: v[at + 2]! });
const tuple = (v: StudioVec3): [number, number, number] => [v.x, v.y, v.z];

export function describeCamera(view: StudioView, metersPerUnit=0.1): { camera: StudioCameraDescription; options: StudioCameraOptions } {
  const c = view.camera, position = vec(c.position), target = vec(c.position.map((x, i) => x + c.forward[i]! * c.focus));
  const focal = 12 / Math.max(1e-8, c.tanY);
  return {
    camera: { position, target, fov: 2 * Math.atan(c.tanY) * 180 / Math.PI, f_stop: c.lensRadius > 0 ? focal*0.001/(2*metersPerUnit*c.lensRadius) : 0, focus: target, focal_mm: focal },
    options: { projection: c.orthographic ? 'orthographic' : 'perspective', up: vec(c.up), ortho_height: c.orthographic ? c.tanY * 2 : 6, near: c.near ?? .01, far: c.far ?? 1000, iris:c.iris },
  };
}

/** Expensive JSON copies are lazy: native transports normally use the binary fast path. */
export function describePreparedScene(input: StudioScene, view: StudioView, maps: StudioTextureSource[]): StudioSceneDescription {
  const objects: StudioSceneDescription['scene']['objects'] = [], identities: StudioSceneDescription['object_ids'] = [], layers: number[] = [];
  const materialOptions: StudioSceneDescription['material_options'] = [], materials: StudioMaterial[] = [];
  for (let o = 0; o < input.materials.length; o += STUDIO_MATERIAL_FLOATS) {
    const m = input.materials;
    materials.push({ base: vec(m, o), metallic: m[o + 3]!, roughness: m[o + 4]!, coat: m[o + 5]!, coat_roughness: m[o + 6]!, transmission: m[o + 7]!, emission: vec(m, o + 8), screen: false, subsurface: m[o + 11]!, procedural:m[o+27]!, ior: m[o + 12]!, specular: m[o + 13]!, anisotropy: m[o + 15]!, sheen: m[o + 20]!, film_nm: m[o + 21]!, dispersion: m[o + 22]!, droplets: m[o + 23]! });
    materialOptions.push({ opacity: m[o + 14]!, ambient_response: input.materialPolicies?.[o / STUDIO_MATERIAL_FLOATS]?.ambientResponse ?? 1, double_sided: input.materialPolicies?.[o / STUDIO_MATERIAL_FLOATS]?.doubleSided ?? true, unlit:(m[o+25]! & 8)!==0, appears_in_reflections:(m[o+25]! & 32)===0, alpha_test: input.materialPolicies?.[o / STUDIO_MATERIAL_FLOATS]?.alphaTest ?? 0, imported: m[o+55]===1?Array.from(m.subarray(o+32,o+STUDIO_MATERIAL_FLOATS)):undefined, maps: [m[o + 16]!, m[o + 17]!, m[o + 28]!, m[o + 29]!, m[o + 19]!, m[o + 30]!] });
  }
  const coloredMaterials=new Map<string,number>();
  const nodes = new Uint32Array(input.blasNodes), baseNode = input.tlasNodes.byteLength / 64;
  for (let instance = 0; instance < input.instanceCount; instance++) {
    const at = input.lightCount * 16 + instance * 36, d = input.sdata;
    const inverse = new THREE.Matrix4().set(d[at]!, d[at + 1]!, d[at + 2]!, d[at + 3]!, d[at + 4]!, d[at + 5]!, d[at + 6]!, d[at + 7]!, d[at + 8]!, d[at + 9]!, d[at + 10]!, d[at + 11]!, 0, 0, 0, 1);
    const world = inverse.invert(), normal = new THREE.Matrix3().getNormalMatrix(world);
    const groups = new Map<number, StudioMesh>();
    const stack = [d[at + 15]! - baseNode];
    while (stack.length) {
      const node = stack.pop()!;
      for (const side of [0, 8]) {
        const o = node * 16 + side, count = nodes[o + 7]!, start = nodes[o + 3]!;
        if (count === 0xffffffff) continue;
        if (count === 0) { stack.push(start - baseNode); continue; }
        for (let t = start; t < start + count; t++) {
          const originalMaterial = input.tris[t * STUDIO_TRIANGLE_FLOATS + 18]!, layerTag=Math.floor(Math.max(d[at+19]!,0)/2), layer = layerTag>0?layerTag-1:input.tris[t * STUDIO_TRIANGLE_FLOATS + 19]!;
          let material=originalMaterial;
          if((d[at+19]! & 1)!==0){
            const color=[d[at+27]!,d[at+31]!,d[at+35]!],key=`${originalMaterial}:${color.join(',')}`;
            const previous=coloredMaterials.get(key);
            if(previous!==undefined)material=previous;
            else{const m=materials[originalMaterial]!,scale=(v:StudioVec3)=>({x:v.x*color[0]!,y:v.y*color[1]!,z:v.z*color[2]!});material=materials.length;materials.push({...m,base:scale(m.base),emission:materialOptions[originalMaterial]?.unlit?scale(m.emission):m.emission});materialOptions.push({...materialOptions[originalMaterial]!});coloredMaterials.set(key,material);}
          }
          let mesh = groups.get(material);
          if (!mesh) { mesh = { positions: [], normals: [], uvs: [], indices: [] }; groups.set(material, mesh); }
          const index = mesh.positions.length;
          const vertexColors=(input.materials[originalMaterial*STUDIO_MATERIAL_FLOATS+25]! & 16)!==0;
          let identity=identities[objects.findIndex(o=>o.mesh===mesh)];
          for (let j = 0; j < 3; j++) {
            const p = new THREE.Vector3().fromArray(input.positions, t * 12 + j * 4).applyMatrix4(world);
            const n = new THREE.Vector3().fromArray(input.tris, t * STUDIO_TRIANGLE_FLOATS + j * 4).applyMatrix3(normal).normalize();
            mesh.positions.push({ x: p.x, y: p.y, z: p.z }); mesh.normals.push({ x: n.x, y: n.y, z: n.z });
            if(vertexColors){
              const packed=new Uint32Array(input.tris.buffer,input.tris.byteOffset,input.tris.length)[t*STUDIO_TRIANGLE_FLOATS+j*4+3]!;
              if(!identity){identity={layer:input.objects[layer]!,part:input.instancePolicies?.[instance]?.part ?? null,cast_shadow:input.instancePolicies?.[instance]?.castShadow ?? true,receive_shadow:input.instancePolicies?.[instance]?.receiveShadow ?? true,vertex_colors:[]};}
              identity.vertex_colors!.push([packed&255,(packed>>>8)&255,(packed>>>16)&255,(packed>>>24)&255].map(x=>x/255) as [number,number,number,number]);
            }
            mesh.uvs.push([input.tris[t * STUDIO_TRIANGLE_FLOATS + 12 + j * 2]!, input.tris[t * STUDIO_TRIANGLE_FLOATS + 13 + j * 2]!]);
            if(input.tris[t*STUDIO_TRIANGLE_FLOATS+30]!>0){
              identity ??= {layer:input.objects[layer]!,part:input.instancePolicies?.[instance]?.part ?? null,cast_shadow:input.instancePolicies?.[instance]?.castShadow ?? true,receive_shadow:input.instancePolicies?.[instance]?.receiveShadow ?? true};
              (identity.secondary_uvs ??= []).push([input.tris[t*STUDIO_TRIANGLE_FLOATS+24+j*2]!,input.tris[t*STUDIO_TRIANGLE_FLOATS+25+j*2]!]);
            }
          }
          mesh.indices.push([index, index + 1, index + 2]);
          if (mesh.indices.length === 1) {
            objects.push({ mesh, material }); layers.push(layer);
            const meta = input.instancePolicies?.[instance];
            identities.push({...identity ?? { layer: input.objects[layer]!, part: meta?.part ?? null, cast_shadow: meta?.castShadow ?? true, receive_shadow: meta?.receiveShadow ?? true },primary_visible:d[at+23]!<.5});
          }
        }
      }
    }
  }
  const lights: StudioSceneDescription['scene']['lights'] = [], extra: StudioSceneDescription['scene']['extra_lights'] = [];
  const areaOptions: StudioSceneDescription['light_options'] = [], extraOptions: StudioSceneDescription['light_options'] = [];
  const generatedSun=worldSun(input.world??{});
  for (let i = 0; i < input.lightCount; i++) {
    if(generatedSun.length && i===input.lightCount-1 && generatedSun.every((v,j)=>Math.abs(v-input.sdata[i*16+j]!)<=Math.max(1e-6,Math.abs(v)*1e-5)))continue;
    const o = i * 16, d = input.sdata, policy = input.lightPolicies?.[i] ?? { castShadow: true, distance: 0, decay: 2 };
    const options = { cast_shadow: policy.castShadow, distance: policy.distance, decay: policy.decay };
    if (d[o + 3] === 0) { lights.push({ center: vec(d, o), u: vec(d, o + 4), v: vec(d, o + 8), radiance: vec(d, o + 12) }); areaOptions.push(options); }
    else { extra.push({ pos: Array.from(d.subarray(o, o + 4)), u: Array.from(d.subarray(o + 4, o + 8)), v: Array.from(d.subarray(o + 8, o + 12)), radiance: Array.from(d.subarray(o + 12, o + 16)) }); extraOptions.push(options); }
  }
  const { camera, options } = describeCamera(view,input.metersPerUnit??0.1);
  return {
    version: 1, scene: { meters_per_unit:input.metersPerUnit??0.1, world: input.world, objects, materials, lights, extra_lights: extra, layers, camera, backdrop: vec(view.backdrop), environment: [vec(view.environment.zenith), vec(view.environment.horizon)], exposure: view.exposure, shadow_catcher: view.shadowCatcher },
    layer_ids: [...input.objects], object_ids: identities, material_options: materialOptions, light_options: [...areaOptions, ...extraOptions],
    camera_options: options, environment_options: { backdrop_opaque: view.backdropOpaque, shadows: view.shadows, floor_y: view.floorY, ground: vec(view.environment.ground),ambient:vec(view.ambient ?? [0,0,0]) },
    textures: maps.map((source, index) => {
      const pixels = source instanceof Uint8Array ? source : (source.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D)!.getImageData(0, 0, input.mapSize ?? MAP_SIZE, input.mapSize ?? MAP_SIZE).data;
      return { id: `map-${index}`, width: input.mapSize ?? MAP_SIZE, height: input.mapSize ?? MAP_SIZE, rgba: Array.from(pixels) };
    }),
  };
}

export function viewFromDescription(document: StudioSceneDescription, aspect: number): StudioView {
  const s = document.scene, c = s.camera, o = document.camera_options;
  const metres=s.meters_per_unit??0.1;if(!Number.isFinite(metres)||metres<=0)throw new Error('Invalid scene scale');
  const position = new THREE.Vector3(...tuple(c.position)), target = new THREE.Vector3(...tuple(c.target));
  const forward=target.clone().sub(position).normalize(),right=forward.clone().cross(new THREE.Vector3(...tuple(o.up))).normalize();
  if(right.lengthSq()<1e-12){if(Math.abs(forward.y)>.9999)right.set(1,0,0);else right.crossVectors(forward,Math.abs(forward.z)<.9?new THREE.Vector3(0,0,1):new THREE.Vector3(0,1,0)).normalize();}
  const up=right.clone().cross(forward).normalize();
  return {
    camera: { position: tuple(c.position), forward: forward.toArray() as [number, number, number], right: right.toArray() as [number, number, number], up: up.toArray() as [number, number, number], tanY: o.projection === 'orthographic' ? o.ortho_height / 2 : Math.tan(c.fov * Math.PI / 360), aspect, lensRadius: c.f_stop > 0 ? c.focal_mm*0.001/(2*c.f_stop*metres) : 0, focus: c.focus ? position.distanceTo(new THREE.Vector3(...tuple(c.focus))) : position.distanceTo(target), orthographic: o.projection === 'orthographic', near: o.near, far: o.far, iris:o.iris },
    ambient:document.environment_options.ambient?tuple(document.environment_options.ambient):[0,0,0],exposure: s.exposure, backdrop: tuple(s.backdrop), backdropOpaque: document.environment_options.backdrop_opaque, shadowCatcher: s.shadow_catcher, shadows: document.environment_options.shadows, floorY: document.environment_options.floor_y,
    environment: { zenith: tuple(s.environment[0]), horizon: tuple(s.environment[1]), ground: tuple(document.environment_options.ground) },
  };
}

/** Portable JSON entry point, also used to verify the canonical scene contract. */
export function compileSceneDescription(document: StudioSceneDescription): { graph: THREE.Scene; scene: StudioScene; maps: StudioTextureSource[] } {
  if (document.version !== 1) throw new Error('Unsupported Studio scene version');
  document=expandEnvironmentGeometry(document);
  if((document.scene.instances?.length??0)>1000000)throw new Error('At most one million Studio instances');
  for(const instance of document.scene.instances??[]){
    if(!Number.isInteger(instance.object)||!document.scene.objects[instance.object]||instance.layer!=null&&!document.layer_ids[instance.layer]||[instance.position,instance.rotation,instance.scale].some(p=>p&&Object.values(p).some(x=>!Number.isFinite(x)))||instance.scale&&Object.values(instance.scale).some(x=>x<1e-4))throw new Error('Invalid Studio instance');
  }
  const graph = new THREE.Scene();
  const textures = document.textures.map(t => { const texture = new THREE.DataTexture(new Uint8Array(t.rgba), t.width, t.height); texture.flipY = false; return texture; });
  const materials = document.scene.materials.map((m, i) => {
    const p = document.material_options[i];
    const material = new THREE.MeshPhysicalMaterial({ color: new THREE.Color(m.base.x, m.base.y, m.base.z), roughness: m.roughness, metalness: m.metallic, clearcoat: m.coat, clearcoatRoughness: m.coat_roughness, transmission: m.transmission, ior: m.ior, specularIntensity: m.specular, emissive: new THREE.Color(m.emission.x, m.emission.y, m.emission.z), emissiveIntensity: 1, opacity: p?.opacity ?? 1, transparent: (p?.opacity ?? 1) < 1, side: p?.double_sided === false ? THREE.FrontSide : THREE.DoubleSide, sheen: m.sheen, iridescence: m.film_nm > 0 ? 1 : 0, iridescenceThicknessRange: [100, m.film_nm] });
    material.userData = { subsurface: m.subsurface, studioProcedural:m.procedural??0, anisotropy: m.anisotropy, studioDispersion: m.dispersion, droplets: m.droplets, studioUnlit:p?.unlit ?? false, materialAmbientUniform: { value: p?.ambient_response ?? 1 } };
    const slots = ['map', 'emissiveMap', 'roughnessMap', 'metalnessMap', 'normalMap', 'aoMap'] as const;
    slots.forEach((slot, j) => { material[slot] = textures[p?.maps[j] ?? -1] ?? null; });
    restoreImportedMaterial(material,p?.imported,textures);
    return material;
  });
  document.scene.objects.forEach((o, i) => {
    if(o.mesh.colors?.length && o.mesh.colors.length!==o.mesh.positions.length)throw new Error('Invalid Studio vertex colors');
    if(o.mesh.triangle_materials?.length && (o.mesh.triangle_materials.length!==o.mesh.indices.length || o.mesh.triangle_materials.some(m=>!Number.isInteger(m) || m<0 || !materials[m])))throw new Error('Invalid Studio triangle materials');
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.Float32BufferAttribute(o.mesh.positions.flatMap(p => tuple(p)), 3));
    g.setAttribute('normal', new THREE.Float32BufferAttribute(o.mesh.normals.flatMap(p => tuple(p)), 3));
    g.setAttribute('uv', new THREE.Float32BufferAttribute(o.mesh.uvs.flat(), 2)); g.setIndex(o.mesh.indices.flat());
    const secondary=document.object_ids[i]?.secondary_uvs;
    if(secondary)g.setAttribute('uv1',new THREE.Float32BufferAttribute(secondary.flat(),2));
    const colors=document.object_ids[i]?.vertex_colors ?? o.mesh.colors?.map(c=>[c.x,c.y,c.z,1]);
    if(colors)g.setAttribute('color',new THREE.Float32BufferAttribute(colors.flat(),4));
    const material=materials[o.material]!.clone();material.userData.studioReflections=document.material_options[o.material]?.appears_in_reflections!==false;material.vertexColors=!!colors;
    let meshMaterial:THREE.MeshPhysicalMaterial|THREE.MeshPhysicalMaterial[]=material;
    if(o.mesh.triangle_materials?.length){
      meshMaterial=materials.map((m,index)=>{const copy=m.clone();copy.vertexColors=!!colors;copy.userData.studioReflections=document.material_options[index]?.appears_in_reflections!==false;return copy;});material.dispose();
      const faces=o.mesh.triangle_materials;let start=0;
      for(let face=1;face<=faces.length;face++)if(face===faces.length || faces[face]!==faces[start]){g.addGroup(start*3,(face-start)*3,faces[start]);start=face;}
    }
    const placements=document.scene.instances?.length?document.scene.instances.filter(p=>p.object===i):null;
    if(placements?.length===0){g.dispose();for(const m of Array.isArray(meshMaterial)?meshMaterial:[meshMaterial])m.dispose();return;}
    const mesh=placements?new THREE.InstancedMesh(g,meshMaterial,placements.length):new THREE.Mesh(g,meshMaterial);
    if(placements){placements.forEach((p,j)=>(mesh as THREE.InstancedMesh).setMatrixAt(j,instanceMatrix(p)));mesh.userData.studioInstanceLayerNames=placements.map(p=>p.layer==null?null:document.layer_ids[p.layer]);}
    mesh.userData.sceneId = document.object_ids[i]?.layer ?? document.layer_ids[document.scene.layers[i]!] ?? `object-${i}`;
    mesh.userData.studioPart = document.object_ids[i]?.part ?? null;mesh.userData.studioPrimaryVisible=document.object_ids[i]?.primary_visible!==false;
    mesh.castShadow = document.object_ids[i]?.cast_shadow ?? true; mesh.receiveShadow = document.object_ids[i]?.receive_shadow ?? true; graph.add(mesh);
  });
  // Lights are already in the shared GPU layout; replace the extracted light segment below.
  const scene = extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
  const lights = document.scene.lights.flatMap(l => [...tuple(l.center), 0, ...tuple(l.u), 4 * new THREE.Vector3(...tuple(l.u)).cross(new THREE.Vector3(...tuple(l.v))).length(), ...tuple(l.v), 0, ...tuple(l.radiance), 0]);
  lights.push(...document.scene.extra_lights.flatMap(l => [...l.pos, ...l.u, ...l.v, ...l.radiance]));
  lights.push(...worldSun(document.scene.world??{}));
  const trailer=packWorld(document.scene.world??{},lights);
  const data = new Float32Array(lights.length + scene.instanceCount * 36+trailer.length); data.set(lights); data.set(scene.sdata.subarray(scene.lightCount * 16,scene.lightCount*16+scene.instanceCount*36), lights.length); data.set(trailer,lights.length+scene.instanceCount*36);scene.world=document.scene.world;scene.metersPerUnit=document.scene.meters_per_unit??0.1; scene.sdata = data; scene.lightCount = lights.length / 16;
  const maps = scene.maps.map(map => {
    const index = textures.findIndex(t => 'source' in map && t.image === map.source);
    const t = document.textures[index];
    if(!t)return new Uint8Array(MAP_SIZE*MAP_SIZE*4);
    if (t.width === (scene.mapSize ?? MAP_SIZE) && t.height === (scene.mapSize ?? MAP_SIZE)) return new Uint8Array(t.rgba);
    const source = new OffscreenCanvas(t.width, t.height), ctx = source.getContext('2d')!;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(t.rgba), t.width, t.height), 0, 0);
    return mapCanvas(source,false,scene.mapSize ?? MAP_SIZE);
  });
  return { graph, scene, maps };
}
