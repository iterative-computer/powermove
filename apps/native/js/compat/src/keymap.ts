/** DOM-free adaptation of v1.2.0 registries.ts binding and bubble resolver. */
import { Registry } from "./registry";
import { chordMatches, chordModifierCount, normalizeChord } from "./keychord";
import type { Data, Disposable } from "./types";
export interface Binding extends Data {
  id: string;
  key: string;
  chord: string;
  command: string;
  ownerId: string;
  priority: number;
  inFields: boolean;
  repeat: boolean;
  looseModifiers?: boolean;
  args?: any[];
}
export interface KeyInput {
  chord: string;
  field: boolean;
  repeat: boolean;
  defaultPrevented: boolean;
  /** DOM bridge has already run panel target/ancestor listeners. */
  preventDefault(): void;
  skipOwner?: string;
}
export class CompatKeymap {
  readonly commands = new Registry<
    Data & { id: string; run: (...args: any[]) => any }
  >();
  readonly keybindings = new Registry<Binding>();
  constructor(private reportError: (error: unknown) => void) {}
  bind(ownerId: string, definition: Data): Disposable {
    const chord = normalizeChord(definition.key ?? "");
    if (!chord)
      throw new Error(`keybindings: invalid key "${String(definition.key)}"`);
    if (typeof definition.command !== "string" || !definition.command)
      throw new Error('keybindings: "command" is required');
    return this.keybindings.register(ownerId, {
      ...definition,
      key: definition.key,
      command: definition.command,
      id: `${chord}::${definition.command}`,
      chord,
      ownerId,
      priority: definition.priority ?? 0,
      inFields: definition.inFields === true,
      repeat: definition.repeat === true,
    });
  }
  unbind(ownerId: string, key: string, all = false): void {
    const chord = normalizeChord(key);
    for (const entry of this.keybindings.entries()) {
      if (entry.item.chord === chord && (all || entry.ownerId === ownerId))
        this.keybindings.disposeEntry(entry);
    }
  }
  bindingsFor(chord: string): Binding[] {
    const wanted = normalizeChord(chord);
    return this.keybindings
      .entries()
      .filter((e) =>
        chordMatches(e.item.chord, wanted, e.item.looseModifiers === true),
      )
      .sort(
        (a, b) =>
          a.item.priority - b.item.priority ||
          Number(b.item.chord === wanted) - Number(a.item.chord === wanted) ||
          (a.item.looseModifiers || b.item.looseModifiers
            ? chordModifierCount(b.item.chord) -
              chordModifierCount(a.item.chord)
            : 0) ||
          a.seq - b.seq,
      )
      .map((e) => e.item);
  }
  run(command: string, args: any[] = []): any {
    const entry = this.commands.get(command);
    return entry ? entry.run(...args) : false;
  }
  bubble(event: KeyInput): void {
    for (const binding of this.bindingsFor(event.chord)) {
      if (
        binding.ownerId === event.skipOwner ||
        (event.field && !binding.inFields) ||
        (event.repeat && !binding.repeat)
      )
        continue;
      if (!binding.inFields && !event.repeat) event.preventDefault();
      try {
        if (this.run(binding.command, binding.args ?? []) === false) continue;
      } catch (error) {
        this.reportError(error);
        return;
      }
      if (binding.inFields || event.repeat) event.preventDefault();
      return;
    }
  }
  disposeOwner(ownerId: string): void {
    this.commands.disposeOwner(ownerId);
    this.keybindings.disposeOwner(ownerId);
  }
}
