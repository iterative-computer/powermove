// Run with Bun from any directory. The authored extension is the source of
// truth; native startup consumes its data without opening an idle V8 realm.
import { activateAudioEffects } from "../../../../apps/native/js/compat/src/audio-effects-extension";
import type {
  AudioEffectDefinition,
  AudioEffectsAPI,
} from "../../../../apps/native/js/compat/api-pack/audio-effects";

const definitions: AudioEffectDefinition[] = [];
const audioEffects: AudioEffectsAPI = {
  register(definition) {
    definitions.push(definition);
    return { dispose() {} };
  },
  list: () => definitions,
  get: (id) => definitions.find((definition) => definition.id === id),
};
activateAudioEffects({ audioEffects });
await Bun.write(
  new URL("../src/audio-builtins.json", import.meta.url),
  JSON.stringify(definitions, null, 2) + "\n",
);
