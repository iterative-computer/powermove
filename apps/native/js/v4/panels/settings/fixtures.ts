/** Synthetic test fixtures only; never real accounts, project or extension data. */
import { HostClient, type HostRequest } from "../../src/host-api";
import type { Json } from "../../src/protocol";
import {
  SettingsModel,
  type Action,
  type Extension,
  type Publication,
  type SettingsWindow,
  type ProfileSettings,
} from "./model";
export const extension = (i = 0): Extension => ({
  id: `fixture-${i}`,
  name: `Fixture ${i}`,
  type: "Effects",
  source: "Installed",
  enabled: true,
  about: "Fixture detail",
  includes: ["Fixture effect"],
  shortcuts: [],
  files: [],
  variables: [],
  canArchive: true,
});
export const publication = (patch: Partial<Publication> = {}): Publication => ({
  revision: 1,
  multiWindow: true,
  extensions: [],
  extensionsState: "ready",
  speech: [],
  ...patch,
});
/** Synthetic profile authority for the test host only. Not a production grant. */
export const profileFor = (host: HostClient): ProfileSettings => ({
  scope: "kernel-profile",
  getSetting: (key) => host.getSetting(key),
  setSetting: (key, value) => host.setSetting(key, value),
});
export function fixture(patch: Partial<Publication> = {}) {
  const requests: HostRequest[] = [];
  const settings = new Map<string, Json>();
  const effects: [string, unknown?][] = [];
  const actions: Action[] = [];
  const window: SettingsWindow = {
    present: (s) => effects.push(["present", s]),
    closeAndRestoreFocus: () => effects.push(["close"]),
    title: (s) => effects.push(["title", s]),
    scrollTo: (s, options) =>
      effects.push(["scroll", { section: s, ...options }]),
    focusSearch: () => effects.push(["focus"]),
    announce: (message) => effects.push(["announce", message]),
  };
  let revision = 0;
  const host = new HostClient(
    { realm: "settings-test", generation: 1, project: "fixture-project" },
    async (request) => {
      requests.push(request);
      const op = request.operation;
      if (op.type === "settings.set") settings.set(op.key, op.value);
      return {
        ...request,
        accepted: true,
        revision: ++revision,
        stateRevision: revision,
        ...(op.type === "settings.get" && settings.has(op.key)
          ? { value: settings.get(op.key) }
          : {}),
      };
    },
  );
  const model: SettingsModel = new SettingsModel(
    host,
    publication(patch),
    window,
    {
      perform: async (action) => {
        actions.push(action);
        return {
          ...model.publication,
          revision: model.publication.revision + 1,
        };
      },
    },
    profileFor(host),
  );
  return { model, host, settings, effects, requests, actions, window };
}
export const bindings = {
  search: "test-lease.search",
  extensionSearch: "test-lease.extension-search",
  extensionType: "test-lease.extension-type",
};
