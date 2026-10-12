# AgX and alpha check for the pinned Blender worker settings (plan section 17).
# Renders emission patches of known scene-linear values at alpha 1, 0.5, and 0.25 on a
# transparent film, saves 16-bit RGBA PNG exactly as the pinned worker does, and writes
# the expected codes from Blender's own OCIO config (AgX view, sRGB display, look None):
#   straight: expected color = AgX(E), alpha = a
#   premultiplied-then-transformed: AgX(a·E)/a (what a premultiplied pipeline would give)
#
#   Blender --background --factory-startup --python-exit-code 1 --python blender/agx_check.py -- <out-dir> <engine>
import bpy, sys, os, json, math
import PyOpenColorIO as OCIO

out_dir, engine = sys.argv[sys.argv.index('--') + 1:][:2]
os.makedirs(out_dir, exist_ok=True)
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.engine = 'CYCLES' if engine == 'cycles' else 'BLENDER_EEVEE_NEXT'
if engine == 'cycles':
    scene.cycles.samples = 16
    scene.cycles.use_denoising = False
    scene.cycles.device = 'CPU'
else:
    scene.eevee.taa_render_samples = 16
values = [0.0, 0.002, 0.01, 0.05, 0.18, 0.5, 1.0, 2.0, 4.0, 16.0]
tints = [(1, 1, 1), (1, 0.2, 0.05), (0.1, 0.6, 1.0)]
alphas = [1.0, 0.5, 0.25]
cols, rows = len(values), len(tints) * len(alphas)
W, H, cell = cols * 24, rows * 24, 24
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = W, H, 100
scene.render.film_transparent = True
scene.render.filter_size = 0.01 if engine == 'cycles' else scene.render.filter_size
# Pinned worker settings (blender/run.ts PINNED, applied to the shipped worker).
s = scene
s.render.image_settings.file_format = 'PNG'; s.render.image_settings.color_mode = 'RGBA'; s.render.image_settings.color_depth = '16'; s.render.image_settings.compression = 15
s.render.dither_intensity = 0
s.display_settings.display_device = 'sRGB'; s.view_settings.view_transform = 'AgX'; s.view_settings.look = 'None'; s.view_settings.gamma = 1; s.view_settings.exposure = 0
s.render.image_settings.color_management = 'FOLLOW_SCENE'
world = bpy.data.worlds.new('W'); world.use_nodes = True; world.node_tree.nodes['Background'].inputs['Strength'].default_value = 0; scene.world = world
cam_data = bpy.data.cameras.new('C'); cam_data.type = 'ORTHO'; cam_data.ortho_scale = cols; cam_data.sensor_fit = 'HORIZONTAL'
cam = bpy.data.objects.new('C', cam_data); scene.collection.objects.link(cam); scene.camera = cam
cam.location = (cols / 2, -rows / 2, 10)
patches = []
for r in range(rows):
    tint, alpha = tints[r // len(alphas)], alphas[r % len(alphas)]
    for c, v in enumerate(values):
        mat = bpy.data.materials.new(f'm{r}_{c}'); mat.use_nodes = True; nt = mat.node_tree; nt.nodes.clear()
        emit = nt.nodes.new('ShaderNodeEmission'); emit.inputs['Color'].default_value = (*tint, 1); emit.inputs['Strength'].default_value = v
        transparent = nt.nodes.new('ShaderNodeBsdfTransparent'); mix = nt.nodes.new('ShaderNodeMixShader'); mix.inputs[0].default_value = alpha
        outp = nt.nodes.new('ShaderNodeOutputMaterial')
        nt.links.new(transparent.outputs[0], mix.inputs[1]); nt.links.new(emit.outputs[0], mix.inputs[2]); nt.links.new(mix.outputs[0], outp.inputs['Surface'])
        if hasattr(mat, 'surface_render_method'): mat.surface_render_method = 'BLENDED'
        mesh = bpy.data.meshes.new(f'p{r}_{c}')
        x0, y0 = c + 0.02, -(r + 1) + 0.02
        mesh.from_pydata([(x0, y0, 0), (x0 + 0.96, y0, 0), (x0 + 0.96, y0 + 0.96, 0), (x0, y0 + 0.96, 0)], [], [(0, 1, 2, 3)])
        obj = bpy.data.objects.new(mesh.name, mesh); scene.collection.objects.link(obj); mesh.materials.append(mat)
        patches.append({'row': r, 'col': c, 'value': v, 'tint': tint, 'alpha': alpha, 'px': [c * cell + cell // 2, r * cell + cell // 2]})
scene.render.filepath = os.path.join(out_dir, f'agx-{engine}.png')
bpy.ops.render.render(write_still=True)

config = OCIO.Config.CreateFromFile(os.path.join(bpy.utils.resource_path('LOCAL'), 'datafiles', 'colormanagement', 'config.ocio'))
proc = config.getProcessor(OCIO.DisplayViewTransform(src='Linear Rec.709', display='sRGB', view='AgX')).getDefaultCPUProcessor()
def agx(rgb):
    return list(proc.applyRGB([float(x) for x in rgb]))
for p in patches:
    e = [p['value'] * t for t in p['tint']]
    p['expected_straight'] = [round(min(1, max(0, x)) * 65535) for x in agx(e)]
    a = p['alpha']
    p['expected_premul_transform'] = [round(min(1, max(0, x / a)) * 65535) if a > 0 else 0 for x in agx([a * x for x in e])]
    p['expected_alpha'] = round(a * 65535)
with open(os.path.join(out_dir, f'agx-{engine}.json'), 'w') as f:
    json.dump({'blender': bpy.app.version_string, 'ocio': OCIO.__version__, 'patches': patches, 'size': [W, H]}, f)
