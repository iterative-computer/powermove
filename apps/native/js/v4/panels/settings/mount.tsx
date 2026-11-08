/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
import { SettingsPanel, SettingsModel, type Publication } from "./index";
import { generalRows } from "./catalogue";
import type { BuiltinEnv, Mounted } from "../../src/builtin";
import type { Json } from "../../src/protocol";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { platform, host, command, fields, report, root } = env;
  const settingsPublication = (publication: Publication) => {
    const general = generalRows(publication.multiWindow);
    const leases = fields(
      general.filter((row) => row.profileKey).map((row) => row.id),
    );
    return {
      ...publication,
      rows: [
        ...(publication.rows ?? []),
        ...general
          .filter((row) => row.profileKey)
          .map((row) => ({ ...row, binding: leases[row.id] })),
      ],
    };
  };
  const model = new SettingsModel(
    host,
    settingsPublication(
      (await host.query("project", ["settings"]))
        .value as unknown as Publication,
    ),
    {
      present: (section) => {
        void platform("settings.present", section).catch(report);
      },
      closeAndRestoreFocus: () => {
        void platform("settings.close").catch(report);
      },
      title: (section) => {
        void platform("settings.title", section).catch(report);
      },
      scrollTo: (section, options) => {
        void platform("settings.scroll", { section, options }).catch(report);
      },
      focusSearch: () => {
        void platform("focus", "settings.search").catch(report);
      },
      announce: (message) => {
        void platform("announce", message).catch(report);
      },
    },
    {
      perform: async (action, revision) =>
        command<Publication>("panel.settings", {
          action,
          revision,
        }),
    },
    {
      scope: "kernel-profile",
      getSetting: async (key) => ({
        ...host.identity,
        version: 1,
        requestId: 0,
        revision: 0,
        accepted: true,
        ...(await command<{ value?: Json; stateRevision: number }>(
          "profile.get",
          { key },
        )),
      }),
      setSetting: async (key, value) => ({
        ...host.identity,
        version: 1,
        requestId: 0,
        revision: 0,
        accepted: true,
        ...(await command<{ stateRevision: number }>("profile.set", {
          key,
          value,
        })),
      }),
    },
  );
  await model.hydrate();
  const r = root();
  await r.render(
    <SettingsPanel
      model={model}
      bindings={fields(["search", "extensionSearch", "extensionType"])}
    />,
  );
  const mounted: Mounted = {
    root: r,
    async dispose() {
      model.dispose();
      await r.dispose();
    },
  };
  const stop = await host.subscribe(
    "settings",
    "project",
    (p) => model.publish(settingsPublication(p as unknown as Publication)),
    ["settings"],
  );
  const dispose = mounted.dispose.bind(mounted);
  mounted.dispose = async () => {
    await stop();
    await dispose();
  };
  mounted.show = async () => {
    model.show();
    await r.flush();
  };
  return mounted;
}
