/** Shipped worker code only. Projects supply bounded JSON recipes and node graphs. */
export const BLENDER_WORKER = String.raw`
import bpy, sys, json, math, os, traceback
from mathutils import Matrix, Vector

def emit(data):
    print('PM_BLENDER:'+json.dumps(data), flush=True)

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.preferences.filepaths.use_scripts_auto_execute=False

def color(value):
    if isinstance(value,str):
        rgb=[int(value[i:i+2],16)/255 for i in (1,3,5)]
        return [v/12.92 if v<=0.04045 else ((v+0.055)/1.055)**2.4 for v in rgb]+[1.0]
    return value

def hexcolor(value):
    rgb=[max(0,min(1,v*12.92 if v<=0.0031308 else 1.055*v**(1/2.4)-0.055)) for v in value[:3]]
    return '#'+''.join('%02X'%round(v*255) for v in rgb)

def socket(collection,key):
    if isinstance(key,int) or str(key).isdigit():
        return collection[int(key)]
    result=collection.get(key)
    if result is None: raise ValueError('Unknown shader socket: '+str(key))
    return result

def set_input(node,key,value):
    target=socket(node.inputs,key)
    if not hasattr(target,'default_value'): raise ValueError('Shader input cannot be edited: '+str(key))
    target.default_value=int(round(value)) if target.type=='INT' else color(value)

def import_material(source,assets):
    filename=assets.get(source['assetId'])
    if not filename: raise ValueError('Missing Blender material source')
    name=source['material']
    # Link only data, never startup scripts, handlers or untrusted Python.
    with bpy.data.libraries.load(filename,link=False) as (src,dst):
        if name not in src.materials: raise ValueError('Blender material is missing: '+name)
        dst.materials=[name]
    material=dst.materials[0]
    for image in bpy.data.images:
        if image.source=='FILE' and not image.packed_file:
            # No access to source-machine paths. Imports must carry their media.
            image.filepath=os.path.join(os.path.dirname(filename),'missing-texture.png')
    return material.copy()

def apply_maps(mat,data,assets):
    tree=mat.node_tree
    if not data.get('maps'):return mat
    if data.get('maps'):
        bsdf=next((n for n in tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
        if bsdf is None:raise ValueError('Texture slots require a Principled surface')
        p={key:{'v':value} for key,value in [('color',hexcolor(bsdf.inputs['Base Color'].default_value))]}
    placement=data.get('placement',{})
    coords=tree.nodes.new('ShaderNodeTexCoord');mapping=tree.nodes.new('ShaderNodeMapping')
    mapping.inputs['Scale'].default_value=[placement.get('scaleX',1),placement.get('scaleY',1),1]
    mapping.inputs['Location'].default_value=[placement.get('offsetX',0),placement.get('offsetY',0),0]
    mapping.inputs['Rotation'].default_value=[0,0,placement.get('rotation',0)*math.pi/180]
    tree.links.new(coords.outputs['UV'],mapping.inputs['Vector'])
    for key,asset_id in data.get('maps',{}).items():
        file=assets.get(asset_id)
        if not file:raise ValueError('Missing material texture: '+asset_id)
        image=tree.nodes.new('ShaderNodeTexImage');image.image=bpy.data.images.load(file,check_existing=False)
        if key not in ('color','emissive'):image.image.colorspace_settings.name='Non-Color'
        tree.links.new(mapping.outputs['Vector'],image.inputs['Vector'])
        if key=='normal':
            normal=tree.nodes.new('ShaderNodeNormalMap');tree.links.new(image.outputs['Color'],normal.inputs['Color']);tree.links.new(normal.outputs['Normal'],bsdf.inputs['Normal'])
        elif key=='ao':
            target=bsdf.inputs['Base Color'];mix=tree.nodes.new('ShaderNodeMixRGB');mix.blend_type='MULTIPLY';mix.inputs[0].default_value=1;mix.inputs[1].default_value=target.default_value
            if target.is_linked:tree.links.new(target.links[0].from_socket,mix.inputs[1])
            tree.links.new(image.outputs['Color'],mix.inputs[2]);tree.links.new(mix.outputs[0],target)
        else:
            label={'color':'Base Color','roughness':'Roughness','metalness':'Metallic','emissive':'Emission Color'}[key]
            if key=='color':
                mix=tree.nodes.new('ShaderNodeMixRGB');mix.blend_type='MULTIPLY';mix.inputs[0].default_value=1;mix.inputs[2].default_value=color(p.get('color',{}).get('v','#FFFFFF'));tree.links.new(image.outputs['Color'],mix.inputs[1]);tree.links.new(mix.outputs[0],bsdf.inputs[label])
            else:tree.links.new(image.outputs['Color'],bsdf.inputs[label])
    return mat

def material(data,assets):
    shader=data.get('shader')
    if shader and shader.get('source'):
        mat=import_material(shader['source'],assets)
    else:
        mat=bpy.data.materials.new(shader['name'] if shader else 'Surface')
        mat.use_nodes=True
        tree=mat.node_tree
        if shader and shader.get('graph'):
            tree.nodes.clear()
            for entry in shader['graph']['nodes']:
                node=tree.nodes.new(entry['type']);node.name=entry['id']
                for key,value in entry.get('properties',{}).items(): setattr(node,key,value)
                if entry.get('image'):
                    file=assets.get(entry['image'])
                    if not file: raise ValueError('Missing shader image')
                    node.image=bpy.data.images.load(file,check_existing=False);node.image.colorspace_settings.name=entry.get('colorSpace','sRGB')
                if entry.get('ramp'):
                    if not hasattr(node,'color_ramp'):raise ValueError('Ramp settings require a Color Ramp node')
                    ramp=node.color_ramp;ramp.interpolation=entry['ramp']['interpolation'];stops=sorted(entry['ramp']['stops'],key=lambda stop:stop['position'])
                    while len(ramp.elements)>2:ramp.elements.remove(ramp.elements[-1])
                    for index,stop in enumerate(stops):
                        element=ramp.elements[index] if index<2 else ramp.elements.new(stop['position']);element.position=stop['position'];element.color=color(stop['color'])
                for key,value in entry.get('inputs',{}).items(): set_input(node,key,value)
            for link in shader['graph']['links']:
                tree.links.new(socket(tree.nodes[link['from']].outputs,link['output']),socket(tree.nodes[link['to']].inputs,link['input']))
    tree=mat.node_tree
    if shader:
        for entry in shader.get('inputs',[]):
            node=tree.nodes.get(entry['node'])
            if node is None: raise ValueError('Missing exposed shader node: '+entry['node'])
            value=shader['p'][entry['id']]
            set_input(node,entry['socket'],value['v'] if isinstance(value,dict) else value)
    else:
        bsdf=next(n for n in tree.nodes if n.type=='BSDF_PRINCIPLED')
        p=data.get('p',{})
        for key,label in [('color','Base Color'),('roughness','Roughness'),('metalness','Metallic'),('emissive','Emission Color'),('emissiveIntensity','Emission Strength'),('opacity','Alpha')]:
            if key in p:set_input(bsdf,label,p[key]['v'] if isinstance(p[key],dict) else p[key])
    return apply_maps(mat,data,assets)

def surface(name,base,metal=0,rough=.4):
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    bsdf=mat.node_tree.nodes.get('Principled BSDF');bsdf.inputs['Base Color'].default_value=color(base);bsdf.inputs['Metallic'].default_value=metal;bsdf.inputs['Roughness'].default_value=rough
    return mat

def created(key,name,mat,location=(0,0,0),rotation=(0,0,0)):
    obj=bpy.context.object;obj.name=key;obj['pm_name']=name
    obj.rotation_euler=rotation
    # Bake construction rotation into geometry; layer rotation stays an override.
    bpy.ops.object.transform_apply(location=False,rotation=True,scale=True)
    obj.location=location
    if mat:obj.data.materials.append(mat)
    return obj

def mesh_object(key,name,vertices,faces,mat=None):
    mesh=bpy.data.meshes.new(key);mesh.from_pydata(vertices,[],faces);mesh.update()
    obj=bpy.data.objects.new(key,mesh);bpy.context.collection.objects.link(obj);obj['pm_name']=name
    if mat:mesh.materials.append(mat)
    return obj

def lathe(key,name,profile,segments,mat):
    if any(p[0]<0 for p in profile):raise ValueError('Lathe radii cannot be negative')
    vertices=[(r*math.cos(a*2*math.pi/segments),y,r*math.sin(a*2*math.pi/segments)) for r,y in profile for a in range(segments)]
    faces=[]
    for j in range(len(profile)-1):
        for i in range(segments):faces.append((j*segments+i,j*segments+(i+1)%segments,(j+1)*segments+(i+1)%segments,(j+1)*segments+i))
    return mesh_object(key,name,vertices,faces,mat)

def generate(recipe):
    p=recipe.get('parameters',{});kind=recipe['kind'];parts=[]
    def n(key,default,lo,hi):
        value=p.get(key,default)
        if not isinstance(value,(int,float)) or not lo<=value<=hi:raise ValueError('Invalid '+key)
        return value
    paint=surface('Paint','#FF6B30',.15,.32);metal=surface('Metal','#BEC4CB',.8,.25);accent=surface('Accent','#38A6AA',.25,.32)
    if kind=='rocket':
        radius=n('radius',.6,.05,10);length=n('length',2.8,.1,50);nose=n('nose',1,.05,20);fins=int(n('fins',4,0,16));fin=n('finSize',.8,.05,10)
        bpy.ops.mesh.primitive_cylinder_add(vertices=48,radius=radius,depth=length)
        parts.append(created('body','Body',paint,rotation=(math.pi/2,0,0)))
        bpy.ops.mesh.primitive_cone_add(vertices=48,radius1=radius,radius2=0,depth=nose)
        parts.append(created('nose','Nose',paint,(0,length/2+nose/2,0),(-math.pi/2,0,0)))
        bpy.ops.mesh.primitive_cone_add(vertices=48,radius1=radius*.75,radius2=radius*.55,depth=radius*.7)
        parts.append(created('engine','Engine',metal,(0,-length/2-radius*.35,0),(math.pi/2,0,0)))
        for i in range(fins):
            obj=mesh_object('fin_%02d'%i,'Fin %d'%(i+1),[(radius*.8,-length*.15,-.06),(radius+fin,-length*.55,-.06),(radius*.8,-length*.5,-.06),(radius*.8,-length*.15,.06),(radius+fin,-length*.55,.06),(radius*.8,-length*.5,.06)],[(0,2,1),(3,4,5),(0,1,4,3),(1,2,5,4),(2,0,3,5)],accent)
            obj.data.transform(Matrix.Rotation(i*math.tau/max(1,fins),4,'Y'));parts.append(obj)
    elif kind=='staircase':
        count=int(n('steps',12,1,128));radius=n('radius',1.5,.1,30);rise=n('rise',.22,.01,5);turn=n('turn',30,-180,180);width=n('width',1.2,.05,10);depth=n('depth',.5,.05,10)
        for i in range(count):
            angle=i*turn*math.pi/180
            bpy.ops.mesh.primitive_cube_add(size=1);bpy.context.object.scale=(width,rise*.6,depth)
            parts.append(created('step_%03d'%i,'Step %d'%(i+1),paint,(radius*math.cos(angle),i*rise,radius*math.sin(angle)),(0,-angle,0)))
    elif kind=='text':
        text=p.get('text','Powermove')
        if not isinstance(text,str) or not text or len(text)>2000:raise ValueError('Invalid text')
        curve=bpy.data.curves.new('Text','FONT');curve.body=text;curve.size=n('size',1,.01,50);curve.extrude=n('depth',.12,0,5);curve.bevel_depth=n('bevel',.02,0,1);curve.align_x='CENTER';curve.align_y='CENTER'
        obj=bpy.data.objects.new('text',curve);bpy.context.collection.objects.link(obj);curve.materials.append(paint);obj['pm_name']='Text';bpy.context.view_layer.objects.active=obj;obj.select_set(True);bpy.ops.object.convert(target='MESH');parts.append(bpy.context.object)
    elif kind=='lathe':
        profile=recipe.get('profile');
        if not profile:raise ValueError('Provide a lathe profile')
        parts.append(lathe('model','Lathe',profile,int(n('segments',48,3,128)),paint))
    elif kind=='extrude':
        profile=recipe.get('profile');
        if not profile or len(profile)<3:raise ValueError('Provide an outline with three points')
        curve=bpy.data.curves.new('Outline','CURVE');curve.dimensions='2D';curve.fill_mode='BOTH';curve.extrude=n('depth',.2,.001,50)/2
        spline=curve.splines.new('POLY');spline.points.add(len(profile)-1)
        for point,(x,y) in zip(spline.points,profile):point.co=(x,y,0,1)
        spline.use_cyclic_u=True;obj=bpy.data.objects.new('model',curve);bpy.context.collection.objects.link(obj);curve.materials.append(paint);obj['pm_name']='Extrusion';bpy.context.view_layer.objects.active=obj;obj.select_set(True);bpy.ops.object.convert(target='MESH');parts.append(bpy.context.object)
    elif kind=='mesh':
        m=recipe.get('mesh')
        if not m or len(m['positions'])%3:raise ValueError('Provide xyz mesh vertices')
        vertices=[tuple(m['positions'][i:i+3]) for i in range(0,len(m['positions']),3)];indices=m.get('indices',list(range(len(vertices))))
        if len(indices)%3 or any(i>=len(vertices) for i in indices):raise ValueError('Invalid triangle indices')
        parts.append(mesh_object('model','Model',vertices,[tuple(indices[i:i+3]) for i in range(0,len(indices),3)],paint))
    for obj in parts:
        for spec in recipe.get('modifiers',[]):
            t=spec['type'];mod=obj.modifiers.new(t,{'bevel':'BEVEL','subdivision':'SUBSURF','solidify':'SOLIDIFY','array':'ARRAY','mirror':'MIRROR'}[t])
            if t=='bevel':mod.width=spec['width'];mod.segments=spec['segments']
            elif t=='subdivision':mod.levels=spec['levels'];mod.render_levels=spec['levels']
            elif t=='solidify':mod.thickness=spec['thickness']
            elif t=='array':mod.count=spec['count'];mod.use_relative_offset=False;mod.use_constant_offset=True;mod.constant_offset_displace=spec['offset']
            elif t=='mirror':mod.use_axis=spec['axes']
        if obj.type=='MESH':
            # Generated UVs support immediate image textures on every recipe.
            bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
            if not obj.data.uv_layers:
                bpy.ops.object.mode_set(mode='EDIT');bpy.ops.mesh.select_all(action='SELECT');bpy.ops.uv.smart_project(angle_limit=1.151917);bpy.ops.object.mode_set(mode='OBJECT')
            for polygon in obj.data.polygons:polygon.use_smooth=kind in ('rocket','lathe') and not obj.name.startswith('fin_')
    bpy.context.view_layer.update()
    return parts

def exposed_material(mat,slot):
    result={'id':'slot%d'%slot,'name':mat.name if mat else 'Surface','p':{'color':'#C7C4FF','roughness':.4,'metalness':.1,'emissive':'#000000','emissiveIntensity':1,'opacity':1},'inputs':[]}
    if not mat or not mat.use_nodes:return result
    bsdf=next((n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
    if bsdf:
        for key,label in [('color','Base Color'),('roughness','Roughness'),('metalness','Metallic'),('emissive','Emission Color'),('emissiveIntensity','Emission Strength'),('opacity','Alpha')]:
            s=bsdf.inputs.get(label)
            if s:result['p'][key]=hexcolor(s.default_value) if key in ('color','emissive') else s.default_value
    # Node group interface inputs become native inspector controls.
    candidates=[n for n in mat.node_tree.nodes if n.type=='GROUP'] or ([bsdf] if bsdf else [])
    for node in candidates:
        for index,s in enumerate(node.inputs):
            if s.is_linked or not hasattr(s,'default_value') or len(result['inputs'])>=24:continue
            if node.type=='BSDF_PRINCIPLED' and s.name not in ('Base Color','Roughness','Metallic','Emission Color','Emission Strength','Alpha','Transmission Weight','IOR'):continue
            if s.type not in ('VALUE','RGBA','BOOLEAN','INT'):continue
            val=s.default_value;kind='color' if s.type=='RGBA' else 'toggle' if s.type=='BOOLEAN' else 'number'
            value=hexcolor(val) if kind=='color' else bool(val) if kind=='toggle' else float(val)
            lo=max(-100000,float(getattr(s,'min_value',-100000)));hi=min(100000,float(getattr(s,'max_value',100000)))
            field={'id':'input%d'%len(result['inputs']),'label':s.name,'kind':kind,'node':node.name,'socket':index,'value':value}
            if kind=='number':field.update({'min':min(lo,value),'max':max(hi,value),'step':.01})
            result['inputs'].append(field)
    return result

def describe_objects(objects,source_id,flatten=False):
    parts=[];total=0;deps=bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        if obj.type not in ('MESH','FONT','CURVE','SURFACE','META'):continue
        evaluated=obj.evaluated_get(deps);mesh=evaluated.to_mesh();mesh.calc_loop_triangles()
        try:
            if flatten:
                location,quaternion,scale=obj.matrix_world.decompose();rotation=quaternion.to_euler('ZYX');basis=Matrix.LocRotScale(location,quaternion,scale);mesh.transform(basis.inverted_safe()@obj.matrix_world)
            if len(mesh.loop_triangles)>33333 or not mesh.vertices:raise ValueError('Model exceeds the per-object geometry budget')
            positions=[];normals=[];uvs=[];groups=[];current=None
            triangles=sorted(mesh.loop_triangles,key=lambda t:t.material_index)
            for triangle in triangles:
                if current!=triangle.material_index:
                    current=triangle.material_index;groups.append({'start':len(positions)//3,'count':0,'materialIndex':current})
                for loop_id in triangle.loops:
                    loop=mesh.loops[loop_id];vertex=mesh.vertices[loop.vertex_index];positions.extend(vertex.co);normals.extend(mesh.corner_normals[loop_id].vector if hasattr(mesh,'corner_normals') else loop.normal if hasattr(loop,'normal') else triangle.normal)
                    uvs.extend(mesh.uv_layers.active.data[loop_id].uv if mesh.uv_layers.active else (0,0));groups[-1]['count']+=1
            total+=len(positions)//3
            if total>500000:raise ValueError('Model exceeds 500,000 generated vertices')
            base={'x':obj.location.x,'y':obj.location.y,'z':obj.location.z,'rx':math.degrees(obj.rotation_euler.x),'ry':math.degrees(obj.rotation_euler.y),'rz':math.degrees(obj.rotation_euler.z),'sx':obj.scale.x,'sy':obj.scale.y,'sz':obj.scale.z}
            if flatten:base={'x':location.x,'y':location.y,'z':location.z,'rx':math.degrees(rotation.x),'ry':math.degrees(rotation.y),'rz':math.degrees(rotation.z),'sx':scale.x,'sy':scale.y,'sz':scale.z}
            mats=[exposed_material(m,i) for i,m in enumerate(obj.data.materials)]
            parts.append({'key':obj.name,'name':obj.get('pm_name',obj.name),'mesh':{'positions':positions,'normals':normals,'uvs':uvs,'groups':groups},'base':base,'materials':mats,'blender':{'assetId':source_id,'object':obj.name}})
        finally:evaluated.to_mesh_clear()
    if not parts:raise ValueError('No supported model objects found')
    return parts

def build_snapshot(req):
    s=req['snapshot'];assets=req['assets'];scene=bpy.context.scene;settings=s['settings']
    # Blender version-specific engine IDs are discovered, not guessed.
    if settings['engine']=='cycles':scene.render.engine='CYCLES'
    else:
        try:scene.render.engine='BLENDER_EEVEE'
        except TypeError:scene.render.engine='BLENDER_EEVEE_NEXT'
    scene.render.resolution_x=s['width'];scene.render.resolution_y=s['height'];scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True
    scene.view_settings.view_transform='AgX';scene.view_settings.exposure=math.log2(max(.01,s.get('environment',{}).get('exposure',1)))
    if settings['engine']=='cycles':
        scene.cycles.samples=settings['samples'];scene.cycles.use_denoising=settings['denoise'];scene.cycles.device='CPU'
        if settings.get('device') in ('auto','gpu'):
            try:
                prefs=bpy.context.preferences.addons['cycles'].preferences
                prefs.compute_device_type='METAL' if sys.platform=='darwin' else 'CUDA';prefs.get_devices();devices=[d for d in prefs.devices if d.type!='CPU']
                if not devices and settings['device']=='gpu':raise ValueError('No supported Cycles GPU found')
                for d in prefs.devices:d.use=d.type!='CPU'
                if devices:scene.cycles.device='GPU'
            except Exception:
                if settings['device']=='gpu':raise
    elif hasattr(scene,'eevee'):
        if hasattr(scene.eevee,'taa_render_samples'):scene.eevee.taa_render_samples=settings['samples']
        if hasattr(scene.eevee,'taa_samples'):scene.eevee.taa_samples=settings['samples']
    env=s.get('environment',{});world=bpy.data.worlds.new('Environment');world.use_nodes=True;scene.world=world
    world.node_tree.nodes['Background'].inputs['Color'].default_value=color(env.get('ambientColor','#FFFFFF'));world.node_tree.nodes['Background'].inputs['Strength'].default_value=env.get('ambient',.5)
    if s.get('background'):
        tree=world.node_tree;ambient=tree.nodes['Background'];background=tree.nodes.new('ShaderNodeBackground');background.inputs['Color'].default_value=color(s['background']);background.inputs['Strength'].default_value=1;ray=tree.nodes.new('ShaderNodeLightPath');mix=tree.nodes.new('ShaderNodeMixShader');tree.links.new(ray.outputs['Is Camera Ray'],mix.inputs[0]);tree.links.new(ambient.outputs[0],mix.inputs[1]);tree.links.new(background.outputs[0],mix.inputs[2]);tree.links.new(mix.outputs[0],tree.nodes['World Output'].inputs['Surface'])
    cam=s['camera'];data=bpy.data.cameras.new('Camera');obj=bpy.data.objects.new('Camera',data);scene.collection.objects.link(obj);scene.camera=obj
    obj.matrix_world=Matrix([cam['matrix'][i::4] for i in range(4)])
    if cam.get('orthographic'):
        data.type='ORTHO';data.ortho_scale=cam['orthoHeight']/cam['zoom']
    else:
        data.type='PERSP';data.sensor_fit='VERTICAL';data.sensor_height=24;data.lens=12*cam['zoom']/math.tan(cam['fov']*math.pi/360)
    data.clip_start=cam['near'];data.clip_end=cam['far']
    for light in s['lights']:
        data=bpy.data.lights.new(light['id'],{'sun':'SUN','point':'POINT','spot':'SPOT','area':'AREA'}[light['type']]);obj=bpy.data.objects.new(light['id'],data);scene.collection.objects.link(obj);obj.location=light['position'];data.color=color(light['color'])[:3]
        data.energy=light['intensity']*(1 if light['type']=='sun' else 4*math.pi)
        if hasattr(data,'use_shadow'):data.use_shadow=light.get('castShadow',True)
        if light['type'] in ('point','spot') and light.get('distance',0)>0:
            data.use_custom_distance=True;data.cutoff_distance=light['distance']
        if light['type']=='sun':data.angle=.08
        if light['type']=='spot':data.spot_size=light.get('angle',45)*math.pi/180*2;data.spot_blend=light.get('penumbra',.4)
        if light['type']=='area':data.shape='RECTANGLE';data.size=light.get('width',1);data.size_y=light.get('height',1)
        if light['type'] in ('sun','spot','area'):
            direction=Vector(light.get('target',[0,0,0]))-obj.location
            if direction.length>1e-9:obj.rotation_euler=direction.to_track_quat('-Z','Y').to_euler()
    layers={};material_cache={};generated_cache={}
    for layer in s['objects']:
        generated=layer.get('generated')
        if not generated:continue
        signature=json.dumps(generated['recipe'],sort_keys=True)
        if signature not in generated_cache:
            objs=generate(generated['recipe']);parts=describe_objects(objs,'generated');generated_cache[signature]={part['key']:part for part in parts}
            for obj in objs:bpy.data.objects.remove(obj,do_unlink=True)
        part=generated_cache[signature].get(generated['part'])
        if part:
            geometry=part['mesh'];groups=geometry['groups']
            for m in layer['meshes']:
                group=next((g for g in groups if g['materialIndex']==m.get('materialIndex',0)),None)
                if not group:continue
                a=group['start'];b=a+group['count'];m['positions']=geometry['positions'][a*3:b*3];m['normals']=geometry['normals'][a*3:b*3];m['uvs']=geometry['uvs'][a*2:b*2]

    for layer in s['objects']:
        objects=[]
        for index,m in enumerate(layer['meshes']):
            vertices=[m['positions'][i:i+3] for i in range(0,len(m['positions']),3)];faces=[(i,i+1,i+2) for i in range(0,len(vertices),3)]
            obj=mesh_object(layer['id']+'_'+str(index),layer['id'],vertices,faces)
            if m.get('matrix'):obj.matrix_world=Matrix([m['matrix'][i::4] for i in range(4)])
            uv=obj.data.uv_layers.new()
            for loop in obj.data.loops:uv.data[loop.index].uv=m['uvs'][loop.vertex_index*2:loop.vertex_index*2+2]
            for face in obj.data.polygons:face.use_smooth=m.get('smooth',True)
            if m.get('normals') and len(m['normals'])==len(m['positions']):
                try:obj.data.normals_split_custom_set([m['normals'][i:i+3] for i in range(0,len(m['normals']),3)])
                except Exception:pass
            mat_key=json.dumps(m['material'],sort_keys=True)
            if mat_key not in material_cache:material_cache[mat_key]=material(m['material'],assets)
            obj.data.materials.append(material_cache[mat_key]);obj.visible_shadow=layer.get('castShadow',True)
            objects.append(obj)
        layers[layer['id']]=objects
    scene.render.use_file_extension=True
    return layers

def execute(req):
    reset();operation=req['operation'];directory=req['dir'];files=[]
    if operation in ('generate','import'):
        source_id='source'
        if operation=='generate':objects=generate(req['recipe']);source='model.blend'
        else:
            filename=req['assets'].get(req['sourceAssetId'])
            if not filename or not filename.lower().endswith('.blend'):raise ValueError('Choose a .blend source')
            bpy.ops.wm.open_mainfile(filepath=filename,load_ui=False,use_scripts=False)
            bpy.context.preferences.filepaths.use_scripts_auto_execute=False
            for image in bpy.data.images:
                if image.source=='FILE' and not image.packed_file:raise ValueError('Pack external textures in Blender before importing this file')
            objects=list(bpy.context.scene.objects);source='imported.blend'
        parts=describe_objects(objects,source_id,flatten=operation=='import')
        # Pack images and save a versioned source; the original is never changed.
        bpy.ops.file.pack_all();bpy.ops.wm.save_as_mainfile(filepath=os.path.join(directory,source),check_existing=False)
        files.append({'id':source_id,'name':source});manifest={'parts':parts,'files':files}
    elif operation=='render':
        layers=build_snapshot(req);blocks=req['snapshot'].get('blocks',{});outputs=req['snapshot'].get('outputs',list(layers))
        shadows={obj:obj.visible_shadow for objects in layers.values() for obj in objects};light_shadows={light:light.use_shadow for light in bpy.data.lights if hasattr(light,'use_shadow')}
        for layer_id in outputs:
            if layer_id not in layers:continue
            block=blocks.get(layer_id,list(layers));receive=next((obj.get('receiveShadow',True) for obj in req['snapshot']['objects'] if obj['id']==layer_id),True)
            for light,shadow in light_shadows.items():light.use_shadow=shadow and receive
            for other_id,objects in layers.items():
                for obj in objects:
                    obj.hide_render=other_id not in block;obj.visible_shadow=shadows[obj] and receive
                    obj.is_holdout=other_id!=layer_id
            bpy.context.scene.render.film_transparent=not bool(req['snapshot'].get('background') and layer_id in req['snapshot'].get('backgroundFor',[]))
            # Holdouts preserve depth and inter-object shadows in the layer stack.
            filename=layer_id+'.png';bpy.context.scene.render.filepath=os.path.join(directory,filename);bpy.ops.render.render(write_still=True)
            files.append({'id':layer_id,'name':filename})
        scene=bpy.context.scene
        manifest={'files':files,'engine':req['snapshot']['settings']['engine'],'renderEngine':scene.render.engine}
        if scene.render.engine=='CYCLES':manifest.update(samples=scene.cycles.samples,device=scene.cycles.device)
    else:raise ValueError('Unsupported Blender operation')
    with open(os.path.join(directory,'manifest.json'),'w') as file:json.dump(manifest,file)

for line in sys.stdin:
    try:
        envelope=json.loads(line)
        with open(envelope['path']) as file:request=json.load(file)
        execute(request);emit({'id':envelope['id'],'ok':True})
    except Exception as error:
        traceback.print_exc(file=sys.stderr);emit({'id':locals().get('envelope',{}).get('id','unknown'),'ok':False,'error':str(error)})
`;
