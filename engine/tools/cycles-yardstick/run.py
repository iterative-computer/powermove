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

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
SCENES = ['hero', 'edge', 'macro', 'back', 'chrome', 'materials', 'caustics', 'wet',
          'env-room', 'env-street', 'env-landscape']


def blender_path(override=None):
    for path in [override, os.environ.get('BLENDER'), '/Applications/Blender.app/Contents/MacOS/Blender', shutil.which('blender')]:
        if path and Path(path).is_file() and os.access(path, os.X_OK):
            return str(path)
    return None


def exporter_script():
    script = (HERE/'blender_scene.py').read_text()
    return script.replace('# BEGIN MAPPING HELPERS\n# END MAPPING HELPERS',
                          '# BEGIN MAPPING HELPERS\n'+(HERE/'mapping.py').read_text()+'\n# END MAPPING HELPERS')


def reuse_reference(original, out):
    files = ['cycles.exr','cycles.npy','cycles-meta.json','render_cycles.py']
    if any(not (original/name).is_file() for name in files):
        raise ValueError(f'{out.name}: reference image or replay proof missing; rerender Cycles')
    previous = json.loads((original/'manifest.json').read_text())
    if ((original/'scene.json').read_bytes() != (out/'scene.json').read_bytes()
            or previous['size'] != json.loads((out/'manifest.json').read_text())['size']
            or (original/'render_cycles.py').read_text() != exporter_script()):
        raise ValueError(f'{out.name}: reference scene, exporter or size changed; rerender Cycles')
    # Keep the verified replay proof as well as the images. A derived result
    # must remain a valid reference source for the next engine-only rerun.
    for filename in files:
        shutil.copy2(original/filename,out/filename)


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
    # Keep every material visible in the report; a broad metal/glass average
    # otherwise hides unrelated model and exporter errors in the Materials row.
    for index in sorted({obj['material'] for obj in data['objects']}):
        mask[f'material-{index}'] = np.zeros(ids.shape, bool)
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
        mask[f"material-{obj['material']}"] |= part
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
        result['display_regions'] = score(decode(a),decode(b),{'floor':floor,'shadows':shadows},tone_map=False)
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
        p95 = f"{old['delta_e_p95']:.3f} → {s['delta_e_p95']:.3f}" if old else f"{s['delta_e_p95']:.3f}"
        ssim = f"{old['ssim']:.4f} → {s['ssim']:.4f}" if old else f"{s['ssim']:.4f}"
        rows.append(f"| {name} | {de} | {p95} | {ssim} | {result['exposure_ev']:+.3f} |")
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
    for result in results:
        summary += f"\n## {result['scene']} regions\n\n| Region | Pixels | Mean ΔE00 | p95 ΔE00 | SSIM | Raw luminance ratio |\n|---|---:|---:|---:|---:|---:|\n"
        for region, s in result['regions'].items():
            if s['pixels']:
                summary += f"| {region} | {s['pixels']} | {s['delta_e_mean']:.3f} | {s['delta_e_p95']:.3f} | {s['ssim']:.4f} | {s['raw_luminance_ratio']:.4f} |\n"
            else:
                summary += f"| {region} | 0 | unavailable | | | |\n"
        if result.get('display_regions'):
            summary += '\nDisplay-only catcher scores (separate from scene-linear radiance):\n\n'
            for region, s in result['display_regions'].items():
                if s['pixels']:
                    summary += f"- {region}: mean ΔE00 {s['delta_e_mean']:.3f}, SSIM {s['ssim']:.4f}\n"
    (out/'report.md').write_text(summary)
    chart_rows = []
    maximum = max([r['regions']['whole']['delta_e_mean'] for r in results]
                  + [r['regions']['whole']['delta_e_mean'] for r in (before or {}).values()] + [1]) * 1.05
    for result in results:
        name = result['scene']
        value = result['regions']['whole']['delta_e_mean']
        previous = before.get(name, {}).get('regions', {}).get('whole', {}).get('delta_e_mean') if before else None
        bars = (f'<i class="before" style="width:{previous/maximum*100:.2f}%"></i>' if previous is not None else '')
        bars += f'<i class="after" style="width:{value/maximum*100:.2f}%"></i>'
        values = f'{previous:.2f} → {value:.2f}' if previous is not None else f'{value:.2f}'
        chart_rows.append(f'<div class="row"><span>{html.escape(name)}</span><div class="bars">{bars}</div><span class="value">{values}</span></div>')
    first = results[0]
    w, h = first['studio']['size']
    settings = f"{w}×{h} · Studio {first['studio']['samples']:,} samples / Cycles {first['cycles']['samples']:,} · exposure matched"
    chart = ('<style>.comparison-chart{font:13px var(--font-sans,system-ui);color:var(--foreground);width:100%}'
             '.comparison-chart p{margin:0 0 12px;color:var(--muted-foreground)}'
             '.comparison-chart .row{display:grid;grid-template-columns:105px minmax(0,1fr) 100px;gap:10px;align-items:center;min-height:27px}'
             '.comparison-chart .bars{display:grid;gap:3px}.comparison-chart i{display:block;min-width:1px;height:5px;border-radius:3px}'
             '.comparison-chart .before{background:var(--chart-1)}.comparison-chart .after{background:var(--chart-2)}'
             '.comparison-chart .value{text-align:right;font-variant-numeric:tabular-nums}'
             '.comparison-chart .legend{display:flex;gap:18px;margin:10px 0}.comparison-chart .legend span{display:flex;gap:6px;align-items:center}'
             '.comparison-chart .legend i{width:15px}@media(max-width:450px){.comparison-chart .row{grid-template-columns:84px minmax(0,1fr) 88px;gap:6px;font-size:11px}}</style>'
             '<div class="comparison-chart"><p>Mean ΔE00 against Cycles · lower is closer</p><div class="legend">'
             + ('<span><i class="before"></i>Before</span>' if before else '')
             + '<span><i class="after"></i>After</span></div>'+''.join(chart_rows)
             + f'<p style="margin-top:12px">{settings}<br>Product foreground and full environment scores use different domains. This is not a parity claim.</p></div>')
    (out/'summary.html').write_text('<!doctype html><meta charset="utf-8">'+chart)
    (out/'report.html').write_text('<!doctype html><meta charset="utf-8"><title>Studio / Cycles</title>'
        '<style>:root{--foreground:#eee;--muted-foreground:#bbb;--chart-1:#b7a9ee;--chart-2:#73cfb6}body{font:16px system-ui;background:#17191f;color:#eee;margin:32px auto;max-width:1400px}img{width:100%;image-rendering:auto}section{margin:48px 0}pre{white-space:pre-wrap;font-size:13px}h1,h2{font-weight:600}</style>'
        '<h1>Studio Final / Cycles Metal</h1><p>Linear EXR comparison, shared PBR Neutral output transform. No denoising. '
        'Scene exposure is fitted once; raw radiometry is recorded separately. Backdrop/catcher and unsupported material mappings are labeled.</p>'+chart+''.join(panels))
    (out/'scores.json').write_text(json.dumps(results,indent=2,allow_nan=False))
    print('\n'.join(rows), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=Path('/tmp/pm-shots/studio3d/laneE/baseline'))
    parser.add_argument('--scenes')
    parser.add_argument('--size')
    parser.add_argument('--samples', type=int)
    parser.add_argument('--cycles-samples', type=int)
    parser.add_argument('--blender')
    parser.add_argument('--studio', type=Path)
    parser.add_argument('--reuse-cycles', type=Path)
    parser.add_argument('--before', type=Path)
    parser.add_argument('--score-only', action='store_true')
    parser.add_argument('--cycles-only', action='store_true', help='rerender exported scenes after reference mapping changes')
    parser.add_argument('--regression', action='store_true')
    args = parser.parse_args()
    blender = blender_path(args.blender)
    if not blender:
        print('SKIP: Blender absent; Cycles is an optional test-only yardstick.')
        return 0
    explicit_samples=args.samples is not None
    explicit_cycles_samples=args.cycles_samples is not None
    if args.regression:
        tolerances = json.loads((HERE/'tolerances.json').read_text())
        args.scenes = args.scenes or ','.join(tolerances['scenes'])
        args.size = args.size or tolerances['size']
        args.samples = args.samples or tolerances['studio_samples']
        args.cycles_samples = args.cycles_samples or tolerances['cycles_samples']
    else:
        args.scenes = args.scenes or ','.join(SCENES)
        args.size = args.size or '320x180'
        args.samples = args.samples or 1024
        args.cycles_samples = args.cycles_samples or 2048
    import numpy  # Delay optional dependencies until after clean absence skip.
    import PIL
    names = args.scenes.split(',')
    if any(name not in SCENES for name in names):
        parser.error('unknown corpus scene')
    if args.regression and any(name not in tolerances['scenes'] for name in names):
        parser.error('regression mode requires a scene listed in tolerances.json')
    if args.score_only and args.cycles_only:
        parser.error('--score-only and --cycles-only are mutually exclusive')
    if args.reuse_cycles and args.cycles_only:
        parser.error('--reuse-cycles and --cycles-only are mutually exclusive')
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
    if not (args.score_only or args.cycles_only) and not args.studio:
        cargo = shutil.which('cargo') or str(Path.home()/'.cargo/bin/cargo')
        env = dict(os.environ,CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='2')
        subprocess.run(['nice','-n','10',cargo,'build','--manifest-path',str(ROOT/'engine/Cargo.toml'),
                        '-p','pm-studio','--bin','studio-yardstick','--features','yardstick','--release'],env=env,check=True)
    results = []
    for name in names:
        print(f'{name}: rendering/scoring',flush=True)
        limit=tolerances['scenes'][name] if args.regression else {}
        studio_samples=args.samples if explicit_samples else limit.get('studio_samples',args.samples)
        cycles_samples=args.cycles_samples if explicit_cycles_samples else limit.get('cycles_samples',args.cycles_samples)
        out = args.out/name
        out.mkdir(exist_ok=True)
        if not args.score_only:
            # The tiny regression writes bounded frames, not a full image corpus.
            # Retain the larger reserve for diagnostic/high-resolution runs.
            width, height = map(int, args.size.split('x'))
            reserve = 5_500_000_000 if args.regression and width * height <= 128 * 72 else 8_000_000_000
            if shutil.disk_usage(args.out).free < reserve:
                raise RuntimeError(f'less than {reserve / 1e9:g} GB free; stopping before further renders')
            if not args.cycles_only:
                run_logged([studio,'--scene',name,'--out',out,'--size',args.size,'--spp',studio_samples],out/'studio.log',175)
            if args.reuse_cycles:
                original = args.reuse_cycles/name
                reuse_reference(original,out)
            else:
                script = out/'render_cycles.py'
                # Concrete replay script alongside each exported scene, with no .blend file.
                script.write_text(exporter_script())
                run_logged([blender,'-b','--factory-startup','--threads','2','--python-exit-code','1',
                            '--python',script,'--',out,cycles_samples],out/'cycles.log',175)
        result = compare(out)
        results.append(result)
        print(f"{name}: ΔE00 {result['regions']['whole']['delta_e_mean']:.3f}, SSIM {result['regions']['whole']['ssim']:.4f}",flush=True)
    before = {r['scene']:r for r in json.loads(args.before.read_text())} if args.before else None
    report(args.out,results,before)
    if args.regression:
        for result in results:
            limit = tolerances['scenes'][result['scene']]
            s = result['regions']['whole']
            if (s['delta_e_mean'] > limit['max_mean_delta_e'] or s['ssim'] < limit['min_ssim']
                    or abs(result['exposure_ev']) > limit['max_abs_exposure_ev']):
                raise AssertionError(f"{result['scene']} exceeded its Cycles regression tolerance: {s}")
    return 0


if __name__ == '__main__':
    sys.exit(main())
