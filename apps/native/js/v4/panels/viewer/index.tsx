import {
  createRoot,
  type HostClient,
  type RootOptions,
  type Transport,
} from "../../src";
import { ViewerPanel } from "./panel";
import { ViewerService } from "./service";
export { ViewerPanel } from "./panel";
export { ViewerService } from "./service";
export * from "./contract";

/** Application built-in registry must register panel id `viewer`, nonclosable.
 * Application and compatibility-provider cutover are deliberately outside this lane.
 */
export async function mountViewer(options: {
  host: HostClient;
  transport: Transport;
  subscription: string;
  rootOptions: RootOptions;
  open: () => void;
  command?: (name: string) => void;
  onError: (error: unknown) => void;
}) {
  const root = createRoot(
    options.transport,
    options.onError,
    options.rootOptions,
  );
  const service = new ViewerService(options.host);
  let rendered = Promise.resolve();
  const actions = {
    open: options.open,
    command: (name: string) => options.command?.(name),
    drive(element: string, argument?: import("../../src").Json) {
      void service.drive(element, argument).catch(options.onError);
    },
  };
  try {
    await service.start(options.subscription, (state) => {
      rendered = rendered.then(async () => {
        await root.render(<ViewerPanel state={state} actions={actions} />);
      });
      // Keep async surface failures observable without an unhandled rejection.
      void rendered.catch(options.onError);
    });
    await rendered;
  } catch (error) {
    await service.stop();
    await root.dispose();
    throw error;
  }
  return {
    root,
    service,
    async dispose() {
      try {
        await service.stop();
      } finally {
        await rendered.catch(() => {});
        await root.dispose();
      }
    },
  };
}
