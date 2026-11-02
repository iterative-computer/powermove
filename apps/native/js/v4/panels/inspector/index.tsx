import { createRoot } from "../../src";
import type { Transport } from "../../src/protocol";
import { InspectorPanel } from "./view";
import { InspectorSession } from "./session";
export * from "./contracts";
export { InspectorPanel, InspectorSession };

/** App activation must register this v4 entry for panel id `inspector`.
 * That unowned shell registration is deliberately not hidden in an example.
 */
export async function mountInspector(
  session: InspectorSession,
  transport: Transport,
  options: { retire: (generation: number) => Promise<void> },
) {
  const root = createRoot(transport, session.onError, {
    generation: session.host.identity.generation,
    retire: async (generation) => {
      try {
        await session.dispose();
      } finally {
        await options.retire(generation);
      }
    },
  });
  try {
    await session.start();
    await root.render(<InspectorPanel session={session} />);
  } catch (error) {
    try {
      await root.dispose();
    } catch (cleanupError) {
      throw new AggregateError(
        [error, cleanupError],
        "Inspector startup and retirement failed",
      );
    } finally {
      await session.dispose();
    }
    throw error;
  }
  return {
    root,
    async dispose() {
      try {
        await root.dispose();
      } finally {
        await session.dispose();
      }
    },
  };
}
