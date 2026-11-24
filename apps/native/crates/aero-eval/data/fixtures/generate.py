#!/usr/bin/env python3
"""Generate the Aero eval fixtures. Python 3.9+, standard library only.

Everything here is synthetic and public: stills are drawn by this script, the
only video is the public engine fixture engine/fixtures/h264-aac.mp4 (160x90,
red then green, 2 s), and the broken extensions are written below. Projects
are PMV4 containers packed the way
apps/native/crates/acceptance/data/fixtures/generate-portable.py packs them.

Run from anywhere:  python3 generate.py
Writes projects/, extensions/, ../references/ and manifest.json (pins).
"""
import hashlib
import json
import shutil
import struct
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[5]
W, H = 1280, 720
CLIP = REPO / 'engine' / 'fixtures' / 'h264-aac.mp4'

DAWN = (232, 116, 59)
DUSK = (31, 138, 138)


# ---------------------------------------------------------------- stills

def png(width, height, rows):
    """8-bit RGB PNG; every row uses the Up filter (gradients compress well)."""
    raw = bytearray()
    prev = bytes(width * 3)
    for row in rows:
        raw.append(2)
        raw += bytes((row[i] - prev[i]) & 0xFF for i in range(len(row)))
        prev = row
    chunk = lambda kind, data: (struct.pack('>I', len(data)) + kind + data
                                + struct.pack('>I', zlib.crc32(kind + data) & 0xFFFFFFFF))
    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(raw), 9))
            + chunk(b'IEND', b''))


def solid(rgb):
    row = bytes(rgb) * W
    return png(W, H, [row] * H)


CHART = [
    [(24, 24, 24), (60, 60, 60), (100, 100, 100), (140, 140, 140), (185, 185, 185), (232, 232, 232)],
    [(176, 52, 48), (64, 148, 70), (52, 80, 168), (54, 160, 170), (168, 64, 150), (214, 190, 60)],
    [(118, 82, 66), (196, 148, 128), (96, 122, 156), (90, 108, 64), (130, 128, 176), (102, 188, 172)],
    [(12, 12, 12), (40, 40, 40), (80, 80, 80), (160, 160, 160), (210, 210, 210), (250, 250, 250)],
]


def chart_rows(transform=lambda c: c):
    gap, cols, rows_n = 16, 6, 4
    pw, ph = (W - gap * (cols + 1)) // cols, (H - gap * (rows_n + 1)) // rows_n
    border = transform((18, 18, 18))
    out = []
    for y in range(H):
        r, yy = divmod(y - gap, ph + gap)
        row = bytearray()
        for x in range(W):
            c, xx = divmod(x - gap, pw + gap)
            inside = 0 <= r < rows_n and 0 <= c < cols and yy < ph and xx < pw and x >= gap and y >= gap
            row += bytes(transform(CHART[r][c]) if inside else border)
        out.append(bytes(row))
    return out


def cast(c):
    """The plate's problem: lifted blacks, low contrast, a cool cast."""
    lift = [0.08 + 0.80 * v / 255 for v in c]
    gains = (0.90, 1.0, 1.08)
    return tuple(max(0, min(255, round(v * g * 255))) for v, g in zip(lift, gains))


def hero_rows():
    """A synthetic dusk scene: sky gradient, horizon, sun, dark ground."""
    out = []
    horizon = int(H * 0.68)
    sun = (int(W * 0.66), int(H * 0.52), 70)
    for y in range(H):
        row = bytearray()
        if y < horizon:
            t = y / horizon
            sky = (int(40 + 200 * t), int(60 + 90 * t), int(140 - 40 * t))
        else:
            t = (y - horizon) / (H - horizon)
            sky = (int(50 - 30 * t), int(44 - 24 * t), int(36 - 20 * t))
        dy = y - sun[1]
        for x in range(W):
            dx = x - sun[0]
            if y < horizon and dx * dx + dy * dy < sun[2] ** 2:
                row += bytes((255, 214, 150))
            else:
                row += bytes(sky)
        out.append(bytes(row))
    return out


# ---------------------------------------------------------------- projects

def layer(id_, type_, name, from_, dur, d, **extra):
    l = dict(id=id_, type=type_, name=name, **{'from': from_}, dur=dur, on=True,
             lock=False, shy=False, collapsed=False, color='#6a8dff', blend='normal',
             mblur=False, parent=None, fx=[], masks=[], d=d, locked_intent={})
    l.update(extra)
    return l


def image(id_, name, asset, from_=0, dur=6, **extra):
    return layer(id_, 'image', name, from_, dur, dict(asset=asset, fit='cover', w=W, h=H), **extra)


def clip(id_, name, from_, dur=2, trim=0):
    return layer(id_, 'video', name, from_, dur,
                 dict(asset='clip', fit='cover', w=160, h=90, trim=trim, speed=1))


def text(id_, name, body, from_=0, dur=6, size=120):
    return layer(id_, 'text', name, from_, dur,
                 dict(text=body, font='Helvetica Neue', size=size, weight=700,
                      color='#ffffff', align='center'))


