import { createRoot, type Transport, type HostClient } from "../../src";
import { EffectsController } from "./controller";
import { EffectControls, type PanelProps } from "./panel";
import { PANEL_ID, PANEL_TITLE, type Platform } from "./contract";

export const effectControlsExtension = Object.freeze({
  apiVersion: 4,
  id: "builtin.effect-controls",
  panel: PANEL_ID,
  title: PANEL_TITLE,
});
export async function mountEffectControls(
  options: Omit<PanelProps, "controller"> & {
    transport: Transport;
    host: HostClient;
    platform: Platform;
    /** Unique for this mount in the host generation; subscription ids never recycle. */
    subscriptionId: string;
  },
) {
  const controller = new EffectsController(
    options.host,
    options.platform,
    options.subscriptionId,
  );
  const root = createRoot(options.transport, (error) =>
    options.platform.report(error),
  );
  try {
    await controller.start();
    await root.render(<EffectControls {...options} controller={controller} />);
  } catch (error) {
    await root.dispose();
    await controller.dispose();
    throw error;
  }
  return {
    root,
    controller,
    async dispose() {
      await root.dispose();
      await controller.dispose();
    },
  };
}
