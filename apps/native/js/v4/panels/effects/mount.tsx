import { PresetSheet } from "./preset-sheet";
import { createEffectsExtension } from "./index";
import { EffectsPanel } from "./panel";
import type { Registry, Target } from "./model";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, host, transport, rootOptions, fields, report } = env;
  let registryPublish: ((registry: Registry) => void) | undefined;
  const mounted = await createEffectsExtension({
    transport,
    host,
    rootOptions,
    bindings: fields(["search", "scope"]),
    subscriptionPrefix: "effects",
    viewport: { start: 0, size: 32 },
    onError: report,
    adapter: {
      registry: {
        subscribe(publish) {
          publish(native("effects.registry"));
          registryPublish = publish;
          return () => {
            registryPublish = undefined;
          };
        },
      },
      target: () => native("effects.target") as Target | null,
      add: (item, target) => [
        {
          command: item.kind === "preset" ? "preset.apply" : "effect.add",
          args: {
            definition: item.id,
            preset: item.id.slice(7),
            target: target.id,
          },
        },
      ],
      reveal: (_item, target) => ({
        command: "effect.reveal",
        args: { target: target.id },
      }),
    },
  });
  const refresh = () =>
    registryPublish?.(native("effects.registry") as Registry);
  let sheetOpen = false;
  const showSheet = async (hint: any) => {
    const choices = native("presets.choices", hint);
    if (!hint.layer)
      hint.layer = (native("effects.target") as Target | null)?.id;
    sheetOpen = true;
    await mounted.root.render(
      <>
        <EffectsPanel
          model={mounted.model}
          bindings={fields(["search", "scope"])}
        />
        <PresetSheet
          choices={choices}
          bindings={fields([
            "name",
            "animated",
            ...choices.effects.map((f: any) => `fx.${f.id}`),
            ...choices.effects.flatMap((f: any) =>
              f.parameters.map((p: any) => `parameter.${f.id}.${p.key}`),
            ),
            ...choices.properties.map((p: any) => `property.${p.path}`),
          ])}
          hint={hint}
          cancel={() => {
            sheetOpen = false;
            void mounted.root.render(
              <EffectsPanel
                model={mounted.model}
                bindings={fields(["search", "scope"])}
              />,
            );
          }}
          save={(value) => {
            try {
              native("presets.save", { ...value, layer: hint.layer });
              sheetOpen = false;
              refresh();
              void mounted.root.render(
                <EffectsPanel
                  model={mounted.model}
                  bindings={fields(["search", "scope"])}
                />,
              );
            } catch (e) {
              report(e);
            }
          }}
        />
      </>,
    );
  };
  mounted.model.presetRenameBinding = fields(["rename"]).rename;
  mounted.model.presetMenu = async (id) => {
    const choice = await env.platform("contextMenu", [
      { id: "rename", label: "Rename…" },
      { id: "delete", label: "Delete" },
    ]);
    if (choice === "rename") {
      mounted.model.renamingPreset = id;
      mounted.model.notifyPresetChange();
    }
    if (choice === "delete") await mounted.model.presetAction?.(id, "delete");
  };
  mounted.model.presetAction = async (id, action, name) => {
    if (action === "delete")
      mounted.model.deletedPresetName =
        (native("presets.list") as any[]).find((p) => p.id === id.slice(7))
          ?.name ?? "Preset";
    native("presets.action", { id: id.slice(7), action, name });
    if (action === "undo") mounted.model.deletedPresetName = null;
    mounted.model.renamingPreset = null;
    refresh();
  };
  (mounted as Mounted).publish = () => {
    const hint = native("presets.takeSheet");
    refresh();
    if (hint && !sheetOpen) return showSheet(hint);
    return Promise.resolve();
  };
  const pending = native("presets.takeSheet");
  if (pending) await showSheet(pending);
  return mounted;
}
