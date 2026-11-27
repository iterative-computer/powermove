/** V1–3 effect, transition, and custom layer definitions use the same wrapper and prelude in v4. */
import { validateEffect, validateTransition } from "../../compat/src/frozen/host/glsl";
import { validateExtensionLayerDefinition } from "../../compat/src/frozen/host/extension-layers";
import { definitionSnapshots } from "../../compat/src/definitions";
import type { EffectDefinition } from "./effect-definition";
import type { LayerDefinition, TransitionDefinition } from "./builtin";
export type { EffectDefinition };

const EMPTY = { entries: () => [] };

function registryAPI<T extends { id: string }>(
  validate: (def: T) => void,
  kind: "effects" | "transitions" | "layerTypes",
  publish: (items: unknown[]) => void,
) {
  const entries = new Map<string, T[]>();
  const registrations = new WeakMap<T, number>();
  let serial = 0;
  const live = () =>
    [...entries.values()].map((stack) => stack[stack.length - 1]);
  const emit = () => {
    const definitions = [...entries.values()]
      .flat()
      .sort((a, b) => registrations.get(a)! - registrations.get(b)!);
    const wrapped = {
      entries: () =>
        definitions.map((item) => ({ id: item.id, ownerId: "v4", item })),
    };
    const snapshots = definitionSnapshots({
      effects: kind === "effects" ? wrapped : EMPTY,
      transitions: kind === "transitions" ? wrapped : EMPTY,
      layerTypes: kind === "layerTypes" ? wrapped : EMPTY,
    });
    publish(
      definitions.map((definition, i) => ({
        definition,
        snapshot: snapshots[i],
        registration: registrations.get(definition),
      })),
    );
  };
  return {
    register(definition: T): () => void {
      validate(definition);
      const entry: T = JSON.parse(JSON.stringify(definition));
      registrations.set(entry, ++serial);
      const stack = entries.get(entry.id) ?? [];
      stack.push(entry);
      entries.set(entry.id, stack);
      try {
        emit();
      } catch (error) {
        stack.pop();
        if (!stack.length) entries.delete(entry.id);
        throw error;
      }
      return () => {
        const at = stack.indexOf(entry);
        if (at < 0) return;
        stack.splice(at, 1);
        if (!stack.length) entries.delete(entry.id);
        emit();
      };
    },
    list: () => live().map((d) => JSON.parse(JSON.stringify(d))),
    get: (id: string) => {
      const d = entries.get(id)?.at(-1);
      return d && JSON.parse(JSON.stringify(d));
    },
  };
}

export function effectsAPI(publish: (effects: unknown[]) => void) {
  return registryAPI<EffectDefinition>(validateEffect, "effects", publish);
}

export function transitionsAPI(publish: (transitions: unknown[]) => void) {
  return registryAPI<TransitionDefinition>(
    validateTransition,
    "transitions",
    publish,
  );
}

export function layersAPI(publish: (layers: unknown[]) => void) {
  return registryAPI<LayerDefinition>(
    validateExtensionLayerDefinition,
    "layerTypes",
    publish,
  );
}
