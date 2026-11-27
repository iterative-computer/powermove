"""Render Studio's reference scene in Mitsuba 3 and compare it with Studio.

    cargo run --release -p pm-studio --bin studio-reference -- --out /tmp/ref
    python compare.py /tmp/ref            # needs: pip install mitsuba numpy pillow

Reads scene.json and studio.pfm from the directory; writes mitsuba.pfm,
compare.png (Studio | Mitsuba | difference ×4) and report.json.
"""

import json
import os
import sys
import time
from pathlib import Path

import mitsuba as mi
import numpy as np
from PIL import Image


def variant() -> str:
    for v in ("metal_ad_rgb", "llvm_ad_rgb", "scalar_rgb"):
        try:
            mi.set_variant(v)
            return v
        except Exception:
            continue
    raise RuntimeError("no Mitsuba variant")


def rim(profile, size=256):
    """The softbox's soft edge, as Studio draws it: smoothstep(1, 0.82, max(|u|, |v|))."""
    e0, e1 = profile
    t = np.linspace(-1.0, 1.0, size)
    m = np.maximum(np.abs(t)[None, :], np.abs(t)[:, None])
    k = np.clip((m - e0) / (e1 - e0), 0.0, 1.0)
    return k * k * (3.0 - 2.0 * k)


def scene_dict(spec):
    w, h = spec["size"]
    cam = spec["camera"]
    T = mi.ScalarTransform4f
    light = spec["light"]
    c, u, v = (np.array(light[k]) for k in ("center", "u", "v"))
    n = np.cross(u, v)
    n /= np.linalg.norm(n)
    to_world = np.identity(4)
    to_world[:3, 0], to_world[:3, 1], to_world[:3, 2], to_world[:3, 3] = u, v, n, c
    radiance = rim(light["rim"])[:, :, None] * np.array(light["radiance"])[None, None, :]
    d = {
        "type": "scene",
        "integrator": {"type": "path", "max_depth": 11},
        "sensor": {
            "type": "perspective",
            "fov": cam["fov_y"],
            "fov_axis": "y",
            "to_world": T().look_at(origin=cam["position"], target=cam["target"], up=[0, 1, 0]),
            "film": {"type": "hdrfilm", "width": w, "height": h, "rfilter": {"type": "box"}, "pixel_format": "rgb"},
            "sampler": {"type": "independent", "sample_count": spec["spp"]},
        },
        "floor": {
            "type": "rectangle",
            "to_world": T().rotate([1, 0, 0], -90).scale(spec["floor"]["size"]),
            "bsdf": {"type": "diffuse", "reflectance": {"type": "rgb", "value": spec["floor"]["albedo"]}},
        },
        "softbox": {
            "type": "rectangle",
            "to_world": T(to_world.tolist()),
            "emitter": {"type": "area", "radiance": {"type": "bitmap", "bitmap": mi.Bitmap(radiance.astype(np.float32)), "raw": True, "filter_type": "bilinear"}},
            "bsdf": {"type": "diffuse", "reflectance": {"type": "rgb", "value": [0, 0, 0]}},
        },
    }
    for i, s in enumerate(spec["spheres"]):
        a = s["roughness"] ** 2
        color = {"type": "rgb", "value": s["color"]}
        bsdf = {
            "matte": {"type": "diffuse", "reflectance": color},
            # Studio's dielectric base follows OpenPBR (specular over diffuse,
            # energy split by Fresnel); "principled" is Mitsuba's closest model.
            # PM_PLASTIC=roughplastic compares against a coating with internal
            # reflections instead.
            "plastic": {"type": "roughplastic", "distribution": "ggx", "alpha": a, "int_ior": s["ior"], "diffuse_reflectance": color, "nonlinear": False}
            if os.environ.get("PM_PLASTIC") == "roughplastic"
            else {"type": "principled", "base_color": color, "roughness": s["roughness"], "specular": 0.5, "metallic": 0.0},
            "gold": {"type": "roughconductor", "distribution": "ggx", "alpha": max(a, 1e-4), "material": "none", "specular_reflectance": color},
            "chrome": {"type": "roughconductor", "distribution": "ggx", "alpha": max(a, 1e-4), "material": "none", "specular_reflectance": color},
            "glass": {"type": "dielectric", "int_ior": s["ior"], "ext_ior": 1.0},
        }[s["kind"]]
        d[f"sphere{i}"] = {"type": "sphere", "center": s["center"], "radius": s["radius"], "bsdf": bsdf}
    return d


