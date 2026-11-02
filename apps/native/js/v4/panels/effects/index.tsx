import { createRoot, type Transport } from "../../src";
import { EffectsHost, type EffectsHostAdapter } from "./host";
import { HostClient } from "../../src/host-api";
import { EffectsModel, workerSearch } from "./model";
import { EffectsPanel, type EffectsPanelProps } from "./panel";
export * from "./model";
export * from "./host";
export * from "./panel";

export class EffectsMountError extends AggregateError {
  constructor(
    errors: unknown[],
    readonly retryCleanup: () => Promise<void>,
  ) {
    super(errors, "Effects mount and cleanup failed");
  }
}

/** Application calls this in its v4 worker, after registering local field bindings.
 * Nothing registers a competing panel or replaces the v1–3 provider implicitly. */
export async function createEffectsExtension(options: {
  transport: Transport;
  host: HostClient;
  adapter: EffectsHostAdapter;
  bindings: EffectsPanelProps["bindings"];
  subscriptionPrefix: string;
  viewport: { start: number; size: number };
  onError: (error: unknown) => void;
}) {
  const host = new EffectsHost(options.host, options.adapter);
  const model = new EffectsModel({
    apply: (item, target) => host.apply(item, target),
    reveal: (item, target) => host.reveal(item, target),
    persist: (key, value) => host.persist(key, value),
    search: workerSearch,
    report: options.onError,
  });
  const root = createRoot(options.transport, options.onError);
  const cleanup = async () => {
    model.dispose();
    const results = await Promise.allSettled([host.dispose(), root.dispose()]);
    const errors = results.flatMap((result) =>
      result.status === "rejected" ? [result.reason] : [],
    );
    if (errors.length)
      throw new AggregateError(errors, "Effects extension retirement failed");
  };
  try {
    model.setViewport(options.viewport.start, options.viewport.size);
    await host.connect(model, options.subscriptionPrefix);
    const ack = await root.render(
      <EffectsPanel model={model} bindings={options.bindings} />,
    );
    if (ack && !ack.accepted) throw new Error("Effects surface rejected");
  } catch (error) {
    try {
      await cleanup();
    } catch (cleanupError) {
      throw new EffectsMountError([error, cleanupError], cleanup);
    }
    throw error;
  }
  return {
    root,
    model,
    inspect: () => model.inspect(),
    dispose: cleanup,
  };
}
