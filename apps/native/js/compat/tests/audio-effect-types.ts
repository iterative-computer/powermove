import type { AudioEffectsAPI } from "../api-pack/audio-effects";

// Compile-time public API contract. These calls are deliberately never run.
export function checkAudioEffectTypes(api: AudioEffectsAPI): void {
  api.register({
    id: "test.typed",
    label: "Typed",
    group: "Audio",
    params: [
      {
        type: "number",
        k: "level",
        label: "Level",
        min: -96,
        max: 24,
        unit: "dB",
        def: 0,
      },
    ],
    graph: [
      { type: "gain", gainDb: { param: "level" } },
      // @ts-expect-error Bindings must name a declared number parameter.
      { type: "gain", gainDb: { param: "missing" } },
      // @ts-expect-error Constants retain their processor's type.
      { type: "gain", gainDb: { value: "loud" } },
    ],
  });
  api.register({
    id: "test.toggle",
    label: "Toggle",
    group: "Audio",
    params: [{ type: "toggle", k: "on", label: "On", def: true }],
    graph: [
      { type: "gate", enabled: { param: "on" } },
      // @ts-expect-error A boolean parameter cannot bind a numeric setting.
      { type: "gain", gainDb: { param: "on" } },
    ],
  });
}
