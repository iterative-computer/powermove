// The Math sweep's input generator, shared by reference.mjs (V8) and
// wasm.ts (the wasm32 build). tests/support/mod.rs mirrors it exactly.

const view = new DataView(new ArrayBuffer(8));

export const MATH_FUNCTIONS = ['sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'atan2', 'exp', 'log', 'pow', 'sqrt', 'random', 'wiggle'];

export function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (t ^ (t >>> 14)) >>> 0;
  };
}

const fromBits = (hi, lo) => {
  view.setUint32(0, hi);
  view.setUint32(4, lo);
  return view.getFloat64(0);
};

const RANGE_A = { sin: 1e3, cos: 1e3, tan: 1e3, asin: 1, acos: 1, atan: 1e3, atan2: 1e3, exp: 745, log: 1e3, pow: 20, sqrt: 1e3, random: 1e3, wiggle: 1e3 };
const RANGE_B = { atan2: 1e3, pow: 40 };

/** The inputs of sample i of function fn; six draws per sample. */
export function sample(fn, i, next) {
  const d = [next(), next(), next(), next(), next(), next()];
  const unit = (k) => d[k] / 4294967296;
  const mode = i % 5;
  let a;
  if (mode === 0) a = fromBits(d[0], d[1]);
  else if (mode === 1) a = (unit(0) * 2 - 1) * RANGE_A[fn];
  else if (mode === 2) a = (Math.floor(unit(0) * 2001) - 1000 + Math.floor(unit(1) * 100) / 100) * 127.1;
  else if (mode === 3) a = unit(0) * 60 * (unit(1) * 8) * 6.2831 + unit(2) * 17.3 + Math.floor(unit(3) * 3) * 2.4;
  else a = (unit(0) * 2 - 1) * fromBits(((Math.floor(unit(1) * 80) - 60 + 1023) << 20) >>> 0, 0);
  let b;
  if (mode === 0) b = fromBits(d[2], d[3]);
  else if (mode === 2 && fn === 'pow') b = Math.floor((unit(4) * 2 - 1) * 40);
  else b = (unit(4) * 2 - 1) * (RANGE_B[fn] ?? 1);
  if (fn === 'log') a = Math.abs(a);
  if ((fn === 'asin' || fn === 'acos') && mode !== 0) a = unit(0) * 2 - 1;
  if (fn === 'pow' && mode === 1) a = unit(0) * 20;
  if (fn === 'wiggle') { a = unit(0) * 60; b = unit(1) * 8; }
  return [a, b];
}
