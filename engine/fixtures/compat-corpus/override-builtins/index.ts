import type { Disposable, PowermoveAPI } from 'powermove';

/*
 * Overrides by id of every built-in contribution (owner amendment 24),
 * written as user extensions do it: the same id, registered after the
 * built-ins (the registries are last-wins stacks). Effects and transitions
 * get a visibly different program (the built-in's, its output then marked
 * pure green where it covers), so a frame shows whose program drew it; every other
 * kind registers a definition of its own that delegates to the built-in's,
 * so the editor keeps working while the override is the one in use.
 *
 * Commands: `override-builtins.state` reports what was overridden;
 * `override-builtins.toggle` disposes the overrides (the built-ins come
 * back) or registers them again.
 */
const BUILTINS = ['effects-basic', 'transitions-basic', 'theme-default', 'keymap-default', 'mods', 'toolbar', 'timeline', 'inspector', 'layers-3d', 'viewer'];
const SERVICES = ['viewer', 'tool', 'timeline', 'inspector', 'shaderHooks'];
/** The program with its output marked: pure green where it covers (alpha
 *  kept), at the end of `main` (a body, or a whole shader). */
const MARK = '\n  o = vec4(0.0, o.a, 0.0, o.a);';
const swapped = (frag: string): string => {
  if (!/void\s+main\s*\(/.test(frag)) return `${frag}${MARK}`;
  const end = frag.lastIndexOf('}');
  return `${frag.slice(0, end)}${MARK}\n${frag.slice(end)}`;
};

export default function activate(api: PowermoveAPI): void {
  const K = (api.host.pm as any).Kernel;
  const builtinEntries = (registry: any) => (registry?.entries?.() ?? []).filter((e: any) => BUILTINS.includes(e.ownerId));
  const hits: Record<string, number> = {};
  let active: Disposable[] = [];
  const overridden: Record<string, string[]> = {};
  const note = (kind: string, id: string) => (overridden[kind] ??= []).push(id);

  const register = () => {
    for (const { item } of builtinEntries(K.effects)) {
      active.push(api.effects.register({ ...item, frag: swapped(item.frag) }));
      note('effects', item.id);
    }
    for (const { item } of builtinEntries(K.transitions)) {
      active.push(api.transitions.register({ ...item, frag: swapped(item.frag) }));
      note('transitions', item.id);
    }
    for (const { item } of builtinEntries(K.commands)) {
      const run = item.run;
      active.push(api.commands.register({ ...item, run: (...args: unknown[]) => { hits[item.id] = (hits[item.id] ?? 0) + 1; return run(...args); } }));
      note('commands', item.id);
    }
    for (const { item } of builtinEntries(K.panels)) {
      active.push(api.panels.register({ ...item }));
      note('panels', item.id);
    }
    for (const { item } of builtinEntries(K.keybindings)) {
      const { id: _id, chord: _chord, ...definition } = item;
      active.push(api.keybindings.bind({ ...definition, key: item.key ?? item.chord, command: item.command }));
      note('keybindings', String(item.id));
    }
    for (const { item } of builtinEntries(K.themes)) {
      active.push(api.theme.register({ ...item }));
      note('themes', item.id);
    }
    for (const { item } of builtinEntries(K.layerTypes)) {
      active.push(api.layers.register({ ...item }));
      note('layers', item.id);
    }
    for (const { item } of builtinEntries(K.inspectorSections)) {
      active.push(api.inspector.registerSection({ ...item }));
      note('inspector', item.id);
    }
    for (const provider of (K.paletteProviders?.() ?? []).filter((p: any) => BUILTINS.includes(p.ownerId))) {
      active.push(api.palette.registerProvider((query: string) => provider.provider(query)));
      note('palette', provider.ownerId);
    }
    for (const name of SERVICES) {
      const current = api.services.get<object>(name);
      if (!current) continue;
      // The same members, through an object of this extension's own.
      active.push(api.services.register(name, new Proxy(current, {})));
      note('services', name);
    }
  };
  register();

  api.commands.register({ id: `${api.id}.state`, label: 'Override state', run: () => ({ active: active.length > 0, overridden, hits: { ...hits } }) });
  api.commands.register({
    id: `${api.id}.toggle`,
    label: 'Toggle overrides',
    run: () => {
      if (active.length) { for (const d of active.splice(0).reverse()) d.dispose(); for (const k of Object.keys(overridden)) delete overridden[k]; }
      else register();
      return active.length > 0;
    }
  });
}
