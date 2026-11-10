#!/usr/bin/env python3
"""Measure saved Studio presets using the DN reference, SSIM/DeltaE and flicker metrics.

All frames/logs stay outside Git. Timings include host readback and filtering;
shutter images share one total sample/time budget. This frozen-scene diagnostic
is separate from actual moving editor exports and their encoder timings.
"""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import shutil
import time

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('dn', Path(__file__).with_name('denoise-quality.py'))
dn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dn)
PRESETS = json.loads((ROOT / 'apps/desktop/src/shared/studio-quality-presets.json').read_text())


def dimensions(raw):
    w, h = map(int, raw.split('x'))
    if not (16 <= w <= 1920 and 16 <= h <= 1080):
        raise ValueError('Use a diagnostic size between 16x16 and 1920x1080')
    return w, h


def check_disk(out):
    if shutil.disk_usage(out).free < 5_000_000_000:
        raise RuntimeError('Stop: keep more than 5 GB free')


def render(host, size, q, at, shutters=1):
    w, h = size
    shutters=min(shutters,q['maxSamples'])
    start = time.perf_counter()
    images, metadata = [], []
    frame_key=f"quality-{host.serial}-{at}-{w}-{h}"
    for i in range(shutters):
        command=dict(op='render', width=w, height=h, tier='final', goal=True,
            samples=max(1, math.floor(q['maxSamples'] / shutters)),
            minSamples=max(1, math.ceil(q['minSamples'] / shutters)),
            noiseThreshold=q['noiseThreshold'], adaptive=q['noiseThreshold'] > 0,
            timeBudgetMs=round(q['timeLimitSeconds'] * 1000 / shutters),
            maxBounces=q['maxBounces'], caustics=q['caustics'], denoise=q['denoise'],
            temporal=False, reset=True, background=True, sharedTexture=False, time=at+(i+.5)/shutters/30)
        if shutters>1:command['shutter']=dict(key=frame_key,index=i,count=shutters)
        result, pixels=host.request(command)
        if not result.get('deferred'):
            images.append(np.frombuffer(pixels,np.uint8).reshape(h,w,4)[...,:3].copy())
        metadata.append(result)
    if len(images)!=1:raise RuntimeError('Host must resolve/denoise one output per frame')
    return images[0],dict(seconds=time.perf_counter()-start,shutterImages=shutters,
        samples=metadata[-1]['samples'],hardwareRt=all(m['hardwareRt'] for m in metadata),
        traceMs=sum(m['timings']['traceMs'] for m in metadata),postMs=sum(m['timings']['postMs'] for m in metadata),images=metadata)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--host', required=True)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--glass', type=Path, help='Exact PMS1 replay from the DN harness')
    p.add_argument('--scenes', default='glass,hero,chrome,env-room,env-default-landscape')
    p.add_argument('--presets', default=','.join(PRESETS))
    p.add_argument('--quality-size', default='320x180')
    p.add_argument('--timing-size', default='1920x1080')
    p.add_argument('--sequence', type=int, default=8)
    p.add_argument('--timing-runs', type=int, default=1)
    p.add_argument('--references','--dn-references',dest='references',type=Path,help='Explicit audited raw reference images; keep corpus/replay unchanged')
    p.add_argument('--reference-spp', type=int, default=2048)
    p.add_argument('--fog-reference-spp', type=int, default=16384)
    p.add_argument('--check', action='store_true', help='Keep DN SSIM >= .90 and DeltaE <= 4 for Production/Final')
    args = p.parse_args()
    out = args.out.resolve()
    if out == ROOT or ROOT in out.parents:
        raise ValueError('Evidence must stay outside Git')
    out.mkdir(parents=True, exist_ok=True)
    qsize, tsize = dimensions(args.quality_size), dimensions(args.timing_size)
    names = args.presets.split(',')
    if any(n not in PRESETS for n in names) or not 1 <= args.sequence <= 32 or not 1 <= args.timing_runs <= 5:
        raise ValueError('Invalid preset/sequence/run count')
    rows = []
    for scene in args.scenes.split(','):
        check_disk(out)
        host = dn.Host(args.host, out / f'{scene}-host.log')
        try:
            if scene == 'glass':
                if not args.glass:
                    raise ValueError('Glass needs an exact --glass replay')
                host.replay(args.glass)
                source = hashlib.sha256(args.glass.read_bytes()).hexdigest()
            else:
                host.request(dict(op='preset', name=scene, finish='starlight', time=0))
                source = 'scene.rs corpus at time zero, starlight'
            old_name = 'glass-0' if scene == 'glass' else scene
            reuse = args.references / f'{old_name}-reference.png' if args.references else None
            if reuse and reuse.exists():
                reference = np.array(Image.open(reuse).convert('RGB'))
                if reference.shape != (qsize[1], qsize[0], 3):
                    raise ValueError('DN reference dimensions differ')
                provenance = dict(path=str(reuse), sha256=hashlib.sha256(reuse.read_bytes()).hexdigest(),
                    source='Explicit raw reference; caller audited unchanged corpus/replay')
            else:
                spp = args.fog_reference_spp if scene == 'env-room' else args.reference_spp
                reference, meta = host.render(*qsize, spp, denoise=False, t=13)
                Image.fromarray(reference).save(out / f'{scene}-reference.png')
                provenance = dict(samples=spp, meta=meta, source='Independent raw stream at t=13')
            for name in names:
                check_disk(out)
                q = PRESETS[name]
                image, quality_timing = render(host, qsize, q, 0, q['motionBlurSamples'])
                score = dn.quality(image, reference)
                Image.fromarray(image).save(out / f'{scene}-{name}.png')
                sequence = []
                for i in range(args.sequence):
                    frame, _ = render(host, qsize, q, (i+1)/30, q['motionBlurSamples'])
                    sequence.append(frame.astype(float)/255)
                flicker = float(np.sqrt(np.mean(np.diff(np.stack(sequence), axis=0)**2))) if len(sequence)>1 else None
                # Warm the requested target separately; do not hide its setup in
                # the first measured preset. Record the warmup in the raw report.
                warmup, _ = host.request(dict(op='render',width=tsize[0],height=tsize[1],tier='final',samples=4,
                    reset=True,denoise=False,background=True,sharedTexture=False))
                _,cold=render(host,tsize,q,9/30,q['motionBlurSamples'])
                timings=[]
                for run in range(args.timing_runs):
                    hd, timing = render(host, tsize, q, (run+10)/30, q['motionBlurSamples'])
                    timings.append(timing)
                Image.fromarray(hd).save(out / f'{scene}-{name}-timed.png')
                result=dict(scene=scene,preset=name,settings=q,source=source,reference=provenance,
                    qualitySize=qsize,timingSize=tsize,quality=score,flickerRms=flicker,sequenceFrames=len(sequence),
                    qualityTiming=quality_timing,warmup=warmup,cold=cold,timings=timings,
                    seconds=float(np.median([t['seconds'] for t in timings])))
                rows.append(result)
                (out/'metrics.json').write_text(json.dumps(rows,indent=2))
                print(json.dumps({k:result[k] for k in ['scene','preset','seconds','quality','flickerRms']}),flush=True)
        finally:
            host.close()
    lines=['# Studio preset measurements','','Frozen-scene shutter integration; native host readback/filtering included, encoder excluded.',
        f'Quality {qsize[0]}x{qsize[1]}; timing {tsize[0]}x{tsize[1]}. No temporal reuse.','',
        '| Scene | Preset | Seconds/frame | SSIM | Mean DeltaE00 | Flicker RMS |', '|---|---|---:|---:|---:|---:|']
    for r in rows:
        flicker = f'{r["flickerRms"]:.5f}' if r['flickerRms'] is not None else 'unmeasured'
        lines.append(f'| {r["scene"]} | {r["preset"]} | {r["seconds"]:.3f} | {r["quality"]["ssim"]:.4f} | {r["quality"]["deltaE"]:.3f} | {flicker} |')
    (out/'report.md').write_text('\n'.join(lines)+'\n')
    if args.check:
        for r in rows:
            if r['preset'] in ['production','final']:
                assert r['quality']['ssim'] >= .90 and r['quality']['deltaE'] <= 4, r


if __name__ == '__main__':
    main()
