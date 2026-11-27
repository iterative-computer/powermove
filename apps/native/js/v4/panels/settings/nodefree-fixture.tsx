import { createRoot } from "../../src/index";
import { HostClient } from "../../src/host-api";
import { SettingsPanel, SettingsModel } from "./index";
const noop = () => {};
const model = new SettingsModel(
  new HostClient(
    { realm: "probe", generation: 1, project: "probe" },
    async (r) => ({ ...r, revision: 0, stateRevision: 0, accepted: true }),
  ),
  {
    revision: 1,
    multiWindow: false,
    extensions: [],
    extensionsState: "unavailable",
    speech: [],
  },
  {
    present: noop,
    closeAndRestoreFocus: noop,
    title: noop,
    scrollTo: noop,
    focusSearch: noop,
    announce: noop,
  },
);
model.show();
const root = createRoot(async (commit) => ({
  accepted: true,
  revision: commit.baseRevision + 1,
}));
(globalThis as Record<string, unknown>).settingsProbe = root
  .render(
    <SettingsPanel
      model={model}
      bindings={{
        search: "probe.search",
        extensionSearch: "probe.extensions",
        extensionType: "probe.type",
      }}
    />,
  )
  .then(() => {
    const result = {
      revision: root.inspect().revision,
      node: typeof process,
      window: typeof window,
      document: typeof document,
      buffer: typeof Buffer,
    };
    root.dispose();
    model.dispose();
    return result;
  });
