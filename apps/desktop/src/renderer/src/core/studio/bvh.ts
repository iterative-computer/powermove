/*
 * The Studio BVH, built on the CPU in the layout trace.wgsl reads: binned
 * surface-area-heuristic splits, then "wide" nodes that store both children's
 * bounds (64 bytes each). Mirrors engine/crates/pm-studio/src/bvh.rs.
 */

export type Bvh = { nodes: ArrayBuffer; order: Uint32Array };

const BINS = 16;
const EMPTY_LEAF = 0xffffffff;

/** A growable array of binary nodes: bounds (6 floats), leftFirst, count. */
class Nodes {
  bounds = new Float64Array(6 * 64);
  left = new Uint32Array(64);
  count = new Uint32Array(64);
  size = 0;
  push(): number {
    if (this.size === this.left.length) {
      const grow = <A extends Float64Array | Uint32Array>(a: A, n: number): A => { const b = new (a.constructor as any)(n) as A; b.set(a); return b; };
      this.bounds = grow(this.bounds, this.bounds.length * 2);
      this.left = grow(this.left, this.left.length * 2);
      this.count = grow(this.count, this.count.length * 2);
    }
    return this.size++;
  }
}

const area = (b: Float64Array, o = 0): number => {
  const ex = b[o + 3]! - b[o]!, ey = b[o + 4]! - b[o + 1]!, ez = b[o + 5]! - b[o + 2]!;
  return ex < 0 ? 0 : ex * ey + ey * ez + ez * ex;
};
const reset = (b: Float64Array, o = 0) => { b.fill(Infinity, o, o + 3); b.fill(-Infinity, o + 3, o + 6); };

/** `corners` holds 9 floats per triangle. Returns wide nodes and the triangle order. */
export function buildBvh(corners: Float32Array): Bvh {
  const count = corners.length / 9;
  const lo = new Float64Array(count * 3), hi = new Float64Array(count * 3), ce = new Float64Array(count * 3);
  for (let t = 0; t < count; t++) {
    for (let a = 0; a < 3; a++) {
      const x = corners[t * 9 + a]!, y = corners[t * 9 + 3 + a]!, z = corners[t * 9 + 6 + a]!;
      const mn = Math.min(x, y, z), mx = Math.max(x, y, z);
      lo[t * 3 + a] = mn; hi[t * 3 + a] = mx; ce[t * 3 + a] = (mn + mx) * 0.5;
    }
  }
  const order = new Uint32Array(count);
  for (let i = 0; i < count; i++) order[i] = i;
  const nodes = new Nodes();
  nodes.push(); nodes.push(); // root, plus a pad so children come in pairs
  const stack: number[] = [0, 0, count];
  const b = new Float64Array(6), cb = new Float64Array(6), acc = new Float64Array(6);
  const bins = new Float64Array(BINS * 6), binN = new Int32Array(BINS), leftA = new Float64Array(BINS), leftN = new Int32Array(BINS);
  while (stack.length) {
    const n = stack.pop()!, first = stack.pop()!, index = stack.pop()!;
    reset(b); reset(cb);
    for (let k = first; k < first + n; k++) {
      const t = order[k]!;
      for (let a = 0; a < 3; a++) {
        b[a] = Math.min(b[a]!, lo[t * 3 + a]!); b[3 + a] = Math.max(b[3 + a]!, hi[t * 3 + a]!);
        cb[a] = Math.min(cb[a]!, ce[t * 3 + a]!); cb[3 + a] = Math.max(cb[3 + a]!, ce[t * 3 + a]!);
      }
    }
    nodes.bounds.set(b, index * 6);
    const leaf = () => { nodes.left[index] = first; nodes.count[index] = n; };
    if (n <= 2) { leaf(); continue; }
    let best = Infinity, bestAxis = 0, bestSplit = 0;
    for (let axis = 0; axis < 3; axis++) {
      const l0 = cb[axis]!, l1 = cb[3 + axis]!;
      if (l1 - l0 < 1e-9) continue;
      const scale = BINS / (l1 - l0);
      for (let i = 0; i < BINS; i++) { reset(bins, i * 6); binN[i] = 0; }
      for (let k = first; k < first + n; k++) {
        const t = order[k]!;
        const i = Math.min(BINS - 1, Math.floor((ce[t * 3 + axis]! - l0) * scale));
        for (let a = 0; a < 3; a++) {
          bins[i * 6 + a] = Math.min(bins[i * 6 + a]!, lo[t * 3 + a]!);
          bins[i * 6 + 3 + a] = Math.max(bins[i * 6 + 3 + a]!, hi[t * 3 + a]!);
        }
        binN[i]!++;
      }
      reset(acc);
      let cnt = 0;
      for (let i = 0; i < BINS - 1; i++) {
        for (let a = 0; a < 3; a++) { acc[a] = Math.min(acc[a]!, bins[i * 6 + a]!); acc[3 + a] = Math.max(acc[3 + a]!, bins[i * 6 + 3 + a]!); }
        cnt += binN[i]!; leftA[i] = area(acc); leftN[i] = cnt;
      }
      reset(acc);
      cnt = 0;
      for (let i = BINS - 1; i >= 1; i--) {
        for (let a = 0; a < 3; a++) { acc[a] = Math.min(acc[a]!, bins[i * 6 + a]!); acc[3 + a] = Math.max(acc[3 + a]!, bins[i * 6 + 3 + a]!); }
        cnt += binN[i]!;
        const cost = leftA[i - 1]! * leftN[i - 1]! + area(acc) * cnt;
        if (leftN[i - 1]! > 0 && cnt > 0 && cost < best) { best = cost; bestAxis = axis; bestSplit = i; }
      }
    }
    if (best >= area(b) * n && n <= 8) { leaf(); continue; }
    let mid = first + (n >> 1);
    if (best < Infinity) {
      const l0 = cb[bestAxis]!, scale = BINS / (cb[3 + bestAxis]! - l0);
      let i = first;
      for (let j = first; j < first + n; j++) {
        const t = order[j]!;
        if (Math.min(BINS - 1, Math.floor((ce[t * 3 + bestAxis]! - l0) * scale)) < bestSplit) { order[j] = order[i]!; order[i] = t; i++; }
      }
      mid = i;
    }
    if (mid === first || mid === first + n) mid = first + (n >> 1);
    const left = nodes.push(); nodes.push();
    nodes.left[index] = left; nodes.count[index] = 0;
    stack.push(left, first, mid - first, left + 1, mid, first + n - mid);
  }
  return { nodes: widen(nodes), order };
}

