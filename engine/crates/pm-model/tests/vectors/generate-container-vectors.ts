/**
 * Writes tests/fixtures/containers/*.pmv and tests/vectors/container-vectors.json
 * with the app's own encoders and decoder (src/shared/project-container.ts and
 * project-incremental.ts). For each file the expectations record what
 * `decodeProjectContainer` returns (format, canonical document hash, media
 * records) or the message it throws. The Rust reader must agree on all of them.
 *
 *   bun engine/crates/pm-model/tests/vectors/generate-container-vectors.ts
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { canonicalHash } from '../../../../../apps/desktop/src/shared/canonical-json';
import { decodeProjectContainer, encodeProjectContainer, isProjectContainer } from '../../../../../apps/desktop/src/shared/project-container';
import { INCREMENTAL_MAGIC, isIncrementalProject, projectFooter, type ProjectIndex } from '../../../../../apps/desktop/src/shared/project-incremental';
import { sha256Hex } from '../../../../../apps/desktop/src/shared/sha256';

const here = import.meta.dir;
const outDir = path.join(here, '..', 'fixtures', 'containers');
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const enc = new TextEncoder();

const project = {
  id: 'P', name: 'Container test', compId: 'C1', compName: 'Main', w: 64, h: 36, fps: 29.97, dur: 2, bg: '#000000',
  layers: [{ id: 'a', type: 'image', name: 'Still', from: 0, dur: 2, d: { asset: 'img', fit: 'contain', w: 64, h: 36 } }],
  assets: { img: { id: 'img', name: 'still.png', kind: 'image' }, clip: { id: 'clip', name: 'clip.webm', kind: 'video' } },
  comps: {}, markers: [], work: [0, 2], text: 'lone \ud800 surrogate and 😀',
};
const document = { proj: project, history: { entries: [], index: 0 }, v: 1 };
const mediaA = new Uint8Array([137, 80, 78, 71, 1, 2, 3]);
const mediaB = new Uint8Array(1000).map((_, i) => (i * 7) & 255);

type Pmv4Options = { mutateIndex?: (index: any) => void; corrupt?: (bytes: Uint8Array, layout: any) => void; footer?: (footer: Uint8Array) => void; documentText?: string };
function pmv4(options: Pmv4Options = {}): Uint8Array {
  const doc = enc.encode(options.documentText ?? JSON.stringify(document));
  const index: ProjectIndex = {
    version: 4, document: { offset: 5, length: doc.length, sha256: sha256Hex(doc) },
    media: [
      { id: 'img', revision: 'r1', type: 'image/png', offset: 5 + doc.length, length: mediaA.length, sha256: sha256Hex(mediaA) },
      { id: 'clip', revision: 'r2', type: 'video/webm', offset: 5 + doc.length + mediaA.length + 3, length: mediaB.length, sha256: sha256Hex(mediaB) },
    ],
  };
  options.mutateIndex?.(index);
  const raw = enc.encode(JSON.stringify(index));
  const indexOffset = 5 + doc.length + mediaA.length + 3 + mediaB.length;
  const footer = projectFooter(indexOffset, raw.length, sha256Hex(raw));
  options.footer?.(footer);
  const bytes = new Uint8Array(indexOffset + raw.length + footer.length);
  bytes.set(INCREMENTAL_MAGIC); bytes.set(doc, 5); bytes.set(mediaA, 5 + doc.length);
  bytes.set(mediaB, 5 + doc.length + mediaA.length + 3); bytes.set(raw, indexOffset); bytes.set(footer, indexOffset + raw.length);
  options.corrupt?.(bytes, { docLength: doc.length, indexOffset });
  return bytes;
}

function pmv3Raw(header: string, body = new Uint8Array(0), declaredLength?: number): Uint8Array {
  const h = enc.encode(header);
  const out = new Uint8Array(9 + h.length + body.length);
  out.set(enc.encode('PMV3\n'));
  new DataView(out.buffer).setUint32(5, declaredLength ?? h.length, true);
  out.set(h, 9); out.set(body, 9 + h.length);
  return out;
}

const files: Record<string, Uint8Array> = {
  'json-plain.pmv': enc.encode(JSON.stringify(project)),
  'json-bom-invalid-utf8.pmv': new Uint8Array([0xef, 0xbb, 0xbf, ...enc.encode('{"proj":{"name":"a'), 0xff, ...enc.encode('b","layers":[]}}')]),
  'json-invalid.pmv': enc.encode('{"proj":'),
  'tiny.pmv': enc.encode('PM'),
  'pmv3.pmv': encodeProjectContainer(document, [{ id: 'img', type: 'image/png', data: mediaA }, { id: 'clip', type: 'video/webm', data: mediaB }]),
  'pmv3-no-media.pmv': encodeProjectContainer(project, []),
  'pmv3-truncated.pmv': enc.encode('PMV3\n\x02\x00'),
  'pmv3-short-header.pmv': pmv3Raw('{', new Uint8Array(0), 1),
  'pmv3-header-overrun.pmv': pmv3Raw('{"document":{"a":1},"media":[]}', new Uint8Array(0), 500),
  'pmv3-no-document.pmv': pmv3Raw('{"document":0,"media":[]}'),
  'pmv3-media-not-array.pmv': pmv3Raw('{"document":{"a":1},"media":{}}'),
  'pmv3-duplicate-ids.pmv': pmv3Raw('{"document":{"a":1},"media":[{"id":"x","type":"t","offset":0,"length":1},{"id":"x","type":"t","offset":1,"length":1}]}', new Uint8Array([1, 2])),
  'pmv3-range-overflow.pmv': pmv3Raw('{"document":{"a":1},"media":[{"id":"x","type":"t","offset":1,"length":2}]}', new Uint8Array([1, 2])),
  'pmv3-fractional-length.pmv': pmv3Raw('{"document":{"a":1},"media":[{"id":"x","type":"t","offset":0,"length":1.5}]}', new Uint8Array([1, 2])),
  'pmv3-negative-zero-offset.pmv': pmv3Raw('{"document":{"a":1},"media":[{"id":"x","type":"t","offset":-0,"length":2}]}', new Uint8Array([1, 2])),
  'pmv3-array-header.pmv': pmv3Raw('[1,2]'),
  'pmv3-bad-json.pmv': pmv3Raw('{"document":'),
  'pmv4.pmv': pmv4(),
  'pmv4-no-media.pmv': pmv4({ mutateIndex: index => { index.media = []; } }),
  'pmv4-short.pmv': new Uint8Array([...INCREMENTAL_MAGIC, 1, 2, 3]),
  'pmv4-bad-end-magic.pmv': pmv4({ footer: footer => { footer[0] = 0x51; } }),
  'pmv4-bad-index-checksum.pmv': pmv4({ footer: footer => { footer[40] ^= 1; } }),
  'pmv4-index-offset-mismatch.pmv': pmv4({ footer: footer => { new DataView(footer.buffer).setFloat64(8, 6, true); } }),
  'pmv4-index-length-huge.pmv': pmv4({ footer: footer => { new DataView(footer.buffer).setUint32(16, 0xffffffff, true); } }),
  'pmv4-index-offset-fraction.pmv': pmv4({ footer: footer => { new DataView(footer.buffer).setFloat64(8, 1e300, true); } }),
  'pmv4-wrong-version.pmv': pmv4({ mutateIndex: index => { index.version = '4'; } }),
  'pmv4-document-too-short.pmv': pmv4({ mutateIndex: index => { index.document.length = 1; } }),
  'pmv4-bad-hash-format.pmv': pmv4({ mutateIndex: index => { index.media[0].sha256 = index.media[0].sha256.toUpperCase(); } }),
  'pmv4-duplicate-media.pmv': pmv4({ mutateIndex: index => { index.media[1].id = 'img'; } }),
  'pmv4-empty-revision.pmv': pmv4({ mutateIndex: index => { index.media[1].revision = ''; } }),
  'pmv4-long-id.pmv': pmv4({ mutateIndex: index => { index.media[1].id = '😀'.repeat(501); } }),
  'pmv4-id-at-limit.pmv': pmv4({ mutateIndex: index => { index.media[1].id = '😀'.repeat(500); } }),
  'pmv4-overlap.pmv': pmv4({ mutateIndex: index => { index.media[1].offset = index.media[0].offset + 1; } }),
  'pmv4-range-past-index.pmv': pmv4({ mutateIndex: index => { index.media[1].length += 10_000; } }),
  'pmv4-zero-length-overlap-ok.pmv': pmv4({ mutateIndex: index => { index.media[1].offset = index.media[0].offset + 1; index.media[1].length = 0; index.media[1].sha256 = sha256Hex(new Uint8Array(0)); } }),
  'pmv4-document-corrupt.pmv': pmv4({ corrupt: bytes => { bytes[8] ^= 1; } }),
  'pmv4-media-corrupt.pmv': pmv4({ corrupt: (bytes, layout) => { bytes[5 + layout.docLength + 1] ^= 1; } }),
  'pmv4-document-not-json.pmv': pmv4({ documentText: '{"proj":' }),
  'pmv4-bare-project.pmv': pmv4({ documentText: JSON.stringify(project) }),
};

const expectations = Object.entries(files).map(([name, bytes]) => {
  writeFileSync(path.join(outDir, name), bytes);
  const format = isIncrementalProject(bytes) ? 'pmv4' : isProjectContainer(bytes) ? 'pmv3' : 'json';
  try {
    const decoded = decodeProjectContainer(bytes);
    // Absolute offsets: decodeProjectContainer returns subarrays of `bytes`.
    const media = decoded.media.map((item: any) => ({
      id: item.id, type: item.type, offset: item.data.byteOffset, length: item.data.byteLength,
      revision: item.revision ?? null, sha256: item.sha256 ?? null, dataSha256: sha256Hex(item.data),
    }));
    return { name, ok: { format, documentHash: canonicalHash(decoded.document), media } };
  } catch (error) {
    const message = (error as Error).message;
    return { name, error: error instanceof SyntaxError ? 'json' : message };
  }
});
writeFileSync(path.join(here, 'container-vectors.json'), JSON.stringify({ generatedBy: 'engine/crates/pm-model/tests/vectors/generate-container-vectors.ts', files: expectations }, null, 1) + '\n');
console.log(`wrote ${expectations.length} container files (${expectations.filter(e => 'error' in e).length} rejected)`);
