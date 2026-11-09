import { prepareBlasPayload } from './blas';
self.onmessage = (e:MessageEvent) => {
 const b=prepareBlasPayload(e.data);
 (self as any).postMessage(b,[b.positions.buffer,b.tris.buffer,b.nodes]);
};