/** Wide nodes: [lmin xyz, left, lmax xyz, lcount, rmin xyz, right, rmax xyz, rcount]. */
function widen(nodes: Nodes): ArrayBuffer {
  const rows: { w: number; children: ({ node: number; index: number; count: number } | null)[] }[] = [];
  if (nodes.count[0]! > 0) {
    rows.push({ w: 0, children: [{ node: 0, index: nodes.left[0]!, count: nodes.count[0]! }, null] });
  } else {
    let next = 1;
    const queue: [number, number][] = [[0, 0]];
    for (let q = 0; q < queue.length; q++) {
      const [node, w] = queue[q]!;
      const kids = [nodes.left[node]!, nodes.left[node]! + 1].map(c => {
        if (nodes.count[c]! > 0) return { node: c, index: nodes.left[c]!, count: nodes.count[c]! };
        const wi = next++;
        queue.push([c, wi]);
        return { node: c, index: wi, count: 0 };
      });
      rows.push({ w, children: kids });
    }
  }
  const total = Math.max(1, rows.reduce((m, r) => Math.max(m, r.w + 1), 0));
  const buffer = new ArrayBuffer(total * 64);
  const f = new Float32Array(buffer), u = new Uint32Array(buffer);
  const clamp = (v: number) => Math.max(-3.4e38, Math.min(3.4e38, v));
  for (const { w, children } of rows) {
    children.forEach((c, side) => {
      const o = w * 16 + side * 8;
      if (!c) {
        f.set([3.4e38, 3.4e38, 3.4e38], o); u[o + 3] = 0;
        f.set([-3.4e38, -3.4e38, -3.4e38], o + 4); u[o + 7] = EMPTY_LEAF;
        return;
      }
      const bo = c.node * 6;
      for (let a = 0; a < 3; a++) { f[o + a] = clamp(nodes.bounds[bo + a]!); f[o + 4 + a] = clamp(nodes.bounds[bo + 3 + a]!); }
      u[o + 3] = c.index;
      u[o + 7] = c.count;
    });
  }
  return buffer;
}
