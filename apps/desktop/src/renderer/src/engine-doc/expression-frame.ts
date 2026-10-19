import type { PmWasm } from './wasm';

export interface ExpressionFrameSlot {
  group: { time?: number; revision: number; values?: Float64Array; kinds?: Uint8Array };
  index: number;
}

/** Resident expression ids, grouped by the exact scope they were interned in.
 * Warm frames cross once. Revision, module, scope and Object.is(time) are exact;
 * nonnumeric results retain the scalar path and its existing throw decoding.
 * Weak owners do not keep deleted layers/channels alive. */
export class ExpressionFrames {
  private groups = new Map<string, { ids: Map<number, WeakRef<object>>; order: number[]; index: Map<number, number>; slots: Map<number, ExpressionFrameSlot>; time?: number; revision: number; values?: Float64Array; kinds?: Uint8Array }>();

  register(comp: string, id: number, owner: object): ExpressionFrameSlot {
    let group = this.groups.get(comp);
    if (!group) { group = { ids: new Map(), order: [], index: new Map(), slots: new Map(), revision: -1 }; this.groups.set(comp, group); }
    if (group.ids.has(id)) return group.slots.get(id)!;
    group.ids.set(id, new WeakRef(owner));
    group.order.push(id);
    group.index.set(id, group.order.length - 1);
    group.time = undefined;
    const slot = { group, index: group.order.length - 1 };
    group.slots.set(id, slot);
    return slot;
  }

  release(id: number): void {
    for (const [comp, group] of this.groups) if (group.ids.delete(id)) {
      if (!group.ids.size) { this.groups.delete(comp); continue; }
      group.order = [...group.ids.keys()];
      group.index = new Map(group.order.map((id, i) => [id, i]));
      group.slots.delete(id);
      group.order.forEach((id, i) => { group.slots.get(id)!.index = i; });
      group.time = undefined;
    }
  }

  read(wasm: PmWasm, comp: string, id: number, time: number, revision: number): { value: number; status: number } | null {
    const group = this.groups.get(comp);
    if (!group || group.order.length < 2 || !group.index.has(id)) return null;
    if (group.time === undefined || !Object.is(group.time, time) || group.revision !== revision) {
      const result = wasm.evBatch(group.order, time);
      // Another scope may use the ABI buffers before this scope is read again.
      group.values = result.values.slice();
      group.kinds = result.kinds.slice();
      group.time = time;
      group.revision = revision;
    }
    const index = group.index.get(id)!;
    return { value: group.values![index]!, status: group.kinds![index]! };
  }
}
