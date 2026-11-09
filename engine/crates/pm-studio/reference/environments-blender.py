"""Optional Cycles visual reference, run in a separate background Blender.

studio-environments --scene env-room --json-only --bake-sky --out /tmp/pm-shots/studio3d/laneC
Blender --background --factory-startup --threads 2 --python environments-blender.py -- /tmp/pm-shots/studio3d/laneC/env-room-scene.json

Uses the same procedural meshes, linear sky pixels, sun and light power. This
is a visual reference, not pixel parity: Principled BSDF and finite fog bounds
are approximations of Studio's material/medium contract. No .blend is saved.
"""
import bpy, json, math, pathlib, sys, time
from mathutils import Vector, Matrix
path=pathlib.Path(sys.argv[sys.argv.index('--')+1]);data=json.loads(path.read_text());out=path.parent;name=path.name.removesuffix('-scene.json')
if data.get('scatters') or data.get('terrains') or data.get('instances'):
    raise RuntimeError('Export expanded geometry before comparing instanced terrain scenes')
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=64;scene.cycles.use_denoising=True;scene.cycles.max_bounces=8
scene.render.threads_mode='FIXED';scene.render.threads=2
scene.render.resolution_x=960;scene.render.resolution_y=540;scene.render.resolution_percentage=100
try:scene.view_settings.view_transform='Khronos PBR Neutral'
except TypeError:scene.view_settings.view_transform='Standard'
scene.view_settings.exposure=math.log2(data['exposure']);scene.view_settings.gamma=1
vec=lambda p:Vector((p['x'],p['y'],p['z']))
materials=[]
for i,m in enumerate(data['materials']):
    mat=bpy.data.materials.new(f'material-{i}');mat.use_nodes=True;node=mat.node_tree.nodes.get('Principled BSDF')
    node.inputs['Base Color'].default_value=(*vec(m['base']),1);node.inputs['Metallic'].default_value=m['metallic'];node.inputs['Roughness'].default_value=m['roughness'];node.inputs['IOR'].default_value=m['ior']
    node.inputs['Coat Weight'].default_value=m['coat'];node.inputs['Coat Roughness'].default_value=m['coat_roughness'];node.inputs['Transmission Weight'].default_value=m['transmission'];node.inputs['Specular IOR Level'].default_value=.5*m['specular']
    node.inputs['Emission Color'].default_value=(*vec(m['emission']),1);node.inputs['Emission Strength'].default_value=1
    materials.append(mat)
for i,o in enumerate(data['objects']):
    m=o['mesh'];mesh=bpy.data.meshes.new(f'mesh-{i}');mesh.from_pydata([vec(p)for p in m['positions']],[],m['indices']);mesh.update()
    for poly in mesh.polygons:poly.use_smooth=True
    mesh.normals_split_custom_set_from_vertices([vec(p)for p in m['normals']]);obj=bpy.data.objects.new(f'object-{i}',mesh);scene.collection.objects.link(obj);obj.data.materials.append(materials[o['material']])
world=bpy.data.worlds.new('Studio sky');world.use_nodes=True;scene.world=world;nodes=world.node_tree.nodes;nodes.clear();output=nodes.new('ShaderNodeOutputWorld');background=nodes.new('ShaderNodeBackground');texture=nodes.new('ShaderNodeTexEnvironment');texture.image=bpy.data.images.load(str(out/f'{name}-sky.exr'))
# Cycles maps u=.5-atan2(y,x)/(2*pi), v=1-acos(z)/pi.
# Studio maps phi=atan2(X,Z), polar=acos(Y). Transform (-Z,X,Y).
coords=nodes.new('ShaderNodeTexCoord');separate=nodes.new('ShaderNodeSeparateXYZ');combine=nodes.new('ShaderNodeCombineXYZ');negate=nodes.new('ShaderNodeMath');negate.operation='MULTIPLY';negate.inputs[1].default_value=-1
links=world.node_tree.links;links.new(coords.outputs['Generated'],separate.inputs[0]);links.new(separate.outputs['Z'],negate.inputs[0]);links.new(negate.outputs[0],combine.inputs['X']);links.new(separate.outputs['X'],combine.inputs['Y']);links.new(separate.outputs['Y'],combine.inputs['Z']);links.new(combine.outputs[0],texture.inputs['Vector'])
world.node_tree.links.new(texture.outputs['Color'],background.inputs['Color']);world.node_tree.links.new(background.outputs[0],output.inputs['Surface'])
lights=list(data.get('extra_lights',[]));sun=json.loads((out/f'{name}-sun.json').read_text());lights.extend([sun]if sun else [])
for i,l in enumerate(lights):
    kind=int(l['pos'][3]);radiance=Vector(l['radiance'][:3]);peak=max(radiance);light=bpy.data.lights.new(f'light-{i}','SUN'if kind==3 else 'POINT');light.color=radiance/max(peak,1e-12)
    obj=bpy.data.objects.new(f'light-{i}',light);scene.collection.objects.link(obj)
    if kind==3:
        light.angle=2*math.acos(l['u'][0]);light.energy=peak*2*math.pi*(1-l['u'][0]);obj.rotation_euler=(-Vector(l['pos'][:3])).to_track_quat('-Z','Y').to_euler()
    else:
        radius=l['u'][0];light.shadow_soft_size=radius;light.energy=peak*4*math.pi**2*radius**2;obj.location=Vector(l['pos'][:3])
    if hasattr(obj,'visible_camera'):obj.visible_camera=False
