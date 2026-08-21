const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

/* Load exporter.js with a minimal PM stub — only the muxer path is exercised. */
function exporter() {
  const PM = {
    version: 'test',
    h: () => ({}),
    proj: { name: 't', fps: 30 },
    store: { get: () => ({}), set() {} },
    toast() {}, download() {}, modal() {}, row() { return {}; }, selectField() {}, toggleField() {},
    round: (v) => v, tc: () => '0:00', renderFrameTo: () => null,
    GL: { canvas: { width: 2, height: 2 }, resize() {}, render() {} },
  };
  const context = vm.createContext({ window: { PM }, console, setTimeout });
  /* vm contextify breaks `new` on host classes — hand it a plain subclass instead */
  context.TextEncoder = class extends TextEncoder { };
  context.TextDecoder = class extends TextDecoder { };
  context.Blob = Blob;
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/exporter.js'), 'utf8'), context);
  return PM.Export;
}

function u32(b, o) { return (b[o] << 24 | b[o + 1] << 16 | b[o + 2] << 8 | b[o + 3]) >>> 0; }

test('muxWebM emits a valid EBML header and segment for video-only input', async () => {
  const X = exporter();
  const blob = X.muxWebM(
    [{ ts: 0, key: true, data: new Uint8Array([1, 2, 3]) }, { ts: 33333, key: false, data: new Uint8Array([4]) }],
    { width: 1920, height: 1080, fps: 30 },
  );
  const buf = new Uint8Array(await blob.arrayBuffer());
  assert.equal(buf[0], 0x1a); assert.equal(buf[1], 0x45); assert.equal(buf[2], 0xdf); assert.equal(buf[3], 0xa3), 'EBML magic';
  const segIdx = buf.indexOf(0x18, 40);
  // find Segment id bytes 0x18538067
  let seg = -1;
  for (let i = 30; i < buf.length - 4; i++) {
    if (buf[i] === 0x18 && buf[i + 1] === 0x53 && buf[i + 2] === 0x80 && buf[i + 3] === 0x67) { seg = i; break; }
  }
  assert.ok(seg > 0, 'segment element present');
});

test('muxWebM interleaves an opus audio track with correct track numbers', async () => {
  const X = exporter();
  const audio = {
    chunks: [{ ts: 0, data: new Uint8Array([9]) }, { ts: 20000, data: new Uint8Array([8]) }],
    priv: new Uint8Array([0x4f, 0x70]), rate: 48000, channels: 2,
  };
  const blob = X.muxWebM(
    [{ ts: 0, key: true, data: new Uint8Array([1]) }, { ts: 10000, key: false, data: new Uint8Array([2]) }],
    { width: 64, height: 64, fps: 30, audio },
  );
  const buf = new Uint8Array(await blob.arrayBuffer());
  const text = Buffer.from(buf).toString('latin1');
  assert.ok(text.includes('A_OPUS'), 'audio codec declared');
  assert.ok(text.includes('V_VP9'), 'video codec declared');
  /* SimpleBlock headers: track byte must appear as 0x81/0x82 vints */
  let t81 = 0, t82 = 0;
  for (let i = 0; i < buf.length - 4; i++) {
    if (buf[i] === 0xa3) { // SimpleBlock
      const sz = buf[i + 1];
      if (buf[i + 2] === 0x81) t81++;
      if (buf[i + 2] === 0x82) t82++;
    }
  }
  assert.equal(t81, 2, 'two video blocks');
  assert.equal(t82, 2, 'two audio blocks');
});

test('blocks beyond a 2s window start a new cluster so relative timecodes stay in range', async () => {
  const X = exporter();
  const frames = [];
  for (let i = 0; i < 90; i++) frames.push({ ts: i * 33333, key: i % 60 === 0, data: new Uint8Array([i & 0xff]) });
  const blob = X.muxWebM(frames, { width: 16, height: 16, fps: 30 });
  const buf = new Uint8Array(await blob.arrayBuffer());
  let clusters = 0;
  for (let i = 0; i < buf.length - 4; i++) {
    if (buf[i] === 0x1f && buf[i + 1] === 0x43 && buf[i + 2] === 0xb6 && buf[i + 3] === 0x75) clusters++;
  }
  assert.ok(clusters >= 2, `expected multiple clusters, got ${clusters}`);
});