def read_pfm(path):
    with open(path, "rb") as f:
        assert f.readline().strip() == b"PF"
        w, h = map(int, f.readline().split())
        scale = float(f.readline())
        data = np.frombuffer(f.read(), dtype="<f4" if scale < 0 else ">f4").reshape(h, w, 3)
    return np.flipud(data).copy()


def write_pfm(path, img):
    h, w, _ = img.shape
    with open(path, "wb") as f:
        f.write(f"PF\n{w} {h}\n-1.0\n".encode())
        f.write(np.flipud(img).astype("<f4").tobytes())


def project(spec, p):
    cam = spec["camera"]
    w, h = spec["size"]
    pos, target = np.array(cam["position"]), np.array(cam["target"])
    fwd = (target - pos) / np.linalg.norm(target - pos)
    right = np.cross(fwd, [0, 1, 0]); right /= np.linalg.norm(right)
    up = np.cross(right, fwd)
    tan_y = np.tan(np.radians(cam["fov_y"]) / 2)
    v = np.array(p) - pos
    z = v @ fwd
    x = (v @ right) / (z * tan_y * w / h)
    y = -(v @ up) / (z * tan_y)
    return (x + 1) / 2 * w, (y + 1) / 2 * h, z, tan_y


def luminance(img):
    return img @ np.array([0.2126, 0.7152, 0.0722])


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "studio-reference")
    spec = json.loads((out / "scene.json").read_text())
    used = variant()
    t = time.time()
    ref = np.array(mi.render(mi.load_dict(scene_dict(spec))))[:, :, :3]
    ref_s = time.time() - t
    write_pfm(out / "mitsuba.pfm", ref)
    studio = read_pfm(out / "studio.pfm")
    h, w, _ = ref.shape
    ys, xs = np.mgrid[0:h, 0:w]

    regions = {}
    for s in spec["spheres"]:
        cx, cy, z, tan_y = project(spec, s["center"])
        r = s["radius"] / z / tan_y * h / 2
        regions[s["kind"]] = (xs - cx) ** 2 + (ys - cy) ** 2 < (0.8 * r) ** 2
        # The floor just in front of each sphere: its shadow, contact and caustic.
        fx, fy, _, _ = project(spec, [s["center"][0], 0.0, s["center"][2] + s["radius"] * 1.4])
        regions[s["kind"] + " floor"] = (np.abs(xs - fx) < r) & (np.abs(ys - fy) < 0.35 * r)
    regions["whole frame"] = np.ones((h, w), bool)

    report = {"variant": used, "spp": spec["spp"], "size": spec["size"], "mitsuba_s": ref_s, "studio_ms": spec.get("studio_ms"), "regions": {}}
    for name, mask in regions.items():
        a, b = luminance(studio)[mask].mean(), luminance(ref)[mask].mean()
        report["regions"][name] = {"studio": float(a), "mitsuba": float(b), "ratio": float(a / b) if b > 0 else None}
    # Per-pixel error after a 4×4 box filter (averages away most sampling noise).
    def box(img):
        hh, ww = h // 4 * 4, w // 4 * 4
        return img[:hh, :ww].reshape(hh // 4, 4, ww // 4, 4, 3).mean(axis=(1, 3))
    la, lb = luminance(box(studio)), luminance(box(ref))
    report["mean_relative_error"] = float(np.mean(np.abs(la - lb) / (lb + 0.02)))
    (out / "report.json").write_text(json.dumps(report, indent=2))

    def show(img):
        return (np.clip(img, 0, 1) ** (1 / 2.2) * 255).astype(np.uint8)
    diff = np.abs(luminance(studio) - luminance(ref))[:, :, None].repeat(3, axis=2) * 4
    Image.fromarray(np.concatenate([show(studio), show(ref), show(diff)], axis=1)).save(out / "compare.png")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
