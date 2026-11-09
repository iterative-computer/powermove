"""TEST ONLY. Run by run.py inside headless Blender; never imported by Powermove."""
import json
import math
import pathlib
import sys
import time

import bpy
import numpy as np
from mathutils import Matrix, Vector


def vector(p):
    return Vector((p['x'], p['y'], p['z']))


def scalar(nodes, op, a=None, b=None):
    n = nodes.new('ShaderNodeMath')
    n.operation = op
    if a is not None:
        n.inputs[0].default_value = a
    if b is not None:
        n.inputs[1].default_value = b
    return n


def build(out, samples):
    data = json.loads((out / 'scene.json').read_text())
    meta = json.loads((out / 'manifest.json').read_text())
    scale = data['meters_per_unit']
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    prefs = bpy.context.preferences.addons['cycles'].preferences
    prefs.compute_device_type = 'METAL'
    prefs.get_devices()
    devices = [d for d in prefs.devices if d.type == 'METAL']
    if not devices:
        raise RuntimeError('Cycles Metal GPU unavailable; no silent CPU fallback')
    for device in prefs.devices:
        device.use = device.type == 'METAL'
    scene.cycles.device = 'GPU'
    scene.cycles.samples = samples
    scene.cycles.seed = 17
    scene.cycles.use_adaptive_sampling = False
    scene.cycles.use_denoising = False
    scene.cycles.max_bounces = 8
    scene.cycles.diffuse_bounces = 8
    scene.cycles.glossy_bounces = 8
    scene.cycles.transmission_bounces = 8
    scene.cycles.volume_bounces = 8
    scene.cycles.transparent_max_bounces = 8
    scene.cycles.sample_clamp_direct = 0
    scene.cycles.sample_clamp_indirect = 0
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    scene.render.resolution_x, scene.render.resolution_y = meta['size']
    scene.render.resolution_percentage = 100
    scene.render.filter_size = 0.01  # Studio uses box/pixel jitter, not a wide reconstruction filter.
    scene.render.film_transparent = data['shadow_catcher'] or not meta['map']['visible']
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1
    warnings = []
    screen = bpy.data.images.load(str(out / 'screen.png'))
    materials = []
    for i, m in enumerate(data['materials']):
        mat = bpy.data.materials.new(f'material-{i}')
        mat.use_nodes = True
        n = mat.node_tree.nodes.get('Principled BSDF')
        n.inputs['Base Color'].default_value = (*vector(m['base']), 1)
        for source, dest in [('metallic', 'Metallic'), ('roughness', 'Roughness'),
                             ('ior', 'IOR'), ('coat', 'Coat Weight'),
                             ('coat_roughness', 'Coat Roughness'),
                             ('transmission', 'Transmission Weight'), ('sheen', 'Sheen Weight')]:
            n.inputs[dest].default_value = m[source]
        n.inputs['Specular IOR Level'].default_value = m['specular'] * 0.5
        n.inputs['Coat IOR'].default_value = 1.5
        n.inputs['Sheen Roughness'].default_value = m['roughness']
        n.inputs['Sheen Tint'].default_value = (1, 1, 1, 1)
        n.inputs['Anisotropic'].default_value = abs(m['anisotropy'])
        n.inputs['Anisotropic Rotation'].default_value = 0.25 if m['anisotropy'] < 0 else 0
        if m['film_nm']:
            if 'Thin Film Thickness' in n.inputs:
                n.inputs['Thin Film Thickness'].default_value = m['film_nm']
                n.inputs['Thin Film IOR'].default_value = 1.38
            warnings.append(f'material {i}: RGB cosine film differs from Cycles interference')
        if m['dispersion']:
            warnings.append(f'material {i}: Cycles Principled has no matching RGB dispersion; IOR is central channel')
        if m['droplets']:
            warnings.append(f'material {i}: procedural Studio droplet normals unsupported by Principled mapping')
        if m['subsurface']:
            n.inputs['Subsurface Weight'].default_value = 1
            n.inputs['Subsurface Scale'].default_value = m['subsurface'] * scale
            n.inputs['Subsurface Radius'].default_value = (1, 1, 1)
            warnings.append(f'material {i}: isotropic Studio random walk differs from Cycles subsurface')
        n.inputs['Emission Color'].default_value = (*vector(m['emission']), 1)
        n.inputs['Emission Strength'].default_value = 1
        if m['screen']:
            tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
            tex.image = screen
            tex.interpolation = 'Linear'
            mul = mat.node_tree.nodes.new('ShaderNodeVectorMath')
            mul.operation = 'MULTIPLY'
            mul.inputs[1].default_value = vector(m['emission'])
            mat.node_tree.links.new(tex.outputs['Color'], mul.inputs[0])
            mat.node_tree.links.new(mul.outputs['Vector'], n.inputs['Emission Color'])
        materials.append(mat)

    meshes = []
    for i, obj in enumerate(data['objects']):
        m = obj['mesh']
        mesh = bpy.data.meshes.new(f'mesh-{i}')
        mesh.from_pydata([vector(p) * scale for p in m['positions']], [], m['indices'])
        mesh.update()
        mesh.polygons.foreach_set('use_smooth', [True] * len(mesh.polygons))
        mesh.normals_split_custom_set_from_vertices([vector(p) for p in m['normals']])
        uv = mesh.uv_layers.new(name='UVMap')
        # Studio's first texture row is UV v=0; Blender's images start at the bottom.
        uv.data.foreach_set('uv', [c for face in m['indices'] for idx in face
                                  for c in (m['uvs'][idx][0], 1 - m['uvs'][idx][1])])
        mesh.materials.append(materials[obj['material']])
        meshes.append(mesh)

    instances = data.get('instances') or [dict(object=i) for i in range(len(meshes))]
    for i, inst in enumerate(instances):
        obj = bpy.data.objects.new(f'object-{i}', meshes[inst['object']])
        scene.collection.objects.link(obj)
        obj.location = vector(inst.get('position', dict(x=0, y=0, z=0))) * scale
        obj.rotation_euler = tuple(math.radians(v) for v in vector(inst.get('rotation', dict(x=0, y=0, z=0))))
        obj.scale = vector(inst.get('scale', dict(x=1, y=1, z=1)))

    world = bpy.data.worlds.new('Studio world')
    world.use_nodes = True
    scene.world = world
    nodes, links = world.node_tree.nodes, world.node_tree.links
    nodes.clear()
    output = nodes.new('ShaderNodeOutputWorld')
    background = nodes.new('ShaderNodeBackground')
    background.inputs['Strength'].default_value = meta['map']['intensity']
    texture = nodes.new('ShaderNodeTexEnvironment')
    texture.image = bpy.data.images.load(str(out / 'world.exr'))
    texture.interpolation = 'Linear'
    coords = nodes.new('ShaderNodeTexCoord')
    rotate = nodes.new('ShaderNodeVectorRotate')
    rotate.rotation_type = 'AXIS_ANGLE'
    rotate.inputs['Axis'].default_value = (0, 1, 0)
    rotate.inputs['Angle'].default_value = -math.radians(meta['map']['rotation'])
    links.new(coords.outputs['Generated'], rotate.inputs['Vector'])
    separate = nodes.new('ShaderNodeSeparateXYZ')
    combine = nodes.new('ShaderNodeCombineXYZ')
    negate = scalar(nodes, 'MULTIPLY', b=-1)
    links.new(rotate.outputs['Vector'], separate.inputs[0])
    links.new(separate.outputs['Z'], negate.inputs[0])
    links.new(negate.outputs[0], combine.inputs['X'])
    links.new(separate.outputs['X'], combine.inputs['Y'])
    links.new(separate.outputs['Y'], combine.inputs['Z'])
    links.new(combine.outputs[0], texture.inputs['Vector'])
    links.new(texture.outputs['Color'], background.inputs['Color'])
    links.new(background.outputs[0], output.inputs['Surface'])

    def emitter(label, mesh, radiance, rim=False, spot=None):
        mat = bpy.data.materials.new(label)
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        nodes.clear()
        output = nodes.new('ShaderNodeOutputMaterial')
        emit = nodes.new('ShaderNodeEmission')
        emit.inputs['Color'].default_value = (*radiance, 1)
        geometry = nodes.new('ShaderNodeNewGeometry')
        if rim:
            uv = nodes.new('ShaderNodeTexCoord')
            sep = nodes.new('ShaderNodeSeparateXYZ')
            links.new(uv.outputs['UV'], sep.inputs[0])
            axes = []
            for axis in ['X', 'Y']:
                remap = scalar(nodes, 'MULTIPLY_ADD', b=2)
                remap.inputs[2].default_value = -1
                links.new(sep.outputs[axis], remap.inputs[0])
                absolute = scalar(nodes, 'ABSOLUTE')
                links.new(remap.outputs[0], absolute.inputs[0])
                axes.append(absolute)
            maximum = scalar(nodes, 'MAXIMUM')
            for k, a in enumerate(axes):
                links.new(a.outputs[0], maximum.inputs[k])
            smooth = nodes.new('ShaderNodeMapRange')
            smooth.interpolation_type = 'SMOOTHSTEP'
            smooth.inputs['From Min'].default_value = 0.82
            smooth.inputs['From Max'].default_value = 1
            smooth.inputs['To Min'].default_value = 1
            smooth.inputs['To Max'].default_value = 0
            links.new(maximum.outputs[0], smooth.inputs[0])
            links.new(smooth.outputs[0], emit.inputs['Strength'])
        elif spot:
            dot = nodes.new('ShaderNodeVectorMath')
            dot.operation = 'DOT_PRODUCT'
            dot.inputs[1].default_value = spot['v'][:3]
            # Incoming is the ray toward the light, as in Studio's spot contract.
            links.new(geometry.outputs['Incoming'], dot.inputs[0])
            smooth = nodes.new('ShaderNodeMapRange')
            smooth.interpolation_type = 'SMOOTHSTEP'
            smooth.inputs['From Min'].default_value = spot['v'][3]
            smooth.inputs['From Max'].default_value = spot['radiance'][3]
            links.new(dot.outputs['Value'], smooth.inputs[0])
            links.new(smooth.outputs[0], emit.inputs['Strength'])
        if rim:
            # Studio panels emit only toward u cross v.
            mix = nodes.new('ShaderNodeMixShader')
            black = nodes.new('ShaderNodeEmission')
            black.inputs['Color'].default_value = (0, 0, 0, 1)
            links.new(geometry.outputs['Backfacing'], mix.inputs[0])
            links.new(emit.outputs[0], mix.inputs[1])
            links.new(black.outputs[0], mix.inputs[2])
            links.new(mix.outputs[0], output.inputs['Surface'])
        else:
            links.new(emit.outputs[0], output.inputs['Surface'])
        mesh.materials.append(mat)
        obj = bpy.data.objects.new(label, mesh)
        scene.collection.objects.link(obj)
        obj.visible_camera = False
        obj.visible_shadow = False  # Studio's visibility traversal excludes explicit lights.
        return obj

    for i, light in enumerate(data['lights']):
        c, u, v = (vector(light[k]) * scale for k in ['center', 'u', 'v'])
        mesh = bpy.data.meshes.new(f'softbox-{i}')
        mesh.from_pydata([c-u-v, c+u-v, c+u+v, c-u+v], [], [(0, 1, 2, 3)])
        mesh.uv_layers.new().data.foreach_set('uv', [0, 0, 1, 0, 1, 1, 0, 1])
        emitter(f'softbox-{i}', mesh, vector(light['radiance']), rim=True)

    lights = data['extra_lights'] + ([meta['sun']] if meta['sun'] else [])
    for i, light in enumerate(lights):
        kind = int(light['pos'][3])
        radiance = Vector(light['radiance'][:3])
        if kind == 3:
            sun = bpy.data.lights.new(f'sun-{i}', 'SUN')
            cosine = light['u'][0]
            sun.angle = 2 * math.acos(cosine)
            peak = max(radiance)
            sun.color = radiance / max(peak, 1e-12)
            sun.energy = peak * math.pi * (1 - cosine*cosine)
            obj = bpy.data.objects.new(f'sun-{i}', sun)
            scene.collection.objects.link(obj)
            obj.rotation_euler = (-Vector(light['pos'][:3])).to_track_quat('-Z', 'Y').to_euler()
        else:
            bpy.ops.mesh.primitive_uv_sphere_add(segments=24, ring_count=12, radius=light['u'][0]*scale)
            temp = bpy.context.object
            mesh = temp.data
            bpy.data.objects.remove(temp, do_unlink=True)
            obj = emitter(f'practical-{i}', mesh, radiance, spot=light if kind == 2 else None)
            obj.location = Vector(light['pos'][:3]) * scale
            if light['u'][1] or light['u'][2] != 2:
                warnings.append(f'light {i}: nonphysical range/decay unsupported')

    def fog_material(label, density, color, g, height=None):
        mat = bpy.data.materials.new(label)
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        nodes.clear()
        output = nodes.new('ShaderNodeOutputMaterial')
        scatter = nodes.new('ShaderNodeVolumeScatter')
        scatter.inputs['Color'].default_value = (*vector(color), 1)
        scatter.inputs['Density'].default_value = density / scale
        scatter.inputs['Anisotropy'].default_value = g
        absorb = nodes.new('ShaderNodeVolumeAbsorption')
        # Volume Absorption uses (1-color)*density, complementing scatter's color*density.
        absorb.inputs['Color'].default_value = (*vector(color), 1)
        absorb.inputs['Density'].default_value = density / scale
        if height and height['height_falloff']:
            geom = nodes.new('ShaderNodeNewGeometry')
            sep = nodes.new('ShaderNodeSeparateXYZ')
            links.new(geom.outputs['Position'], sep.inputs[0])
            sub = scalar(nodes, 'SUBTRACT', b=height['base_height']*scale)
            links.new(sep.outputs['Y'], sub.inputs[0])
            maximum = scalar(nodes, 'MAXIMUM', b=0)
            links.new(sub.outputs[0], maximum.inputs[0])
            mul = scalar(nodes, 'MULTIPLY', b=-height['height_falloff']/scale)
            links.new(maximum.outputs[0], mul.inputs[0])
            exp = scalar(nodes, 'EXPONENT')
            links.new(mul.outputs[0], exp.inputs[0])
            dens = scalar(nodes, 'MULTIPLY', b=density/scale)
            links.new(exp.outputs[0], dens.inputs[0])
            links.new(dens.outputs[0], scatter.inputs['Density'])
            links.new(dens.outputs[0], absorb.inputs['Density'])
        add = nodes.new('ShaderNodeAddShader')
        links.new(scatter.outputs[0], add.inputs[0])
        links.new(absorb.outputs[0], add.inputs[1])
        links.new(add.outputs[0], output.inputs['Volume'])
        return mat

    fog = data['world']['atmosphere']
    if fog['density']:
        bpy.ops.mesh.primitive_uv_sphere_add(segments=48, ring_count=24, radius=fog['distance']*scale,
                                           location=vector(data['camera']['position'])*scale)
        obj = bpy.context.object
        obj.name = 'finite atmosphere (mapping approximation)'
        obj.data.materials.append(fog_material('atmosphere', fog['density'], fog['color'], fog['anisotropy'], fog))
        warnings.append('Global fog: camera-centered finite sphere approximates Studio per-segment distance cap')
    for i, volume in enumerate(data['world']['volumes']):
        if volume['shape'] == 'box':
            bpy.ops.mesh.primitive_cube_add(size=2, location=vector(volume['center'])*scale)
            obj = bpy.context.object
            obj.scale = vector(volume['size']) * scale
        else:
            bpy.ops.mesh.primitive_uv_sphere_add(segments=48, ring_count=24,
                                               radius=volume['size']['x']*scale,
                                               location=vector(volume['center'])*scale)
            obj = bpy.context.object
        obj.data.materials.append(fog_material(f'volume-{i}', volume['density'], volume['color'], volume['anisotropy']))

    if data['shadow_catcher']:
        bpy.ops.mesh.primitive_plane_add(size=2000*scale)
        floor = bpy.context.object
        floor.name = 'display shadow catcher'
        floor.rotation_euler.x = -math.pi/2
        floor.is_shadow_catcher = True
        mat = bpy.data.materials.new('secondary diffuse floor')
        mat.use_nodes = True
        nodes = mat.node_tree.nodes
        nodes.clear()
        diffuse = nodes.new('ShaderNodeBsdfDiffuse')
        diffuse.inputs['Color'].default_value = (0.82, 0.82, 0.82, 1)
        output = nodes.new('ShaderNodeOutputMaterial')
        mat.node_tree.links.new(diffuse.outputs[0], output.inputs['Surface'])
        floor.data.materials.append(mat)
        warnings.append('Display-only catcher: physical metrics exclude the void; display shadow ratios use Cycles catcher alpha')

    c = data['camera']
    camera = bpy.data.cameras.new('camera')
    obj = bpy.data.objects.new('camera', camera)
    scene.collection.objects.link(obj)
    obj.location = vector(c['position']) * scale
    forward = (vector(c['target']) - vector(c['position'])).normalized()
    right = forward.cross(Vector((0, 1, 0)))
    right = right.normalized() if right.length > 1e-6 else Vector((1, 0, 0))
    up = right.cross(forward)
    obj.rotation_euler = Matrix((right, up, -forward)).transposed().to_euler()
    camera.sensor_fit = 'VERTICAL'
    camera.sensor_height = 24
    camera.lens = 12 / math.tan(math.radians(c['fov'])/2)
    camera.clip_start = 0.00001
    camera.clip_end = 100000
    camera.dof.use_dof = c['f_stop'] > 0
    if camera.dof.use_dof:
        camera.dof.aperture_fstop = c['f_stop']
        focus = vector(c['focus'] or c['target']) - vector(c['position'])
        camera.dof.focus_distance = focus.dot(forward) * scale
    scene.camera = obj
    scene.render.image_settings.file_format = 'OPEN_EXR'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.render.image_settings.color_depth = '32'
    scene.render.filepath = str(out / 'cycles.exr')
    start = time.perf_counter()
    bpy.ops.render.render(write_still=True)
    image = bpy.data.images.load(str(out / 'cycles.exr'), check_existing=False)
    rgba = np.asarray(image.pixels[:], dtype=np.float32).reshape(meta['size'][1], meta['size'][0], 4)[::-1]
    np.save(out / 'cycles.npy', rgba)
    result = dict(blender=bpy.app.version_string, device=[d.name for d in devices], samples=samples,
                  seconds=time.perf_counter()-start, warnings=warnings,
                  sky_reference='Studio-baked map; tests transport, not independent sky accuracy')
    (out / 'cycles-meta.json').write_text(json.dumps(result, indent=2))
    print('YARDSTICK_RENDER', json.dumps(result))


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--')+1:]
    build(pathlib.Path(args[0]), int(args[1]))
