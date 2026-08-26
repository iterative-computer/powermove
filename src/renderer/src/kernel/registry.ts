/*
 * Generic contribution registry with LIFO override semantics.
 *
 * Every contribution kind (panels, commands, effects, …) is a `Registry<T>`.
 * Registering an item whose id already exists pushes onto that id's stack and
 * becomes the active entry; disposing it restores the previous one. Disposing a
 * buried entry silently removes it without disturbing the top.
 *
 * Ownership: every entry carries the extension id that registered it, so
 * `disposeOwner(id)` releases everything an extension contributed in one call —
 * the invariant the loader relies on for deactivate/reload.
 */
import type { Disposable } from './api';

export type RegistryChangeKind = 'add' | 'remove' | 'replace';

export interface RegistryChange {
  id: string;
  kind: RegistryChangeKind;
}

export interface RegistryEntry<T> {
  id: string;
  ownerId: string;
  item: T;
  /** Monotonic registration sequence — stable tie-break for priority sorts. */
  seq: number;
}

let SEQ = 0;

/** Test seam: makes generated sequence numbers deterministic across files. */
export function resetRegistrySeq(): void {
  SEQ = 0;
}

export class Registry<T extends { id: string }> {
  /** id → stack, top of stack (last element) is active. Insertion-ordered. */
  private stacks = new Map<string, Array<RegistryEntry<T>>>();
  private listeners = new Set<(change: RegistryChange) => void>();

  register(ownerId: string, item: T): Disposable {
    const id = item?.id;
    if (typeof id !== 'string' || id.length === 0) throw new Error('registry: item.id is required');
    const entry: RegistryEntry<T> = { id, ownerId, item, seq: SEQ++ };
    let stack = this.stacks.get(id);
    if (!stack) {
      stack = [];
      this.stacks.set(id, stack);
    }
    const wasEmpty = stack.length === 0;
    stack.push(entry);
    this.emit({ id, kind: wasEmpty ? 'add' : 'replace' });

    let disposed = false;
    return {
      dispose: () => {
        if (disposed) return;
        disposed = true;
        this.remove(entry);
      }
    };
  }

  /** Remove a specific entry (used by `unbind`, which targets entries not ids). */
  disposeEntry(entry: RegistryEntry<T>): void {
    this.remove(entry);
  }

  private remove(entry: RegistryEntry<T>): void {
    const stack = this.stacks.get(entry.id);
    if (!stack) return;
    const index = stack.indexOf(entry);
    if (index === -1) return;
    const wasTop = index === stack.length - 1;
    stack.splice(index, 1);
    if (stack.length === 0) {
      this.stacks.delete(entry.id);
      this.emit({ id: entry.id, kind: 'remove' });
    } else if (wasTop) {
      this.emit({ id: entry.id, kind: 'replace' });
    }
    // Removing a buried entry changes nothing observable — no event.
  }

  get(id: string): T | undefined {
    return this.topEntry(id)?.item;
  }

  topEntry(id: string): RegistryEntry<T> | undefined {
    const stack = this.stacks.get(id);
    return stack && stack.length > 0 ? stack[stack.length - 1] : undefined;
  }

  has(id: string): boolean {
    return this.stacks.has(id);
  }

  /** Active items, in first-registration order of their ids. */
  list(): T[] {
    return this.entries().map((entry) => entry.item);
  }

  /** Active entries (item + owner + seq), in first-registration order of their ids. */
  entries(): Array<RegistryEntry<T>> {
    const out: Array<RegistryEntry<T>> = [];
    for (const stack of this.stacks.values()) {
      const top = stack[stack.length - 1];
      if (top) out.push(top);
    }
    return out;
  }

  ids(): string[] {
    return [...this.stacks.keys()];
  }

  /** Every entry an owner registered, including buried ones. */
  ownerEntries(ownerId: string): Array<RegistryEntry<T>> {
    const out: Array<RegistryEntry<T>> = [];
    for (const stack of this.stacks.values()) for (const entry of stack) if (entry.ownerId === ownerId) out.push(entry);
    return out;
  }

  disposeOwner(ownerId: string): void {
    for (const entry of this.ownerEntries(ownerId)) this.remove(entry);
  }

  clear(): void {
    for (const id of [...this.stacks.keys()]) {
      this.stacks.delete(id);
      this.emit({ id, kind: 'remove' });
    }
  }

  onChange(fn: (change: RegistryChange) => void): Disposable {
    this.listeners.add(fn);
    return { dispose: () => void this.listeners.delete(fn) };
  }

  private emit(change: RegistryChange): void {
    for (const fn of [...this.listeners]) {
      try {
        fn(change);
      } catch (error) {
        console.error('[kernel] registry listener failed', error);
      }
    }
  }
}
