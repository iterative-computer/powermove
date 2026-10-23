#!/usr/bin/env python3
"""Regenerate the source render steps with pinned Skia and Dawn/Tint tools.

Arguments: graphite-shader-oracle (see ../graphite-shader-oracle.cpp), tint.
Skia: 747cf654ed6f864f4402e7d6eb54a6ede60141e5 (m152).
Dawn: 571752c9cb3ed36a3194854374984427794c4dda (Electron 44 / Chromium 152).
"""
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parents[2] / 'pm-render/src/text/graphite'
OUT.mkdir(parents=True, exist_ok=True)
SKIA, TINT = map(str, map(pathlib.Path.resolve, map(pathlib.Path, sys.argv[1:])))
HEADER = ('// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.\n'
          '// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.\n')
METAL = ('// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.\n'
         '#pragma METAL fp math_mode(relaxed)\n')
with tempfile.TemporaryDirectory() as tmp:
    for name, kind in [('analytic', 'vertex'), ('analytic', 'fragment'), ('fill', 'vertex'),
                       ('stroke', 'vertex'), ('sdf', 'vertex'), ('sdf', 'fragment')]:
        ext = 'vert' if kind == 'vertex' else 'frag'
        stem = name + ('-vs' if kind == 'vertex' else '-fs')
        prefix = pathlib.Path(tmp) / stem
        subprocess.run([SKIA, kind, str(HERE / f'{name}.{ext}.sksl'), str(prefix), 'force-f32'], check=True)
        wgsl = prefix.with_suffix('.wgsl').read_text()
        # Naga does not know Chromium's diagnostic rule. It changes no shader code.
        wgsl = wgsl.replace('diagnostic(off, chromium.unreachable_code);\n', '')
        # SkSL assigns separate sampler/texture IDs. The single atlas occupies
        # the explicit common layout's texture and sampler bindings.
        for atlas in range(4):
            wgsl = wgsl.replace(f'@binding({10001+2*atlas})', f'@binding({3+2*atlas})')
            wgsl = wgsl.replace(f'@binding({10002+2*atlas})', f'@binding({2+2*atlas})')
        (OUT / f'{stem}.wgsl').write_text(HEADER + wgsl)
    for stem in ['analytic-vs', 'analytic-fs', 'fill-vs', 'stroke-vs', 'sdf-vs', 'sdf-fs',
                 'primitive-vs', 'primitive-fs']:
        source = 'primitive' if stem.startswith('primitive') else stem
        entry = 'primitive_vs' if stem == 'primitive-vs' else 'primitive_fs' if stem == 'primitive-fs' else 'main'
        dest = pathlib.Path(tmp) / f'{stem}.metal'
        # Bounds are guaranteed by the host's vertex/instance ranges. Avoid
        # a buffer-length binding for the five-vec4-per-instance patch packet.
        subprocess.run([TINT, str(OUT / f'{source}.wgsl'), '--format', 'msl',
                        '--entry-point', entry, '--rename-all', 'true',
                        '--disable-robustness', 'true', '-o', str(dest)], check=True)
        msl = dest.read_text()
        if stem == 'sdf-fs':
            # Tint's standalone writer compacts the sole active buffer to
            # slot 0. The shared pipeline layout reserves slot 0 for the
            # intrinsic uniforms and slot 1 for the combined storage data.
            msl = msl.replace('[[buffer(0)]]', '[[buffer(1)]]')
        (OUT / f'{stem}.metal').write_text(HEADER + METAL + msl)