def effect(id_, type_):
    return dict(id=id_, type=type_, on=True, p={})


def project(id_, name, dur, layers, assets, markers=()):
    return dict(id=id_, name=name, compId=id_ + '-main', compName=name, w=W, h=H, fps=30,
                dur=dur, bg='#000000', layers=layers, assets=assets, comps={},
                markers=list(markers), work=[0, dur], params={}, revision=1, edits=[],
                created=0)


def pack(proj, media):
    """PMV4: magic, document, media bytes, index, 64-byte footer."""
    enc = lambda v: json.dumps(v, ensure_ascii=False, separators=(',', ':')).encode()
    document = enc(dict(proj=proj))
    rec = lambda data, at: dict(offset=at, length=len(data), sha256=hashlib.sha256(data).hexdigest())
    body = b'PMV4\n' + document
    index_media = []
    for ident, mime, data in media:
        index_media.append(dict(id=ident, revision='synthetic-v1', type=mime, **rec(data, len(body))))
        body += data
    index = enc(dict(version=4, document=rec(document, 5), media=index_media))
    footer = (b'PMV4END\n' + struct.pack('<dI', len(body), len(index)) + bytes(12)
              + hashlib.sha256(index).digest())
    return body + index + footer


def still_asset(id_, name):
    return {id_: dict(id=id_, kind='image', name=name, w=W, h=H)}


