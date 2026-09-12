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

export function paletteEntries(PM: OverlayPM, query: string): PaletteEntry[] {
  const hasQuery = query.toLowerCase().trim().length > 0;
  const matches = (value: unknown): boolean => scorePaletteMatch(value, query) != null;
  const entries: PaletteEntry[] = [];
  for (const command of Object.values(PM.commands ?? {}) as Array<Record<string, any>>) {
    /* `when` is the kernel's "runnable by id, but not offered here" flag. */
    if (typeof command.when === 'function' && !command.when()) continue;
    if (!hasQuery || matches(command.label)) {
      entries.push({
        id: `command:${command.id}`,
        label: String(command.label),
        cat: String(command.cat ?? 'General'),
        kb: command.kb,
        run: () => PM.cmd(command.id)
      });
    }
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
  for (const entry of paletteProviderEntries(PM, query)) entries.push(entry);
  return entries.slice(0, 60);
}

function paletteProviderEntries(PM: OverlayPM, query: string): PaletteEntry[] {
  const providers = (PM as Record<string, any>).Kernel?.paletteProviders?.() ?? [];
  const out: PaletteEntry[] = [];
  for (const { provider } of providers as Array<{ provider: (q: string) => unknown }>) {
    let produced: unknown;
    try {
      produced = provider(query);
    } catch (error) {
      console.error('[palette] provider failed', error);
      continue;
    }
    if (!Array.isArray(produced)) continue;
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
  }
  return out;
}
