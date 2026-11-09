#!/usr/bin/env python3
"""Optional offline Cycles yardstick. All scene/render/report artifacts stay outside Git."""
import argparse
import html
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
SCENES = ['hero', 'edge', 'macro', 'back', 'chrome', 'materials', 'caustics', 'wet',
          'env-room', 'env-street', 'env-landscape']


def blender_path(override=None):
    for path in [override, os.environ.get('BLENDER'), '/Applications/Blender.app/Contents/MacOS/Blender', shutil.which('blender')]:
        if path and Path(path).is_file() and os.access(path, os.X_OK):
            return str(path)
    return None


def run_logged(command, log, timeout):
    with log.open('w') as stream:
        subprocess.run(['nice', '-n', '10', *map(str, command)], stdout=stream, stderr=subprocess.STDOUT,
                       timeout=timeout, check=True)


def masks_for(data, ids, rgba, ref):
    import numpy as np
    # Erode by two pixels to avoid assigning mixed silhouettes to a material.
    def interior(mask):
        padded = np.pad(mask, 2)
        return np.logical_and.reduce([padded[y:y+mask.shape[0], x:x+mask.shape[1]]
                                      for y in range(5) for x in range(5)])
    product = data['shadow_catcher']
    valid = (rgba[..., 3] > .98) & (ref[..., 3] > .98) if product else np.ones(ids.shape, bool)
    mask = {name: np.zeros(ids.shape, bool) for name in ['metal', 'glass', 'floor', 'shadows', 'sky']}
    assignments = {}
    if data['instances']:
        for inst in data['instances']:
            obj = inst['object']
            layer = inst.get('layer')
            if layer is None:
                layer = data['layers'][obj] if data['layers'] else obj
            assignments[layer+1] = obj
    else:
        assignments = {(data['layers'][i] if data['layers'] else i)+1: i for i in range(len(data['objects']))}
    for layer, index in assignments.items():
        obj = data['objects'][index]
        m = data['materials'][obj['material']]
        part = ids == layer
        if m['metallic'] > .5:
            mask['metal'] |= part
        if m['transmission'] > .5 or m['coat'] > .8:
            mask['glass'] |= part
        # Environment floors: first room/street mesh, or generated landscape terrain.
        if ((not product and index == 0 and not data['instances'])
                or (data['instances'] and index >= 2)):
            mask['floor'] |= part
    mask['sky'] = ids == 0
    if product:
        # Synthetic catcher is a display policy, not scene-linear surface radiance.
        mask['floor'] = (ids == 0) & (np.indices(ids.shape)[0] >= ids.shape[0]//2)
        mask['sky'] &= ~mask['floor']
    floor_values = (ref[..., :3] @ np.array([.2126, .7152, .0722]))[mask['floor']]
    if floor_values.size:
        mask['shadows'] = mask['floor'] & ((ref[..., :3] @ np.array([.2126, .7152, .0722])) <= np.percentile(floor_values, 25))
    mask = {k: interior(v) & valid for k, v in mask.items()}
    mask['whole'] = interior(valid) if product else valid
    return mask


def compare(out):
    import numpy as np
    from PIL import Image
    from metrics import exposure_match, neutral, score, srgb, ssim_map, decode
    data = json.loads((out/'scene.json').read_text())
    meta = json.loads((out/'manifest.json').read_text())
    w, h = meta['size']
    studio = np.fromfile(out/'studio.rgba32f', dtype='<f4').reshape(h, w, 4)
    ref = np.load(out/'cycles.npy')
    ids = np.fromfile(out/'studio.ids', dtype='<u4').reshape(h, w)
    if not np.isfinite(studio).all() or not np.isfinite(ref).all():
        raise ValueError('non-finite render pixels')
    masks = masks_for(data, ids, studio, ref)
    if not masks['whole'].any():
        raise ValueError('no common opaque foreground pixels; increase resolution or change the camera pose')
    gain = exposure_match(studio[..., :3], ref[..., :3], masks['whole'])
    regions = score(studio[..., :3]*data['exposure'], ref[..., :3]*data['exposure'], masks, gain)
    def display(rgba, gain=1):
        alpha = rgba[..., 3:4]
        rgb = srgb(neutral(rgba[..., :3]/np.maximum(alpha, 1e-6)*gain*data['exposure']))*alpha
        return np.clip(rgb+np.array(list(data['backdrop'].values()))*(1-alpha), 0, 1)
    a = np.asarray(Image.open(out/'studio.png').convert('RGB'), dtype=float)/255
    b = display(ref, 1)
    matched = display(studio, gain)
    diff = np.clip(np.abs(matched-b)*4, 0, 1)
    Image.fromarray(np.uint8(np.concatenate([a, b, diff], axis=1)*255+.5)).save(out/'comparison.png')
    Image.fromarray(np.uint8(b*255+.5)).save(out/'cycles.png')
    for name, mask in masks.items():
        Image.fromarray(np.uint8(mask)*255).save(out/f'mask-{name}.png')
    result = dict(scene=meta['scene'], studio=meta, cycles=json.loads((out/'cycles-meta.json').read_text()),
                  exposure_gain=gain, exposure_ev=float(np.log2(gain)), regions=regions,
                  display_ssim=float(ssim_map(a,b).mean()),
                  shadow_mask='darkest quartile of first-hit floor (proxy, not isolated direct-shadow pass)',
                  comparison='scene-linear EXR, common PBR Neutral, one scene exposure fit; product foreground excludes display-only catcher')
    if data['shadow_catcher']:
        # Explicitly report floor/catcher display error, separate from physical EXR scores.
        y = np.indices(ids.shape)[0]
        floor = (ids == 0) & (y >= h//2)
        shadows = floor & (b.mean(axis=-1) < .95)
        result['display_regions'] = score(decode(a),decode(b),{'floor':floor,'shadows':shadows})
    try:
        import flip_evaluator as flip
    except ImportError:
        result['flip'] = dict(available=False, fallback='11x11 Gaussian SSIM + linear RGB/luminance MAE')
    else:
        _, flip_mean, _ = flip.evaluate(srgb(neutral(ref[..., :3]*data['exposure'])),
                                        srgb(neutral(studio[..., :3]*data['exposure']*gain)), 'LDR')
        result['flip'] = dict(available=True, mean=float(flip_mean))
    (out/'scores.json').write_text(json.dumps(result,indent=2, allow_nan=False))
    return result


def report(out, results, before=None):
    rows = ['| Scene | Mean ΔE00 | p95 ΔE00 | SSIM | Exposure EV |', '|---|---:|---:|---:|---:|']
    panels = []
    for result in results:
        name = result['scene']
        s = result['regions']['whole']
        old = before.get(name, {}).get('regions', {}).get('whole') if before else None
        de = f"{old['delta_e_mean']:.3f} → {s['delta_e_mean']:.3f}" if old else f"{s['delta_e_mean']:.3f}"
        rows.append(f"| {name} | {de} | {s['delta_e_p95']:.3f} | {s['ssim']:.4f} | {result['exposure_ev']:+.3f} |")
        warnings = result['cycles']['warnings']
        relative = Path(name)/'comparison.png'
        panels.append(f'<section><h2>{html.escape(name)}</h2><p>Studio Final | Cycles Metal | matched difference ×4</p>'
                      f'<img src="{relative}" alt="{html.escape(name)} comparison"><p>Mean ΔE00 {de}; SSIM {s["ssim"]:.4f}</p>'
                      f'<details><summary>Mappings and region scores</summary><pre>{html.escape(json.dumps(dict(regions=result["regions"],warnings=warnings),indent=2))}</pre></details></section>')
    summary = '# Studio / Cycles yardstick\n\n'+'\n'.join(rows)+'\n\n'
    summary += 'ΔE00 uses a common PBR Neutral transform and one scalar exposure fit; raw exposure and region errors remain in scores.json. '
    summary += 'Product scores cover opaque foreground; the display-only backdrop and catcher have separate display scores. '
    summary += 'Sky maps are baked from Studio, so this tests transport, not independent atmospheric accuracy. '
    summary += 'No claim of Cycles parity. See docs/studio/cycles-yardstick.md for mapping limits.\n'
    (out/'report.md').write_text(summary)
    (out/'report.html').write_text('<!doctype html><meta charset="utf-8"><title>Studio / Cycles</title>'
        '<style>body{font:16px system-ui;background:#17191f;color:#eee;margin:32px auto;max-width:1400px}img{width:100%;image-rendering:auto}section{margin:48px 0}pre{white-space:pre-wrap;font-size:13px}h1,h2{font-weight:600}</style>'
        '<h1>Studio Final / Cycles Metal</h1><p>Linear EXR comparison, shared PBR Neutral output transform. No denoising. '
        'Scene exposure is fitted once; raw radiometry is recorded separately. Backdrop/catcher and unsupported material mappings are labeled.</p>'+''.join(panels))
    (out/'scores.json').write_text(json.dumps(results,indent=2,allow_nan=False))
    print(summary, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=Path('/tmp/pm-shots/studio3d/laneE/baseline'))
    parser.add_argument('--scenes', default=','.join(SCENES))
    parser.add_argument('--size', default='320x180')
    parser.add_argument('--samples', type=int, default=256)
    parser.add_argument('--cycles-samples', type=int, default=512)
    parser.add_argument('--blender')
    parser.add_argument('--studio', type=Path)
    parser.add_argument('--reuse-cycles', type=Path)
    parser.add_argument('--before', type=Path)
    parser.add_argument('--score-only', action='store_true')
    parser.add_argument('--regression', action='store_true')
    args = parser.parse_args()
    blender = blender_path(args.blender)
    if not blender:
        print('SKIP: Blender absent; Cycles is an optional test-only yardstick.')
        return 0
    import numpy  # Delay optional dependencies until after clean absence skip.
    import PIL
    names = args.scenes.split(',')
    if any(name not in SCENES for name in names):
        parser.error('unknown corpus scene')
    if not 1 <= args.cycles_samples <= 4096 or not 1 <= args.samples <= 4096:
        parser.error('samples must be 1..4096')
    args.out = args.out.resolve()
    try:
        args.out.relative_to(ROOT)
    except ValueError:
        pass
    else:
        parser.error('render artifacts must be outside the public repository')
    args.out.mkdir(parents=True,exist_ok=True)
    target = Path(os.environ.get('CARGO_TARGET_DIR', '~/Developer/pm-target')).expanduser()
    studio = args.studio or target/'release/studio-yardstick'
    if not args.score_only and not args.studio:
        cargo = shutil.which('cargo') or str(Path.home()/'.cargo/bin/cargo')
        env = dict(os.environ,CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='2')
        subprocess.run(['nice','-n','10',cargo,'build','--manifest-path',str(ROOT/'engine/Cargo.toml'),
                        '-p','pm-studio','--bin','studio-yardstick','--features','yardstick','--release'],env=env,check=True)
    results = []
    for name in names:
        print(f'{name}: rendering/scoring',flush=True)
        out = args.out/name
        out.mkdir(exist_ok=True)
        if not args.score_only:
            if shutil.disk_usage(args.out).free < 8_000_000_000:
                raise RuntimeError('less than 8 GB free; stopping before further renders')
            run_logged([studio,'--scene',name,'--out',out,'--size',args.size,'--spp',args.samples],out/'studio.log',175)
            if args.reuse_cycles:
                for filename in ['cycles.exr','cycles.npy','cycles-meta.json']:
                    shutil.copy2(args.reuse_cycles/name/filename,out/filename)
            else:
                script = out/'render_cycles.py'
                # Concrete replay script alongside each exported scene, with no .blend file.
                script.write_text((HERE/'blender_scene.py').read_text())
                run_logged([blender,'-b','--factory-startup','--threads','2','--python-exit-code','1',
                            '--python',script,'--',out,args.cycles_samples],out/'cycles.log',175)
        result = compare(out)
        results.append(result)
        print(f"{name}: ΔE00 {result['regions']['whole']['delta_e_mean']:.3f}, SSIM {result['regions']['whole']['ssim']:.4f}",flush=True)
    before = {r['scene']:r for r in json.loads(args.before.read_text())} if args.before else None
    report(args.out,results,before)
    if args.regression:
        tolerances = json.loads((HERE/'tolerances.json').read_text())
        for result in results:
            limit = tolerances['scenes'][result['scene']]
            s = result['regions']['whole']
            if s['delta_e_mean'] > limit['max_mean_delta_e'] or s['ssim'] < limit['min_ssim']:
                raise AssertionError(f"{result['scene']} exceeded its Cycles regression tolerance: {s}")
    return 0


if __name__ == '__main__':
    sys.exit(main())
