/** V1–3 effect definitions use the same wrapper and prelude in v4. */
import { validateEffect } from "../../compat/src/frozen/host/glsl";
import { definitionSnapshots } from "../../compat/src/definitions";
import type { EffectDefinition } from "./effect-definition";
export type { EffectDefinition };

export function effectsAPI(publish: (effects: unknown[]) => void) {
  const entries = new Map<string, EffectDefinition[]>();
  const registrations = new WeakMap<EffectDefinition, number>();
  let serial = 0;
  const live = () =>
    [...entries.values()].map((stack) => stack[stack.length - 1]);
  const emit = () => {
    // Keep every registration, including shadowed ones, in original order.
    // The kernel owns the global provider stack across extension realms.
    const definitions = [...entries.values()]
      .flat()
      .sort((a, b) => registrations.get(a)! - registrations.get(b)!);
    const snapshots = definitionSnapshots({
      effects: {
        entries: () =>
          definitions.map((item) => ({ id: item.id, ownerId: "v4", item })),
      },
      transitions: { entries: () => [] },
      layerTypes: { entries: () => [] },
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
    register(definition: EffectDefinition): () => void {
      validateEffect(definition);
      const entry: EffectDefinition = JSON.parse(JSON.stringify(definition));
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
