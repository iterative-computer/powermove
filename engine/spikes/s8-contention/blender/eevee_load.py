"""S8 Blender EEVEE load: renders a 3-model 4K scene with shadows in a loop.

Run as Powermove runs its Blender worker:
  Blender --background --factory-startup --disable-autoexec --python-exit-code 1 \
          --python eevee_load.py -- --out DIR [--width 3840 --height 2160 --samples 16]

The scene mirrors the plan's 3D export gate (3 models with shadows, EEVEE, 16 samples):
three subdivided models with principled materials, a sun and an area light with soft
shadows, a ground plane, AgX view transform, transparent film, and RGBA 16-bit PNG
output (decision 15). Objects move every frame so nothing is cached. Each finished
frame prints `S8BLENDER <frame> <render_ms> <write_ms>`; the loop runs until the
process is terminated.
"""

import math
import os
import sys
import time

import bpy


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    opts = {"out": None, "width": 3840, "height": 2160, "samples": 16, "frames": 0}
    i = 0
    while i < len(argv):
        key = argv[i].lstrip("-")
        opts[key] = argv[i + 1]
        i += 2
    for k in ("width", "height", "samples", "frames"):
        opts[k] = int(opts[k])
    if not opts["out"]:
        raise SystemExit("--out DIR is required")
    return opts


def build(opts):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    try:
        scene.render.engine = "BLENDER_EEVEE"
    except TypeError:
        scene.render.engine = "BLENDER_EEVEE_NEXT"
    scene.render.resolution_x = opts["width"]
    scene.render.resolution_y = opts["height"]
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.image_settings.color_depth = "16"
    scene.view_settings.view_transform = "AgX"
    ee = scene.eevee
    if hasattr(ee, "taa_render_samples"):
        ee.taa_render_samples = opts["samples"]
    if hasattr(ee, "use_shadows"):
        ee.use_shadows = True

    world = bpy.data.worlds.new("World")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.4
    scene.world = world

    cam_data = bpy.data.cameras.new("Camera")
    cam = bpy.data.objects.new("Camera", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    cam.location = (0.0, -9.0, 4.0)
    cam.rotation_euler = (math.radians(68), 0.0, 0.0)
    cam_data.lens = 35

    sun = bpy.data.lights.new("Sun", "SUN")
    sun.energy = 3.0
    sun.angle = math.radians(4)
    sun_obj = bpy.data.objects.new("Sun", sun)
    sun_obj.rotation_euler = (math.radians(40), math.radians(15), math.radians(30))
    scene.collection.objects.link(sun_obj)

    area = bpy.data.lights.new("Area", "AREA")
    area.energy = 600.0
    area.size = 3.0
    area_obj = bpy.data.objects.new("Area", area)
    area_obj.location = (-3.0, -3.0, 5.0)
    area_obj.rotation_euler = (math.radians(35), math.radians(-30), 0.0)
    scene.collection.objects.link(area_obj)

    def material(name, rgb, rough, metal):
        m = bpy.data.materials.new(name)
        m.use_nodes = True
        p = m.node_tree.nodes["Principled BSDF"]
        p.inputs["Base Color"].default_value = (*rgb, 1.0)
        p.inputs["Roughness"].default_value = rough
        p.inputs["Metallic"].default_value = metal
        return m

    bpy.ops.mesh.primitive_plane_add(size=30.0, location=(0.0, 0.0, -1.0))
    bpy.context.object.data.materials.append(material("Ground", (0.6, 0.6, 0.62), 0.6, 0.0))

    models = []
    for i, (op, rgb, rough, metal) in enumerate(
        [
            (bpy.ops.mesh.primitive_monkey_add, (0.8, 0.25, 0.2), 0.35, 0.0),
            (bpy.ops.mesh.primitive_torus_add, (0.85, 0.7, 0.3), 0.2, 1.0),
            (bpy.ops.mesh.primitive_ico_sphere_add, (0.2, 0.45, 0.85), 0.1, 0.0),
        ]
    ):
        op(location=(-3.0 + 3.0 * i, 0.0, 0.3))
        obj = bpy.context.object
        mod = obj.modifiers.new("Subsurf", "SUBSURF")
        mod.levels = 3
        mod.render_levels = 3
        bpy.ops.object.shade_smooth()
        obj.data.materials.append(material(f"M{i}", rgb, rough, metal))
        models.append(obj)
    return scene, models


def main():
    opts = args()
    os.makedirs(opts["out"], exist_ok=True)
    scene, models = build(opts)
    n = 0
    while opts["frames"] == 0 or n < opts["frames"]:
        for i, obj in enumerate(models):
            obj.rotation_euler = (0.1 * n, 0.07 * n * (i + 1), 0.05 * n)
            obj.location.z = 0.3 + 0.4 * math.sin(0.2 * n + i)
        t0 = time.perf_counter()
        bpy.ops.render.render(write_still=False)
        t1 = time.perf_counter()
        img = bpy.data.images["Render Result"]
        img.save_render(os.path.join(opts["out"], f"frame-{n % 4}.png"), scene=scene)
        t2 = time.perf_counter()
        print(f"S8BLENDER {n} {(t1 - t0) * 1e3:.1f} {(t2 - t1) * 1e3:.1f}", flush=True)
        n += 1


main()
