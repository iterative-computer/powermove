import { createRoot, type Transport } from "../../src";
import type { HostClient } from "../../src/host-api";
import type {
  NativeTimelineModel,
  TimelineFrame,
  WindowRequest,
} from "./model";
import { TimelinePanel } from "./panel";
import { TimelineSession } from "./session";

export const timelinePanel = Object.freeze({
  id: "timeline",
  title: "Timeline",
});

/** Worker entry for the built-in v4 panel. The application registers this entry
 * in its existing panel slot; it supplies native bindings and accepted-op queue.
 * No guessed PM registration method, document store or compatibility replacement.
 */
export async function mountTimeline(options: {
  transport: Transport;
  host: HostClient;
  model: NativeTimelineModel;
  window: WindowRequest;
  instance: string;
  onError: (error: unknown) => void;
  retireSurface: (generation: number) => Promise<void>;
}) {
  const root = createRoot(options.transport, options.onError, {
    generation: options.host.identity.generation,
    retire: options.retireSurface,
  });
  const session = new TimelineSession(
    options.model,
    options.host,
    options.window,
    options.instance,
    options.onError,
  );
  try {
    await session.start();
    await session.settled();
    if (!session.frame)
      throw new Error("Native timeline projection unavailable");
    const initial = session.frame;
    await root.render(<TimelinePanel frame={initial} send={session.send} />);
    let queue = Promise.resolve();
    let newest: TimelineFrame = initial;
    let retiring = false;
    let failure: unknown;
    const unsubscribe = session.subscribe((frame) => {
      if (frame === newest || retiring) return;
      newest = frame;
      queue = queue
        .then(async () => {
          if (!retiring && newest === frame)
            await root.render(
              <TimelinePanel frame={frame} send={session.send} />,
            );
        })
        .catch((error) => {
          failure = error;
          options.onError(error);
        });
    });
    let retirement: Promise<void> | undefined;
    return {
      ...timelinePanel,
      root,
      session,
      async settled() {
        await session.settled();
        await queue;
        if (failure) throw failure;
      },
      dispose(): Promise<void> {
        if (retirement) return retirement;
        retiring = true;
        unsubscribe();
        retirement = (async () => {
          const errors: unknown[] = [];
          try {
            await session.dispose();
          } catch (error) {
            errors.push(error);
          }
          await queue;
          try {
            await root.dispose();
          } catch (error) {
            errors.push(error);
          }
          if (errors.length)
            throw new AggregateError(
              errors,
              "Timeline panel retirement failed",
            );
        })();
        return retirement;
      },
    };
  } catch (error) {
    const errors: unknown[] = [error];
    try {
      await session.dispose();
    } catch (cleanupError) {
      errors.push(cleanupError);
    }
    try {
      await root.dispose();
    } catch (cleanupError) {
      errors.push(cleanupError);
    }
    if (errors.length > 1)
      throw new AggregateError(errors, "Timeline mount and cleanup failed");
    throw error;
  }
}
