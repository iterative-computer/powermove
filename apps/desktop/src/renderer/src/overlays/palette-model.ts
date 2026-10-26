import { whenCheck } from '../kernel/registries';
import type { OverlayPM } from './types';

export type PaletteEntry = {
  id: string;
  label: string;
  cat: string;
  kb?: string | null;
  run(): unknown;
};

/** The legacy matcher is case-insensitive trimmed substring matching. The
 * numeric result makes that contract testable without changing legacy order. */
export function scorePaletteMatch(value: unknown, query: unknown): number | null {
  const needle = String(query ?? '').toLowerCase().trim();
  if (!needle) return 0;
  const index = String(value ?? '').toLowerCase().indexOf(needle);
  return index < 0 ? null : index;
}

/* After Effects names for Powermove commands, so a search in the words an
   After Effects user already knows still finds the command. */
export const AFTER_EFFECTS_TERMS: Readonly<Record<string, readonly string[]>> = {
  toggleContinuousRaster: ['Collapse Transformations'],
  toggle3DLayer: ['3D Layer Switch'],
  precompose: ['Nest Layers', 'Precompose'],
  releasePrecomp: ['Unprecompose'],
  newNull: ['Null Object'],
  newAdjustment: ['Adjustment Layer'],
  newShape: ['Shape Layer'],
  graph: ['Speed Graph', 'Value Graph'],
  easyEase: ['Keyframe Assistant'],
  split: ['Split Layer'],
  groupLayers: ['Nest'],
  compositionSettings: ['Composition Settings'],
  palette: ['Search'],
};

const isThenable = (value: unknown): value is PromiseLike<unknown> =>
  !!value && typeof (value as { then?: unknown }).then === 'function';

/**
 * The palette's rows for one query, in legacy order and capped at 60.
 * Asynchronous answers are placed at once with what is known now: a
 * sandboxed command by its last `when` answer, a provider's rows not at
 * all. When one settles to something different, `onLate` gets the whole
 * list again with it in place. A caller keeps those only while it still
 * shows `query`.
 */
export function paletteEntries(PM: OverlayPM, query: string, onLate?: (entries: PaletteEntry[]) => void): PaletteEntry[] {
  const hasQuery = query.toLowerCase().trim().length > 0;
  const matches = (value: unknown): boolean => scorePaletteMatch(value, query) != null;
  /* Rows in order, a run of them per part; a part still answering holds what is known now. */
  const parts: PaletteEntry[][] = [];
  const answers: Array<{ index: number; answer: PromiseLike<PaletteEntry[]> }> = [];
  let entries: PaletteEntry[] = [];
  parts.push(entries);
  const later = (now: PaletteEntry[], answer: PromiseLike<PaletteEntry[]>): void => {
    answers.push({ index: parts.length, answer });
    parts.push(now);
    entries = [];
    parts.push(entries);
  };
  for (const command of Object.values(PM.commands ?? {}) as Array<Record<string, any>>) {
    const term = hasQuery && !matches(command.label)
      ? (AFTER_EFFECTS_TERMS[command.id] ?? []).find((name) => matches(name))
      : undefined;
    if (hasQuery && !matches(command.label) && !term) continue;
    const entry: PaletteEntry = {
      id: `command:${command.id}`,
      label: term ? `${command.label} — ${term} in After Effects` : String(command.label),
      cat: String(command.cat ?? 'General'),
      kb: command.kb,
      run: () => PM.cmd(command.id)
    };
    /* `when` is the kernel's "runnable by id, but not offered here" flag.
       Its registered form answers at once with the last reply; the palette
       asks afresh. */
    const check = whenCheck(PM.Kernel?.commands?.get?.(command.id));
    const shown = check ? check.ask() : typeof command.when === 'function' ? command.when() : true;
    if (isThenable(shown)) {
      const now = check?.last() ? [entry] : [];
      later(now, Promise.resolve(shown).then(value => value ? [entry] : [], () => now));
    } else if (shown) entries.push(entry);
  }
  for (const layer of PM.proj?.layers ?? []) {
    if (hasQuery && matches(layer.name)) {
      entries.push({
        id: `layer:${layer.id}`,
        label: String(layer.name),
        cat: 'Layer',
        run: () => PM.selectLayers(layer.id)
      });
    }
  }
  for (const workspace of PM.WS?.list?.() ?? []) {
    if (!hasQuery || matches(workspace.name)) {
      entries.push({
        id: `workspace:${workspace.id}`,
        label: `Workspace · ${workspace.name}`,
        cat: 'Workspace',
        run: () => PM.WS.activate(workspace.id)
      });
    }
  }
  for (const [key, definition] of Object.entries(PM.FX ?? {}) as Array<[string, Record<string, any>]>) {
    if (hasQuery && matches(definition.label)) {
      entries.push({
        id: `effect:${key}`,
        label: `Effect · ${definition.label}`,
        cat: 'Effect',
        run: () => {
          const layer = PM.firstSel();
          if (layer) {
            return PM.Edit.apply(
              { type: 'add_effect', target: layer.id, effect: key },
              { label: `Add ${definition.label}`, origin: 'command-palette' }
            );
          }
        }
      });
    }
  }
  /* Extension-contributed entries come last and share the same 60-item cap.
     Providers do their own matching — the raw query is passed straight through. */
  const providers = (PM as Record<string, any>).Kernel?.paletteProviders?.() ?? [];
  for (const { provider } of providers as Array<{ provider: (q: string) => unknown }>) {
    let produced: unknown;
    try {
      produced = provider(query);
    } catch (error) {
      console.error('[palette] provider failed', error);
      continue;
    }
    if (isThenable(produced)) {
      later([], Promise.resolve(produced).then(providerEntries, error => {
        console.error('[palette] provider failed', error);
        return [];
      }));
    } else entries.push(...providerEntries(produced));
  }
  const assemble = (): PaletteEntry[] => parts.flat().slice(0, 60);
  for (const { index, answer } of answers) {
    void Promise.resolve(answer).then(settled => {
      const before = parts[index]!;
      parts[index] = settled;
      if (settled.length !== before.length || settled.some((entry, at) => entry !== before[at])) onLate?.(assemble());
    });
  }
  return assemble();
}

function providerEntries(produced: unknown): PaletteEntry[] {
  if (!Array.isArray(produced)) return [];
  const out: PaletteEntry[] = [];
  for (const item of produced as Array<Record<string, any>>) {
    if (!item || typeof item.run !== 'function') continue;
    out.push({
      id: String(item.id ?? `provider:${out.length}`),
      label: String(item.label ?? ''),
      cat: String(item.category ?? 'Extension'),
      kb: item.kb ?? null,
      run: () => item.run()
    });
  }
  return out;
}
