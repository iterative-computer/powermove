import * as compat from "../../../js/compat/src";
import { RemoteDOM } from "../../../js/remote-dom/src/shim";
Object.assign(globalThis, { PowermoveCompat: { ...compat, RemoteDOM } });
