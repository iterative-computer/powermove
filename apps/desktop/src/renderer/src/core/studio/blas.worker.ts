import { prepareBlasPayload } from './blas';
self.onmessage = (e:MessageEvent) => {
 const b=prepareBlasPayload(e.data);
 (self as any).postMessage(b,'positions' in b?[b.positions.buffer,b.tris.buffer,b.nodes]:[b.nodes,b.order.buffer]);
};
