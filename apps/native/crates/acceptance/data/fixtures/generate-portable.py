#!/usr/bin/env python3
"""Generate the public synthetic packed-byte fixture. Python 3.9+, no codecs."""
import base64
import hashlib
import json
from pathlib import Path
import struct


def generate():
    project = dict(id='packed-synthetic', name='Synthetic packed media', w=32, h=18,
                   fps=29.97, dur=2, layers=[dict(id='image', type='image',
                   name='Synthetic', **{'from': 0}, dur=2,
                   d=dict(asset='image-bytes', fit='contain', w=1, h=1))],
                   assets={'image-bytes': dict(id='image-bytes', kind='image', name='Synthetic.png')},
                   comps={}, unknown=dict(exact=1.23456789012345, text='λ'))
    encode = lambda v: json.dumps(v, ensure_ascii=False, separators=(',', ':')).encode()
    document = encode(dict(proj=project, unknownEnvelope=dict(keep=True)))
    png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==')
    chunks = [('image-bytes', 'image/png', png),
              ('opaque-unreachable', 'application/octet-stream', bytes(range(256)) * 4)]
    record = lambda data, offset: dict(offset=offset, length=len(data), sha256=hashlib.sha256(data).hexdigest())
    body = b'PMV4\n' + document
    media = []
    for identity, kind, data in chunks:
        media.append(dict(id=identity, revision='synthetic-v1', type=kind, **record(data, len(body))))
        body += data
    index = encode(dict(version=4, document=record(document, 5), media=media))
    footer = b'PMV4END\n' + struct.pack('<dI', len(body), len(index)) + bytes(12) + hashlib.sha256(index).digest()
    return body + index + footer


if __name__ == '__main__':
    Path(__file__).with_name('portable-media.pmv').write_bytes(generate())
