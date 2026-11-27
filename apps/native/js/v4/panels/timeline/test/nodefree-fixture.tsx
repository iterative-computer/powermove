import { createRoot } from "../../../src";
import { TimelinePanel } from "../panel";
import { frame } from "./fixtures";

(globalThis as unknown as { timelineProbe: Promise<unknown> }).timelineProbe =
  (async () => {
    const root = createRoot(async (commit) => ({
      accepted: true,
      revision: commit.baseRevision + 1,
    }));
    const actions: unknown[] = [];
    await root.render(
      <TimelinePanel frame={frame()} send={(action) => actions.push(action)} />,
    );
    const play = root
      .inspect()
      .nodes.find((node) => node.props.id === "timeline:play")!;
    await root.drive(play.id, "click");
    const globals = globalThis as Record<string, unknown>;
    const result = {
      actions,
      globals: ["process", "Buffer", "window", "document"].map(
        (key) => typeof globals[key],
      ),
    };
    await root.dispose();
    return result;
  })();
