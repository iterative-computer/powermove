#!/usr/bin/env python3
"""Render exact native project replays and the corpus against raw high-spp frames.

Capture project inputs with the opt-in studio-denoising Playwright spec first.
Requires NumPy/Pillow, like the Cycles yardstick. All artifacts stay outside Git.
"""
import argparse
import importlib.util
import json
import os
import tempfile
from pathlib import Path
import struct
import subprocess
import time

import numpy as np
from PIL import Image, ImageDraw

spec = importlib.util.spec_from_file_location('yardstick_metrics', Path(__file__).parent / 'cycles-yardstick/metrics.py')
metrics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(metrics)


class Host:
    def __init__(self, executable, log):
        self.log = open(log, 'wb')
        self.scratch = tempfile.TemporaryDirectory(prefix='gpu-', dir=Path(log).parent)
        self.process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, env=dict(os.environ, TMPDIR=self.scratch.name))
        self.serial = 0

    def request(self, command, binary=b''):
        self.serial += 1
        payload = json.dumps(dict(command, id=self.serial)).encode()
        self.process.stdin.write(b'PMS1' + struct.pack('<II', len(payload), len(binary)) + payload + binary)
        self.process.stdin.flush()
        header = self.process.stdout.read(12)
        if len(header) != 12:
            raise RuntimeError('Studio host exited; inspect ' + self.log.name)
        magic, size, count = struct.unpack('<4sII', header)
        assert magic == b'PMS1'
        result = json.loads(self.process.stdout.read(size))
        pixels = bytearray()
        while len(pixels) < count:
            chunk = self.process.stdout.read(count - len(pixels))
            if not chunk:
                raise RuntimeError('Truncated Studio frame')
            pixels.extend(chunk)
        if not result.get('ok'):
            raise RuntimeError(result)
        return result['result'], pixels

    def replay(self, file):
        data = Path(file).read_bytes()
        offset = 0
        while offset < len(data):
            magic, size, count = struct.unpack_from('<4sII', data, offset)
            assert magic == b'PMS1'
            offset += 12
            command = json.loads(data[offset:offset + size])
            offset += size
            self.request(command, data[offset:offset + count])
            offset += count

    def render(self, w, h, samples, *, denoise=True, adaptive=False, temporal=False, t=0., budget=0):
        start = time.perf_counter()
        result, pixels = self.request(dict(op='render', width=w, height=h, tier='final', samples=samples,
                                          reset=True, denoise=denoise, adaptive=adaptive, temporal=temporal,
                                          time=t, timeBudgetMs=budget, background=True, sharedTexture=False))
        return np.frombuffer(pixels, np.uint8).reshape(h, w, 4)[..., :3].copy(), dict(result, wallSeconds=time.perf_counter() - start)

    def close(self):
        try:
            self.request(dict(op='shutdown'))
        finally:
            self.process.stdin.close()
            self.process.wait(timeout=10)
            self.log.close()
            self.scratch.cleanup()


def quality(image, reference):
    a, b = image.astype(float) / 255., reference.astype(float) / 255.
    de = metrics.delta_e2000(metrics.lab(metrics.decode(a)), metrics.lab(metrics.decode(b)))
    return dict(ssim=float(metrics.ssim_map(a, b).mean()), deltaE=float(de.mean()),
                deltaE95=float(np.percentile(de, 95)), rmse=float(np.sqrt(np.mean((a - b)**2))))


