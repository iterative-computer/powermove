import type { Workspace } from '../../layout/model';

export const UI_PLACEMENT_PREFIX = 'POWERMOVE_UI_TARGET ';

/** A temporary visual location, never a workspace edit or permission to edit. */
export type UIPlacement =
  | {
      kind: 'panel';
      id: string;
      label: string;
      /** Optional descendant of the panel that owns the work. */
      selector?: string;
      /** Reserve a new section beside the descendant instead of covering it. */
      insert?: 'before' | 'after';
    }
  | { kind: 'dock'; id: string; label: string; beforePanelId: string | null };

export function isUIPlacementMessage(text: unknown): text is string {
  return typeof text === 'string' && text.trimStart().startsWith(UI_PLACEMENT_PREFIX);
}

/** Separate line-leading protocol messages from prose, including incomplete
 * streamed prefixes. Never interpret an example embedded in a sentence. */
export function splitUIPlacementText(source: string, streaming = false): { text: string; messages: string[] } {
  const marker = UI_PLACEMENT_PREFIX.trimEnd();
  const pattern = /(^|\n)[ \t]*POWERMOVE_UI_TARGET(?=\s|\{|$)/g;
  const messages: string[] = [];
  let text = '', cursor = 0;
  for (let match; (match = pattern.exec(source));) {
    text += source.slice(cursor, match.index) + match[1];
    const start = pattern.lastIndex;
    let jsonStart = start;
    while (/\s/.test(source[jsonStart] || '') && jsonStart < source.length) jsonStart++;
    let end = source.length;
    if (source[jsonStart] === '{') {
      let depth = 0, quoted = false, escaped = false;
      for (let i = jsonStart; i < source.length; i++) {
        const char = source[i];
        if (quoted) {
          if (escaped) escaped = false;
          else if (char === '\\') escaped = true;
          else if (char === '"') quoted = false;
        } else if (char === '"') quoted = true;
        else if (char === '{') depth++;
        else if (char === '}' && --depth === 0) { end = i + 1; break; }
      }
      if (end <= source.length && source[end - 1] === '}') messages.push(UI_PLACEMENT_PREFIX + source.slice(jsonStart, end));
    } else {
      const newline = source.indexOf('\n', start);
      if (newline !== -1) end = newline;
    }
    cursor = end;
    pattern.lastIndex = end;
  }
  text += source.slice(cursor);
  if (streaming) {
    text = text.replace(/(^|\n)([ \t]*)([A-Z_]+)$/, (match, newline, _space, tail) => marker.startsWith(tail) ? newline : match);
  }
  return { text, messages };
}

export function parseUIPlacement(text: string, workspace: Workspace | null | undefined): UIPlacement | null {
  if (!isUIPlacementMessage(text) || text.length > 600) return null;
  try {
    const value = JSON.parse(text.trim().slice(UI_PLACEMENT_PREFIX.length));
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    if (typeof value.id !== 'string' || !/^[a-zA-Z0-9_-]{1,80}$/.test(value.id)) return null;
    if (typeof value.label !== 'string' || !value.label.trim() || value.label.length > 64) return null;
    const label = value.label.replace(/[\u0000-\u001f\u007f]/g, ' ').trim();
    if (!label) return null;
    const docks = workspace?.layout?.docks?.filter(dock => !dock.hidden) ?? [];
    if (value.kind === 'panel' && docks.some(dock => dock.panels.some(panel => panel.id === value.id))) {
      const selector = value.selector;
      const insert = value.insert;
      if (selector !== undefined && (typeof selector !== 'string' || !selector.trim() || selector.length > 160 || /[\u0000-\u001f\u007f]/.test(selector))) return null;
      if (insert !== undefined && insert !== 'before' && insert !== 'after') return null;
      if (insert !== undefined && selector === undefined) return null;
      return {
        kind: 'panel', id: value.id, label,
        ...(selector === undefined ? {} : { selector: selector.trim() }),
        ...(insert === undefined ? {} : { insert })
      };
    }
    if (value.kind === 'dock') {
      const dock = docks.find(dock => dock.id === value.id);
      if (!dock) return null;
      const beforePanelId = value.beforePanelId ?? null;
      if (beforePanelId !== null && !dock.panels.some(panel => panel.id === beforePanelId)) return null;
      return { kind: 'dock', id: dock.id, label, beforePanelId };
    }
  } catch { /* Ignore malformed model output without interrupting the run. */ }
  return null;
}

export function uiPlacementInstructions(workspace: Workspace | null | undefined): string {
  const docks = (workspace?.layout?.docks ?? []).filter(dock => !dock.hidden).slice(0, 12).map(dock => ({
    id: dock.id,
    panels: dock.panels.slice(0, 32).map(panel => ({ id: panel.id, title: panel.title || panel.id }))
  }));
  return `EARLY INTERFACE PLACEMENT
For an interface/panel change, first inspect only enough context to decide where it belongs. Before editing files or building controls, send one standalone public commentary message using one of these exact forms:
${UI_PLACEMENT_PREFIX}{"kind":"panel","id":"EXISTING_PANEL_ID","label":"Short description"}
${UI_PLACEMENT_PREFIX}{"kind":"panel","id":"EXISTING_PANEL_ID","selector":".selector-inside-panel","label":"Specific area"}
${UI_PLACEMENT_PREFIX}{"kind":"panel","id":"EXISTING_PANEL_ID","selector":".existing-section","insert":"after","label":"New section title"}
${UI_PLACEMENT_PREFIX}{"kind":"dock","id":"EXISTING_DOCK_ID","beforePanelId":null,"label":"New panel title"}
Use kind=panel when modifying an existing visible panel. Inspect the panel's DOM or source early: when you can identify the exact area, include a selector that resolves inside that panel. For a new section inside an existing panel, include its nearest existing sibling's selector and insert=before or insert=after so space is reserved where the section will appear. Omit selector only when the change affects the whole panel or a precise target cannot yet be verified. For a new panel, use kind=dock with beforePanelId set to an existing panel in that dock, or null to append. Choose only ids from the live layout below. Keep label under 65 characters and selectors under 161 characters. No markdown fences or extra prose in the placement message. This is public placement metadata, not private reasoning. Powermove will show a click-through loading ghost there while you work; it never persists a workspace layout change. Keep the implementation at the announced location, or send a revised placement before changing course. Do not emit a placement for scene/media-only work. Then continue the task and return the usual final result. Never restart or reopen Powermove for a panel change.
LIVE PANEL LOCATIONS
${JSON.stringify(docks)}`;
}
