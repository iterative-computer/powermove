import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { pointsDigest } from './point-digest.mts';

function fromBits(bits) {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, bits, true);
  return view.getFloat64(0, true);
}

test('empty coordinates keep the FNV offset basis', () => {
  assert.equal(pointsDigest([]), 'cbf29ce484222325');
});

test('only unrecorded NaN sign/payload bits are canonicalized', () => {
  const expected = pointsDigest([[NaN, 1]]);
  for (const bits of [0x7ff8000000000001n, 0xfff8000000000000n, 0x7ff0000000000001n]) {
    assert.equal(pointsDigest([[fromBits(bits), 1]]), expected);
  }
});

test('signed zero, one finite ULP, and infinity signs stay distinct', () => {
  for (const [a, b] of [[0, -0], [1, fromBits(0x3ff0000000000001n)], [Infinity, -Infinity]]) {
    assert.notEqual(pointsDigest([[a, 1]]), pointsDigest([[b, 1]]));
  }
});

test('readable frozen coordinates retain every stored digest', () => {
  const vectors = JSON.parse(readFileSync(new URL('./geometry-vectors.json', import.meta.url), 'utf8'));
  const decode = value => typeof value === 'number' ? value : Number(value);
  let compared = 0;
  for (const entry of vectors.paths.flatMap(entry => entry.paths)) {
    if (!entry.points) continue;
    assert.equal(pointsDigest(entry.points.map(point => point.map(decode))), entry.pointsDigest, entry.id);
    compared++;
  }
  assert.ok(compared > 20);
});