def fog_material(label, density, color, anisotropy):
    mat=bpy.data.materials.new(label);mat.use_nodes=True
    n=mat.node_tree.nodes;n.clear();o=n.new('ShaderNodeOutputMaterial')
    scatter=n.new('ShaderNodeVolumeScatter');scatter.inputs['Density'].default_value=density
    scatter.inputs['Color'].default_value=(*vec(color),1)
    scatter.inputs['Anisotropy'].default_value=anisotropy
    absorption=n.new('ShaderNodeVolumeAbsorption');absorption.inputs['Density'].default_value=density
    absorption.inputs['Color'].default_value=(*vec(color),1)
    add=n.new('ShaderNodeAddShader');links=mat.node_tree.links
    links.new(scatter.outputs[0],add.inputs[0]);links.new(absorption.outputs[0],add.inputs[1])
    links.new(add.outputs[0],o.inputs['Volume']);return mat
fog=data['world']['atmosphere']
if fog['height_falloff']!=0:
    raise RuntimeError('This room reference runner requires homogeneous global fog')
if fog['density']>0:
    bpy.ops.mesh.primitive_cube_add(size=2,location=vec(data['camera']['position']))
    obj=bpy.context.object;obj.name='finite atmosphere reference';obj.scale=(fog['distance'],)*3
    obj.data.materials.append(fog_material('atmosphere',fog['density'],fog['color'],fog['anisotropy']))
for i,volume in enumerate(data['world'].get('volumes',[])):
    if volume['shape']=='box':
        bpy.ops.mesh.primitive_cube_add(size=2,location=vec(volume['center']))
        obj=bpy.context.object;obj.scale=vec(volume['size'])
    else:
        bpy.ops.mesh.primitive_uv_sphere_add(segments=32,ring_count=16,radius=volume['size']['x'],location=vec(volume['center']))
        obj=bpy.context.object
    obj.name=f'local fog {i}'
    obj.data.materials.append(fog_material(f'local fog {i}',volume['density'],volume['color'],volume['anisotropy']))
c=data['camera'];camera=bpy.data.cameras.new('camera');obj=bpy.data.objects.new('camera',camera);scene.collection.objects.link(obj);obj.location=vec(c['position']);forward=(vec(c['target'])-obj.location).normalized();right=forward.cross(Vector((0,1,0))).normalized();up=right.cross(forward);obj.rotation_euler=Matrix((right,up,-forward)).transposed().to_euler();camera.type='PERSP';camera.lens_unit='FOV';camera.sensor_fit='VERTICAL';camera.angle=math.radians(c['fov']);camera.clip_end=10000;camera.clip_start=.004;scene.camera=obj
scene.render.image_settings.file_format='PNG';scene.render.filepath=str(out/f'{name}-cycles64.png');start=time.perf_counter();bpy.ops.render.render(write_still=True)
(out/f'{name}-cycles-metrics.json').write_text(json.dumps({'blender':bpy.app.version_string,'engine':'Cycles CPU','threads':2,'size':[960,540],'samples':64,'denoise':True,'seconds':time.perf_counter()-start,'limitations':'Principled material scattering and finite fog bounds differ; visual reference only.'},indent=2))