def compare_image(name, before, after, reference, out):
    h, w = reference.shape[:2]
    sheet = Image.new('RGB', (w * 3, h + 28), '#181818')
    draw = ImageDraw.Draw(sheet)
    for i, (label, pixels) in enumerate([('16 spp + previous OIDN', before), ('Adaptive Final + OIDN', after), ('Raw high-spp reference', reference)]):
        sheet.paste(Image.fromarray(pixels), (w * i, 28))
        draw.text((w * i + 6, 8), label, fill='white')
        Image.fromarray(pixels).save(out / f'{name}-{["before", "after", "reference"][i]}.png')
    sheet.save(out / f'{name}-comparison.png')
    # Glass and caustic detail, enlarged with nearest-neighbour so blur is visible.
    crop = sheet.crop((0, 28 + h // 4, w * 3, h + 28)).resize((w * 6, (h - h // 4) * 2), Image.Resampling.NEAREST)
    crop.save(out / f'{name}-crop.png')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--host', required=True)
    p.add_argument('--before-host', required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--capture', type=Path, action='append', default=[])
    p.add_argument('--corpus', nargs='*', default=['chrome', 'materials', 'env-room'])
    p.add_argument('--size', default='256x144')
    p.add_argument('--reference-spp', type=int, default=2048)
    p.add_argument('--final-spp', type=int, default=512)
    p.add_argument('--budget-ms', type=int, default=0)
    p.add_argument('--sequence', type=int, default=8)
    p.add_argument('--check', action='store_true', help='Fail on quality/flicker tolerances')
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    w, h = map(int, args.size.split('x'))
    assert 16 <= w <= 1920 and 16 <= h <= 1080 and 1 <= args.reference_spp <= 4096
    scenes = [(f'glass-{i}', file) for i, file in enumerate(args.capture)] + [(name, None) for name in args.corpus]
    results = []
    for name, capture in scenes:
        previous = Host(args.before_host, args.out / f'{name}-before.log')
        current = Host(args.host, args.out / f'{name}-after.log')
        try:
            for host in [previous, current]:
                if capture:
                    host.replay(capture)
                else:
                    host.request(dict(op='preset', name=name, finish='starlight', time=0.))
            before, timing_before = previous.render(w, h, 16)
            reference, timing_reference = current.render(w, h, args.reference_spp, denoise=False)
            after, timing_after = current.render(w, h, args.final_spp, adaptive=True, budget=args.budget_ms)
            compare_image(name, before, after, reference, args.out)
            result = dict(scene=name, size=[w, h], referenceSpp=args.reference_spp,
                          before=quality(before, reference), after=quality(after, reference),
                          timings=dict(before=timing_before, after=timing_after, reference=timing_reference))
            if args.sequence > 1:
                sequences = [[], []]
                # Freeze the evaluated scene/camera. Only random seeds advance:
                # every pixel is static, so this isolates denoising flicker.
                for i in range(args.sequence):
                    t = (i + 1) / 30.
                    for j, host in enumerate([previous, current]):
                        frame, _ = host.render(w, h, 16 if j == 0 else args.final_spp, adaptive=j == 1,
                                               temporal=j == 1, t=t, budget=args.budget_ms)
                        sequences[j].append(frame.astype(float) / 255.)
                flicker = [float(np.sqrt(np.mean(np.diff(np.stack(s), axis=0)**2))) for s in sequences]
                result['flicker'] = dict(beforeRms=flicker[0], afterRms=flicker[1], frames=args.sequence,
                                         method='RMS successive sRGB frame differences, frozen scene/camera, advancing 30fps seeds')
            results.append(result)
            (args.out / 'metrics.json').write_text(json.dumps(results, indent=2))
            print(json.dumps(result), flush=True)
        finally:
            previous.close()
            current.close()
    rows = ['# Denoising verification', '', f'Raw {args.reference_spp} spp reference; {args.final_spp} spp adaptive ceiling at {w}×{h}.', '',
            '| Scene | SSIM before → after | ΔE00 before → after | Flicker RMS before → after | Final seconds/frame |',
            '|---|---|---|---|---|']
    for r in results:
        f = r.get('flicker', {})
        rows += [f'| {r["scene"]} | {r["before"]["ssim"]:.4f} → {r["after"]["ssim"]:.4f} | {r["before"]["deltaE"]:.3f} → {r["after"]["deltaE"]:.3f} | {f.get("beforeRms", 0):.5f} → {f.get("afterRms", 0):.5f} | {r["timings"]["after"]["wallSeconds"]:.3f} |', '',
                 f'![{r["scene"]}: previous denoise, new Final, raw reference]({r["scene"]}-comparison.png)', '',
                 f'![{r["scene"]}: enlarged glass/material/caustic crop]({r["scene"]}-crop.png)', '']
    rows += ['Static sequences hold geometry and camera fixed to isolate noise flicker. The exported glass clip separately exercises actual camera motion, DOF and shutter sampling. Timing includes host round-trip and denoising, but excludes initial GPU setup unless noted in the JSON.', '']
    (args.out / 'captions.md').write_text('\n'.join(rows))
    if args.check:
        for r in results:
            assert r['after']['ssim'] >= .90, r
            assert r['after']['deltaE'] <= 4., r
            assert r['after']['rmse'] <= r['before']['rmse'] * 1.05, r
            if 'flicker' in r:
                assert r['flicker']['afterRms'] <= r['flicker']['beforeRms'] * 1.05, r


if __name__ == '__main__':
    main()