def build():
    chart = png(W, H, chart_rows())
    chart_cast = png(W, H, chart_rows(cast))
    hero = png(W, H, hero_rows())
    dawn, dusk = solid(DAWN), solid(DUSK)
    clip_bytes = CLIP.read_bytes()

    promo_layers = lambda: [image('bg', 'Background plate', 'hero'),
                            text('title', 'Title', 'SUMMER DROP')]
    promo_media = [('hero', 'image/png', hero)]
    promo_assets = still_asset('hero', 'Dusk plate.png')

    out = {}
    out['interview.pmv'] = pack(project('interview', 'Interview', 12, [
        text('lower', 'Lower third', 'Dana Reyes — Director', 0.5, 4, size=48),
        clip('t1', 'Take 1', 0), clip('t2', 'Take 2', 2), clip('t3', 'Take 3', 4),
        clip('broll', 'B-roll', 6), clip('t4', 'Take 4', 8),
    ], {'clip': dict(id='clip', kind='video', name='Interview take.mp4', w=160, h=90, dur=2)}),
        [('clip', 'video/mp4', clip_bytes)])
    out['speed.pmv'] = pack(project('speed', 'Speed ramp', 8, [
        clip('clip1', 'Clip', 0, 2), clip('clip2', 'Clip 2', 2, 2),
    ], {'clip': dict(id='clip', kind='video', name='Street.mp4', w=160, h=90, dur=2)}),
        [('clip', 'video/mp4', clip_bytes)])
    out['chart-plate.pmv'] = pack(project('chart-plate', 'Chart plate', 4, [
        image('plate', 'Plate', 'chart', dur=4)], still_asset('chart', 'Chart.png')),
        [('chart', 'image/png', chart)])
    out['grade-match.pmv'] = pack(project('grade-match', 'Grade match', 4, [
        image('ref', 'Reference', 'chart', dur=4, on=False),
        image('plate', 'Plate', 'cast', dur=4),
    ], {**still_asset('chart', 'Reference still.png'), **still_asset('cast', 'Plate.png')}),
        [('chart', 'image/png', chart), ('cast', 'image/png', chart_cast)])
    mono = image('plate', 'Plate', 'chart', dur=4)
    mono['fx'] = [effect('fx-mono', 'mono.effect')]
    out['chart-mono.pmv'] = pack(project('chart-mono', 'Chart mono', 4, [mono],
                                         still_asset('chart', 'Chart.png')),
                                 [('chart', 'image/png', chart)])
    out['promo.pmv'] = pack(project('promo', 'Promo', 6, promo_layers(), promo_assets), promo_media)
    glow = promo_layers()
    glow[1]['fx'] = [effect('fx-glow', 'softglow.effect')]
    out['glow-title.pmv'] = pack(project('glow-title', 'Glow title', 6, glow, promo_assets), promo_media)
    out['markers.pmv'] = pack(project('markers', 'Markers', 6, promo_layers(), promo_assets, [
        dict(id='m-intro', t=0, name='Intro'), dict(id='m-drop', t=2.5, name='Drop'),
        dict(id='m-outro', t=5, name='Outro')]), promo_media)
    out['cards.pmv'] = pack(project('cards', 'Two shots', 6, [
        image('a', 'Shot A', 'dawn', 0, 3), image('b', 'Shot B', 'dusk', 3, 3),
    ], {**still_asset('dawn', 'Dawn.png'), **still_asset('dusk', 'Dusk.png')}),
        [('dawn', 'image/png', dawn), ('dusk', 'image/png', dusk)])

    projects = HERE / 'projects'
    projects.mkdir(exist_ok=True)
    for name, data in out.items():
        (projects / name).write_bytes(data)
    refs = HERE.parent / 'references' / 'grade-match'
    refs.mkdir(parents=True, exist_ok=True)
    (refs / 'chart.png').write_bytes(chart)
    write_extensions()
    pins = {f'projects/{n}': hashlib.sha256(d).hexdigest() for n, d in sorted(out.items())}
    for ext in sorted((HERE / 'extensions').iterdir()):
        pins[f'extensions/{ext.name}'] = tree_hash(ext)
    manifest = dict(format='aero-eval-fixtures', note='written by generate.py; do not edit',
                    pins=pins)
    (HERE / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


# ---------------------------------------------------------------- broken extensions

EXTENSIONS = {
    # Compile error: the shader calls luma(), which is never defined.
    'softglow': ({
        'id': 'softglow', 'name': 'Soft Glow', 'version': '1.2.0', 'apiVersion': 3,
        'permissions': [], 'description': 'A soft bloom around bright edges.',
        'contributes': ['effects'], 'author': 'user'}, """import type { EffectDefinition, PowermoveAPI } from 'powermove';

export const definition: EffectDefinition = {
  id: 'softglow.effect', label: 'Soft Glow', group: 'Light & Shadow', passes: 1,
  params: [
    { k: 'threshold', label: 'Threshold', def: 0.6, min: 0, max: 1, step: 0.01 },
    { k: 'radius', label: 'Radius', def: 12, min: 0, max: 64, step: 0.5, unit: 'px' },
    { k: 'intensity', label: 'Intensity', def: 0.8, min: 0, max: 2, step: 0.01 }
  ],
  frag: `
    vec4 c = texture(u_tex, v_st);
    vec2 px = u_radius / u_res;
    vec3 sum = vec3(0.0);
    for (int i = 0; i < 8; i++) {
      float a = float(i) * 0.7853982;
      vec4 s = texture(u_tex, v_st + vec2(cos(a), sin(a)) * px);
      sum += s.rgb * smoothstep(u_threshold, 1.0, luma(s.rgb));
    }
    o = vec4(c.rgb + sum / 8.0 * u_intensity * c.a, c.a);
  `
};

export default function activate(api: PowermoveAPI) {
  api.effects.register(definition);
}
"""),
    # Wrong output: the luma is subtracted from alpha, so the picture is a negative.
    'mono': ({
        'id': 'mono', 'name': 'Mono', 'version': '1.0.3', 'apiVersion': 3,
        'permissions': [], 'description': 'Black and white.',
        'contributes': ['effects'], 'author': 'user'}, """import type { EffectDefinition, PowermoveAPI } from 'powermove';

export const definition: EffectDefinition = {
  id: 'mono.effect', label: 'Mono', group: 'Color',
  params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 1, step: 0.01 }],
  frag: `
    vec4 c = texture(u_tex, v_st);
    float y = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
    vec3 mono = vec3(c.a - y);
    o = vec4(mix(c.rgb, mono, u_amount), c.a);
  `
};

export default function activate(api: PowermoveAPI) {
  api.effects.register(definition);
}
"""),
    # Runtime error: the panel reads project.comp.layers; a project has no
    # `comp`, so building the panel throws a TypeError.
    'shot-notes': ({
        'id': 'shot-notes', 'name': 'Shot Notes', 'version': '0.4.0', 'apiVersion': 3,
        'permissions': [], 'description': 'A note for every layer.',
        'contributes': ['panels'], 'author': 'user'}, """import type { PowermoveAPI } from 'powermove';

const notes = new Map<string, string>();

export default function activate(api: PowermoveAPI) {
  api.panels.register({
    id: 'shot-notes.panel',
    title: 'Shot Notes',
    icon: 'note',
    build(body) {
      const project = api.project.get() as any;
      const list = document.createElement('div');
      for (const layer of project.comp.layers) {
        const row = document.createElement('label');
        row.textContent = layer.name;
        const note = document.createElement('input');
        note.value = notes.get(layer.id) ?? '';
        note.placeholder = 'Note';
        note.oninput = () => notes.set(layer.id, note.value);
        row.append(note);
        list.append(row);
      }
      body.append(list);
    }
  });
}
"""),
}


def write_extensions():
    root = HERE / 'extensions'
    if root.exists():
        shutil.rmtree(root)
    for name, (manifest, source) in EXTENSIONS.items():
        d = root / name
        d.mkdir(parents=True)
        (d / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        (d / 'index.ts').write_text(source)


def tree_hash(path):
    """sha256 over sorted 'relpath\\0sha256\\n' lines; dotfiles skipped (as in Rust)."""
    lines = []
    for p in sorted(path.rglob('*')):
        if p.is_file() and not any(part.startswith('.') for part in p.relative_to(path).parts):
            rel = p.relative_to(path).as_posix()
            lines.append((rel, hashlib.sha256(p.read_bytes()).hexdigest()))
    lines.sort()
    return hashlib.sha256(''.join(f'{r}\0{s}\n' for r, s in lines).encode()).hexdigest()


if __name__ == '__main__':
    build()
