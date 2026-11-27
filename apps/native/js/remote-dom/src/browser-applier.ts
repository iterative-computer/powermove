/** Host-built page entry. Extension code executes only in the V8 owner. */
import { PageApplier } from './applier';
Object.assign(globalThis, { PMRemoteDOM: { PageApplier } });
