/* Workspace commands shared by the titlebar's workspace menu and the
   Workspaces panel, after After Effects' Window ▸ Workspace menu:
   Save Changes to This Workspace, Reset to Saved Layout, a default for new
   windows and projects, and New / Rename / Delete.

   The live layout autosaves as panels move. "Save Changes" records the
   current arrangement as the one Reset goes back to (the engine's
   `savedState`); built-in workspaces reset to their shipped layout. */

type PMLike = Record<string, any>;
type MenuItem = '-' | { header: string } | {
  label: string; icon?: string; kb?: string | null; on?: boolean; disabled?: boolean; run?: () => unknown;
};

export interface WorkspaceLike {
  id: string;
  name: string;
  builtin?: boolean;
  savedState?: Record<string, unknown>;
  [key: string]: unknown;
}

const SAVED_KEYS = ['layout', 'hiddenPanels', 'chrome', 'density', 'theme', 'features', 'custom'] as const;

/** JSON with sorted keys, so a reloaded workspace compares equal to its baseline. */
function stable(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`;
  if (value && typeof value === 'object') {
    return `{${Object.keys(value as object).sort()
      .filter((key) => (value as Record<string, unknown>)[key] !== undefined)
      .map((key) => `${JSON.stringify(key)}:${stable((value as Record<string, unknown>)[key])}`).join(',')}}`;
  }
  return JSON.stringify(value ?? null);
}

const quoted = (name: string) => `“${name}”`;

export interface WorkspaceStatus {
  workspace: WorkspaceLike | null;
  builtin: boolean;
  /** The live layout differs from what Reset would restore. */
  changed: boolean;
  /** Something to reset to: a saved layout (custom) or the shipped one (built-in). */
  canReset: boolean;
  canSave: boolean;
  isDefault: boolean;
}

export function workspaceStatus(PM: PMLike, id: string | undefined = PM.WS?.current?.id): WorkspaceStatus {
  const WS = PM.WS;
  const workspace: WorkspaceLike | null = (id && WS?.get?.(id)) || null;
  const isDefault = !!workspace && WS?.defaultId?.() === workspace.id;
  if (!workspace) return { workspace, builtin: false, changed: false, canReset: false, canSave: false, isDefault };
  const builtin = !!workspace.builtin;
  // The window's live copy can be newer than the library entry while docks are mounted.
  const live: WorkspaceLike = WS.current?.id === workspace.id ? WS.current : workspace;
  const saved = workspace.savedState;
  const changed = !!saved && SAVED_KEYS.some((key) => stable(live[key]) !== stable(saved[key]));
  return {
    workspace, builtin, changed, isDefault,
    canReset: builtin || (!!saved && changed),
    canSave: !builtin && !WS.editing && (!saved || changed),
  };
}

/** Records the current arrangement as the layout Reset returns to. */
export function saveWorkspaceChanges(PM: PMLike, id: string | undefined = PM.WS?.current?.id): boolean {
  const workspace = id ? PM.WS?.get?.(id) : null;
  if (!workspace) return false;
  if (workspace.builtin) {
    PM.toast?.('Built-in workspaces keep their layout. Use New Workspace… to keep this arrangement.', 3600);
    return false;
  }
  if (!PM.WS.saveLayout(id)) {
    PM.toast?.('Finish editing the workspace before saving it.', 3200, { error: true });
    return false;
  }
  PM.toast?.(`Saved ${quoted(workspace.name)}. Reset now returns to this layout.`, 2600, { icon: 'panel', key: 'workspace-status' });
  PM.bus?.emit?.('workspaces');
  return true;
}

export function resetWorkspace(PM: PMLike, id: string | undefined = PM.WS?.current?.id): boolean {
  const workspace = id ? PM.WS?.get?.(id) : null;
  if (!workspace) return false;
  if (!PM.WS.reset(id)) {
    PM.toast?.(`${quoted(workspace.name)} has no saved layout to return to yet.`, 3200);
    return false;
  }
  PM.toast?.(`Reset ${quoted(workspace.name)} to its saved layout`, 2400, { icon: 'undo', key: 'workspace-status' });
  return true;
}

/** Choosing the default again clears it, so new windows follow the last-used workspace. */
export function toggleDefaultWorkspace(PM: PMLike, id: string | undefined = PM.WS?.current?.id): boolean {
  const workspace = id ? PM.WS?.get?.(id) : null;
  if (!workspace) return false;
  const clearing = PM.WS.defaultId?.() === workspace.id;
  if (!PM.WS.setDefault(clearing ? null : workspace.id)) return false;
  PM.toast?.(clearing
    ? 'No default workspace. New windows open in the last one you used.'
    : `New windows and projects open in ${quoted(workspace.name)}`, 2800, { key: 'workspace-status' });
  return true;
}

function nameField(PM: PMLike, value: string, label: string): HTMLInputElement {
  const input = document.createElement('input');
  input.className = 'settings-input is-wide';
  input.type = 'text';
  input.value = value;
  input.maxLength = 80;
  input.spellcheck = false;
  input.setAttribute('aria-label', label);
  return input;
}

function nameForm(input: HTMLInputElement, note?: string): HTMLElement {
  const form = document.createElement('div');
  form.className = 'new-project-form';
  const field = document.createElement('label');
  field.className = 'new-project-field';
  const caption = document.createElement('span');
  caption.textContent = 'Name';
  field.append(caption, input);
  form.append(field);
  if (note) {
    const line = document.createElement('p');
    line.className = 'precompose-note';
    line.textContent = note;
    form.append(line);
  }
  return form;
}

const focusSoon = (input: HTMLInputElement) => window.setTimeout(() => { input.focus(); input.select(); }, 30);

/** New Workspace…: the current arrangement under a new name, made current. */
export function newWorkspaceDialog(PM: PMLike): void {
  const current = PM.WS?.current;
  if (!current) return;
  const input = nameField(PM, `${String(current.name).replace(/ \(edited\)$/, '')} copy`, 'Workspace name');
  PM.modal({
    title: 'New Workspace', width: 420,
    body: nameForm(input, 'Saves the panels as they are now. The new workspace becomes current.'),
    actions: [
      { label: 'Cancel' },
      { label: 'Create', pri: true, run: () => {
        const created = PM.WS.saveCurrent(input.value);
        PM.toast?.(`Created ${quoted(created.name)}`, 2200, { icon: 'panel', key: 'workspace-status' });
      } },
    ],
  });
  focusSoon(input);
}

export function renameWorkspaceDialog(PM: PMLike, id: string | undefined = PM.WS?.current?.id): void {
  const workspace = id ? PM.WS?.get?.(id) : null;
  if (!workspace || workspace.builtin) return;
  const input = nameField(PM, workspace.name, 'Workspace name');
  PM.modal({
    title: 'Rename Workspace', width: 420, body: nameForm(input),
    actions: [
      { label: 'Cancel' },
      { label: 'Rename', pri: true, run: () => {
        if (input.value.trim() === workspace.name) return;
        if (!PM.WS.rename(workspace.id, input.value)) throw new Error('Name the workspace (up to 80 characters).');
        PM.bus?.emit?.('workspaces');
      } },
    ],
  });
  focusSoon(input);
}

/** Deleted workspaces go to the engine's trash, so the toast can undo. */
export async function deleteWorkspace(PM: PMLike, id: string | undefined = PM.WS?.current?.id): Promise<boolean> {
  const workspace = id ? PM.WS?.get?.(id) : null;
  if (!workspace || workspace.builtin) return false;
  const ok = await PM.confirm?.({
    message: `Delete ${quoted(workspace.name)} workspace?`,
    detail: 'This removes the saved workspace layout. Your project and its layers will not be deleted.',
    confirmLabel: 'Delete Workspace',
  });
  if (!ok) return false;
  const wasCurrent = PM.WS.current?.id === workspace.id;
  const wasDefault = PM.WS.defaultId?.() === workspace.id;
  PM.WS.remove(workspace.id);
  PM.toast?.(`Deleted ${quoted(workspace.name)}`, 5000, {
    key: 'workspace-status',
    action: { label: 'Undo', run: () => {
      const restored = PM.WS.restore?.(workspace.id);
      if (!restored) return;
      if (wasDefault) PM.WS.setDefault?.(restored.id);
      if (wasCurrent) PM.WS.activate(restored.id, true);
      PM.bus?.emit?.('workspaces');
    } },
  });
  return true;
}

/** The workspace-management rows, below the list of workspaces. */
export function workspaceManageItems(PM: PMLike, id: string | undefined = PM.WS?.current?.id): MenuItem[] {
  const status = workspaceStatus(PM, id);
  const workspace = status.workspace;
  const custom = !!workspace && !status.builtin;
  /* Disabled rows say why in the trailing column, where a Mac menu puts
     secondary text, so the menu never changes shape. */
  const saveHint = status.builtin ? 'Built-in' : status.canSave ? null : 'Saved';
  const resetHint = status.canReset ? null : status.builtin || workspace?.savedState ? 'Unchanged' : 'Nothing saved';
  return [
    { label: 'Save Changes to This Workspace', disabled: !status.canSave, kb: saveHint, run: () => saveWorkspaceChanges(PM, id) },
    { label: 'Reset to Saved Layout', disabled: !status.canReset, kb: resetHint, run: () => resetWorkspace(PM, id) },
    { label: 'Set as Default Workspace', on: status.isDefault, disabled: !workspace, run: () => toggleDefaultWorkspace(PM, id) },
    '-',
    { label: 'New Workspace…', run: () => newWorkspaceDialog(PM) },
    { label: 'Rename Workspace…', disabled: !custom, run: () => renameWorkspaceDialog(PM, id) },
    { label: 'Delete Workspace…', disabled: !custom, run: () => { void deleteWorkspace(PM, id); } },
  ];
}

/** The titlebar's full workspace menu: switcher, management, then the panel library. */
export function workspaceMenuItems(PM: PMLike): MenuItem[] {
  const all = (PM.WS?.list?.() ?? []) as WorkspaceLike[];
  const current = PM.WS?.current?.id;
  const defaultId = PM.WS?.defaultId?.();
  return [
    { header: 'Workspaces' },
    ...all.map((item) => ({
      label: item.name,
      on: item.id === current,
      kb: item.id === defaultId ? 'Default' : null,
      run: () => { if (item.id !== current) PM.WS.activate(item.id); },
    })),
    '-',
    ...workspaceManageItems(PM, current),
    '-',
    { label: 'Panel Library…', icon: 'grid', run: () => PM.LibraryUI?.open?.() },
  ];
}
