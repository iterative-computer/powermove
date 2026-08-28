/** Stable defaults for existing panels; newly authored panels supply `icon`. */
export const PANEL_ICONS: Record<string, string> = {
  viewer: 'frame', fxbrowser: 'wand', assets: 'project', mods: 'puzzle', notes: 'note',
  perf: 'speedometer', pexels: 'image', agent: 'sparkle', inspector: 'sliders',
  shader: 'code', takes: 'layers', timeline: 'timeline', toolbar: 'tools', workspaces: 'grid'
};

export function panelIcons(panels: Array<{ id: string; title?: string; icon?: string }>, icons: Record<string, unknown>) {
  const result: Record<string, string> = {};
  const used = new Set<string>();
  const candidates = ['sliders', 'graph', 'clock', 'cam', 'type', 'shape', 'diamond', 'gear', 'list', 'link', 'sun', 'eye', 'music', 'film', 'project', 'grid', 'code', 'note', 'puzzle', 'image', 'tools', 'timeline', 'speedometer'];
  for (const panel of [...panels].sort((a, b) => Number(!PANEL_ICONS[a.id]) - Number(!PANEL_ICONS[b.id]) || a.id.localeCompare(b.id))) {
    const preferred = [panel.icon, PANEL_ICONS[panel.id], ...candidates];
    const icon = preferred.find((name): name is string => !!name && name !== 'panel' && !!icons[name] && !used.has(name))
      || preferred.find((name): name is string => !!name && !!icons[name]) || 'panel';
    used.add(icon); result[panel.id] = icon;
  }
  return result;
}
