/** Built-in v4 worker entry. The app supplies the accepted native transport and
 * registered model projection; no DOM, Node, WebKit or UI-thread JS calls. */
import {
  createRoot,
  type HostClient,
  type RootOptions,
  type Transport,
} from "../../src";
import { ExportModel, snapshot } from "./model";
import { ExportSheet } from "./panel";
export { ExportModel, snapshot, progress } from "./model";
export { ExportSheet } from "./panel";
export { ExportButton } from "./trigger";
export type { ExportSnapshot, Options, Run } from "./model";
export async function mountExportPanel(config: {
  host: HostClient;
  transport: Transport;
  anchor: number;
  subscription: string;
  rootOptions?: RootOptions;
}) {
  // Capabilities and generation-owned bindings must already be known. Query and
  // subscribe use the existing v4 HostDataService, not a separate JS document.
  const ack = await config.host.query("project", ["export"]);
  const model = new ExportModel(config.host, snapshot(ack.value));
  const root = createRoot(config.transport, undefined, config.rootOptions);
  try {
    await model.connect(config.subscription);
    await root.render(<ExportSheet model={model} anchor={config.anchor} />);
  } catch (error) {
    await model.dispose();
    await root.dispose();
    throw error;
  }
  return {
    model,
    root,
    async show() {
      await model.show();
      await root.render(<ExportSheet model={model} anchor={config.anchor} />);
    },
    /** Call after shell registry/field arbitration; both shipped bindings pass. */
    async key(key: string, meta = false, shift = false) {
      const handled = await model.key(key, meta, shift);
      await root.render(<ExportSheet model={model} anchor={config.anchor} />);
      return handled;
    },
    async dispose() {
      await model.dispose();
      await root.dispose();
    },
  };
}
