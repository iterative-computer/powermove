import { expect, test } from 'bun:test';
import { validateViewerHandles } from '../src/viewer-handles';
const def = (viewerHandles: unknown) => ({params:[{k:'x'},{k:'y'},{k:'angle'},{k:'radius'},{k:'tint',type:'color'}], viewerHandles});
test('viewer declarations cover each control kind', () => {
  const p={x:'x',y:'y'};
  expect(() => validateViewerHandles(def([{kind:'point',point:p},{kind:'line',start:p,end:p},{kind:'radius',center:p,radius:'radius'},{kind:'angle',center:p,angle:'angle'},{kind:'rect',start:p,end:p}]))).not.toThrow();
});
test('viewer declarations reject unknown keys, fields and kinds', () => {
  for (const h of [{kind:'point',point:{x:'missing',y:'y'}},{kind:'point',point:{x:'tint',y:'y'}},{kind:'point',point:{x:'x',y:'y'},unknown:1},{kind:'ellipse'}]) {
    expect(() => validateViewerHandles(def([h]))).toThrow();
  }
});
