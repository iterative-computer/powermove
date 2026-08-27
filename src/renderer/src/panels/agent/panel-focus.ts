import type { Workspace } from '../../layout/model';

export interface PanelFocusOption { id: string; title: string; hidden: boolean; dockId: string }
export interface PanelFocusContext { scope: string; label: string; panels: PanelFocusOption[] }

export function panelFocusOptions(workspace: Workspace | null | undefined, definitions: Record<string, any>): PanelFocusOption[] {
  const options = new Map<string, PanelFocusOption>();
  for (const dock of workspace?.layout?.docks ?? []) {
    for (const panel of dock.panels) {
      if (!definitions[panel.id]) continue;
      options.set(panel.id, { id: panel.id, title: panel.title || definitions[panel.id].title || panel.id, hidden: !!dock.hidden, dockId: dock.id });
    }
  }
  for (const panel of workspace?.hiddenPanels ?? []) {
    if (!definitions[panel.id] || options.has(panel.id)) continue;
    options.set(panel.id, { id: panel.id, title: panel.spec?.title || definitions[panel.id].title || panel.id, hidden: true, dockId: panel.dockId || '' });
  }
  for (const [id, definition] of Object.entries(definitions)) {
    if (!options.has(id)) options.set(id, { id, title: definition.title || id, hidden: id !== 'toolbar', dockId: '' });
  }
  return [...options.values()];
}

export function scopePanelIds(scope: string): string[] {
  if (!/^panels?:/.test(scope)) return [];
  return [...new Set(scope.slice(scope.indexOf(':') + 1).split(',').filter(Boolean))].slice(0, 32);
}

export function panelScope(ids: string[]): string {
  const unique = [...new Set(ids)].slice(0, 32);
  return unique.length === 0 ? 'workspace' : `${unique.length === 1 ? 'panel' : 'panels'}:${unique.join(',')}`;
}

export function panelFocusContext(scope: string, workspace: Workspace | null | undefined, definitions: Record<string, any>): PanelFocusContext {
  const options = panelFocusOptions(workspace, definitions);
  const panels = scopePanelIds(scope).map(id => options.find(option => option.id === id)).filter((option): option is PanelFocusOption => !!option);
  return {
    scope: scope === 'composition' ? scope : panelScope(panels.map(panel => panel.id)),
    label: panels.length ? panels.map(panel => panel.title).join(', ') : scope === 'composition' ? 'Composition content' : 'All panels',
    panels
  };
}

export function panelFocusPrompt(focus: PanelFocusContext): string {
  return `USER-SELECTED PANEL FOCUS\n${JSON.stringify(focus)}\nThe user explicitly chose these panels in the composer (or by drawing a selection box). Resolve references such as "this panel" or "these controls" against this focus. Prefer these targets for interface edits and early loading placement. Other panels in a screenshot are reference, not additional targets. Keep dependent changes elsewhere minimal; this focus does not grant additional authority.`;
}

export const NATIVE_PANEL_DESIGN = `PANEL DESIGN DEFAULT
Unless the user explicitly requests a different visual style, every new or changed panel must match Powermove's existing design language one to one, as if it shipped with the app. Use api.panels.register and let the host supply the normal panel frame, header, docking and scrolling; do not draw a second card or title bar. Reuse api.ui.controls and the nearest built-in panel's patterns for fields, rows, sections, buttons and icons. Match its spacing, control heights, label alignment, typography, corner radii, borders and surfaces using the app's CSS tokens (--f-ui, --fs-md, --row-h, --ctl-h, --r-base, --bg-panel, --bg-field, --tx, --tx-2, --line, --accent). Inherit light/dark themes; do not hardcode a separate palette, oversized headings, gradients, decorative cards or a custom design system. Preserve normal panel chrome when modifying an existing panel. An explicit user style request overrides this default only for the requested surface. Keep controls connected to editable source and normal Undo.`;

export interface SelectionRect { x: number; y: number; width: number; height: number }
export function intersectingPanels(rect: SelectionRect, panels: Array<{ id: string; rect: SelectionRect }>): string[] {
  return panels.map(panel => {
    const width = Math.max(0, Math.min(rect.x + rect.width, panel.rect.x + panel.rect.width) - Math.max(rect.x, panel.rect.x));
    const height = Math.max(0, Math.min(rect.y + rect.height, panel.rect.y + panel.rect.height) - Math.max(rect.y, panel.rect.y));
    return { id: panel.id, area: width * height };
  }).filter(panel => Number.isFinite(panel.area) && panel.area > 0)
    .sort((a, b) => b.area - a.area).map(panel => panel.id);
}
