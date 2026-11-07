/* After Effects composition dialogs: Composition Settings (new and existing)
   and Pre-compose. The model lives in core/compositions. */
import type { PMRegistry } from '../registry';
import { createNewProjectForm } from './project-settings';
import { selectionBounds } from '../core/selection-bounds';

/** Composition ▸ New Composition (⌘N) / Composition Settings (⌥⌘K). */
export function compositionSettingsDialog(PM: PMRegistry, id?: string | null): void {
  const existing = id ? PM.Comps.get(id) : null;
  if (id && !existing) return;
  const projectId = PM.proj.id;
  /* AE seeds a new comp from the one open; settings seed from the comp itself. */
  const source = existing || PM.Comps.get(PM.Comps.active());
  const seed = {
    name: existing ? existing.name : PM.Comps.list().length ? nextCompName(PM) : 'Comp 1',
    w: source?.w, h: source?.h, fps: source?.fps, dur: source?.dur, bg: existing ? existing.bg : '#000000',
  };
  const form = createNewProjectForm(seed, {
    nameLabel: 'Composition Name',
    backgroundField: PM.colorField ? (get, set) => PM.colorField(get, set, { label: 'Background', local: true }) : undefined,
  });
  PM.modal({
    title: existing ? 'Composition Settings' : 'New Composition', body: form.element, width: 420,
    actions: [
      { label: 'Cancel' },
      /* A button that names what it does, as the HIG asks of dialogs. */
      { label: existing ? 'Apply' : 'Create', pri: true, run: () => {
        if (PM.projectLoading || PM.proj.id !== projectId) {
          throw new Error('The project has changed or is still opening. Close this dialog and try again.');
        }
        const values = form.values();
        if (existing) {
          if (!PM.Comps.exists(existing.id) || !PM.Comps.setSettings(existing.id, values)) {
            throw new Error('This composition is no longer available. Close this dialog and try again.');
          }
        } else PM.Comps.create(values);
      } },
    ],
  });
  window.setTimeout(() => form.focus(), 30);
}

function nextCompName(PM: PMRegistry): string {
  const taken = new Set(PM.Comps.list().map((comp: any) => comp.name));
  for (let n = 1; ; n++) if (!taken.has(`Comp ${n}`)) return `Comp ${n}`;
}

export const PRECOMPOSE_OPTIONS_KEY = 'precomposeOptions';
export interface PrecomposeOptions { mode: 'move' | 'leave'; adjustDuration: boolean; sizeToSelection: boolean; open: boolean }

/** The Pre-compose dialog's last-used choices, from the app's preference store. */
export function readPrecomposeOptions(PM: PMRegistry): PrecomposeOptions {
  let saved: any = null;
  try { saved = PM.store?.get?.(PRECOMPOSE_OPTIONS_KEY, null); } catch { saved = null; }
  const flag = (value: unknown) => value === true;
  return {
    mode: saved?.mode === 'leave' ? 'leave' : 'move',
    adjustDuration: flag(saved?.adjustDuration),
    sizeToSelection: flag(saved?.sizeToSelection),
    open: flag(saved?.open),
  };
}

/** Layer ▸ Pre-compose (⇧⌘C). */
export function precomposeDialog(PM: PMRegistry, ids: string[] = PM.sel.layers): void {
  const layers = ids.map((layerId: string) => PM.L(layerId)).filter(Boolean);
  if (!layers.length) { PM.toast?.('Select one or more layers to pre-compose'); return; }
  const locked = (layer: any) => layer.lock || (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock);
  if (layers.some(locked)) {
    PM.toast?.('Selected layers are locked. Unlock them or their group before pre-composing.');
    return;
  }
  const projectId = PM.proj.id, compositionId = PM.Comps.active();
  const targets = layers.map((layer: any) => layer.id);
  const h = PM.h;
  const single = layers.length === 1 ? layers[0] : null;
  const canLeave = !!single && PM.Comps.canLeave(single);
  const taken = new Set(PM.Comps.list().map((comp: any) => comp.name));
  let n = 1;
  while (taken.has(`Pre-comp ${n}`)) n++;
  const name = h('input.settings-input.is-wide', { type: 'text', value: `Pre-comp ${n}`, 'aria-label': 'New composition name', spellcheck: false });
  /* AE's two modes as one segmented choice, with a line that says what the
     selected one does. Keeping attributes needs a single layer with a source. */
  /* The last choices come back next time, as After Effects remembers them.
     Keep on layer only returns where it is available. */
  const remembered = readPrecomposeOptions(PM);
  let mode: 'move' | 'leave' = remembered.mode === 'leave' && canLeave ? 'leave' : 'move';
  let adjustDuration = remembered.adjustDuration;
  let sizeToSelection = remembered.sizeToSelection;
  let openAfter = remembered.open;
  /* What “Move layers” would create, measured the same way the engine does,
     so the dialog can say the new size before anything changes. */
  const picked = (() => {
    const expanded = new Set(PM.expandGroups ? PM.expandGroups(targets) : targets);
    return PM.proj.layers.filter((layer: any) => expanded.has(layer.id));
  })();
  let fitted: { w: number; h: number } | null = null, sizeIssue = '';
  try { fitted = selectionBounds(PM, picked, PM.time); } catch (error) { sizeIssue = error instanceof Error ? error.message : String(error); }
  const timed = picked.filter((layer: any) => layer.type !== 'group');
  const fittedDuration = timed.length
    ? Math.max(1 / PM.proj.fps, Math.max(...timed.map((layer: any) => layer.from + layer.dur)) - Math.min(...timed.map((layer: any) => layer.from)))
    : PM.proj.dur;
  const layerName = single ? `“${single.name}”` : 'the layer';
  const modes = [
    { id: 'move', label: 'Move layers', note: 'Moves the selected layers, with their transforms and effects, into a new composition.' },
    { id: 'leave', label: `Keep on layer`, note: `${layerName} keeps its transforms, effects and timing here. Only its source moves into the new composition.` },
  ] as const;
  const segmented = h('div.segmented', { role: 'radiogroup', 'aria-label': 'Attributes' });
  const note = h('p.precompose-note');
  const buttons = modes.map((item) => {
    const disabled = item.id === 'leave' && !canLeave;
    const button = h('button', { type: 'button', role: 'radio', disabled, title: disabled ? 'Available for a single layer with a source, such as footage, an image or a solid' : '' }, item.label);
    button.addEventListener('click', () => { mode = item.id; sync(); });
    segmented.appendChild(button);
    return button;
  });
  const field = (label: string, control: HTMLElement) => h('div.new-project-field', h('span', label), control);
  const adjust = PM.toggleField(() => adjustDuration, (value: boolean) => { adjustDuration = value; sync(); }, { local: true, label: 'Fit duration to layers' });
  const openToggle = PM.toggleField(() => openAfter, (value: boolean) => { openAfter = value; }, { local: true, label: 'Open new composition' });
  const sizeToggle = PM.toggleField(() => sizeToSelection, (value: boolean) => { sizeToSelection = value; sync(); }, { local: true, label: 'Fit size to layers' });
  const adjustRow = field('Fit duration to layers', adjust);
  const sizeRow = field('Fit size to layers', sizeToggle);
  sizeRow.dataset.precomposeSize = '';
  /* After Effects' “Adjust composition size to the selected layers”. */
  sizeRow.title = sizeIssue || 'Crops the new composition to the selected layers’ bounds. The pre-comp layer is placed so everything stays exactly where it is.';
  const summary = h('p.precompose-note.precompose-summary', { 'aria-live': 'polite' });
  const seconds = (value: number) => `${Number(value.toFixed(2))} s`;
  function sync(): void {
    buttons.forEach((button: HTMLButtonElement, index: number) => {
      const on = modes[index]!.id === mode;
      button.classList.toggle('on', on);
      button.setAttribute('aria-checked', String(on));
    });
    note.textContent = modes.find((item) => item.id === mode)!.note;
    // Duration fitting only applies when the layers move.
    adjustRow.classList.toggle('is-dim', mode !== 'move');
    adjust.querySelectorAll('button').forEach((button: HTMLButtonElement) => { button.disabled = mode !== 'move'; });
    // Size fitting also needs the layers to move, and layers it can measure.
    const sizeable = mode === 'move' && !!fitted;
    sizeRow.classList.toggle('is-dim', !sizeable);
    sizeToggle.querySelectorAll('button').forEach((button: HTMLButtonElement) => { button.disabled = !sizeable; });
    const w = sizeToSelection && fitted ? fitted.w : PM.proj.w, hgt = sizeToSelection && fitted ? fitted.h : PM.proj.h;
    summary.textContent = `New composition: ${w} × ${hgt} px, ${seconds(adjustDuration ? fittedDuration : PM.proj.dur)}`;
    summary.hidden = mode !== 'move';
  }
  const body = h('div.new-project-form.precompose-form',
    h('label.new-project-field', h('span', 'Name'), name),
    field('Attributes', segmented),
    note,
    adjustRow,
    sizeRow,
    field('Open new composition', openToggle),
    summary);
  sync();
  PM.modal({
    title: 'Pre-compose', body, width: 440,
    actions: [
      { label: 'Cancel' },
      { label: 'Pre-compose', pri: true, run: () => {
        if (PM.projectLoading || PM.proj.id !== projectId || PM.Comps.active() !== compositionId) {
          throw new Error('The project or composition has changed. Close this dialog and try again.');
        }
        const current = targets.map((id: string) => PM.L(id));
        if (current.some((layer: any) => !layer)) {
          throw new Error('A selected layer is no longer available. Close this dialog and try again.');
        }
        if (current.some(locked)) {
          throw new Error('These layers are now locked. Unlock them or their group before pre-composing.');
        }
        const created = PM.Comps.precompose(targets, {
          name: name.value.trim() || `Pre-comp ${n}`,
          mode,
          adjustDuration: mode === 'move' && adjustDuration,
          sizeToSelection: mode === 'move' && sizeToSelection && !!fitted,
          open: openAfter,
        });
        if (!created) throw new Error('These layers could not be pre-composed');
        // Remember what was asked for, even where this selection could not use it.
        PM.store?.set?.(PRECOMPOSE_OPTIONS_KEY, { mode, adjustDuration, sizeToSelection, open: openAfter } satisfies PrecomposeOptions);
      } },
    ],
  });
  window.setTimeout(() => { name.focus(); name.select(); }, 30);
}

/** Project panel ▸ Delete, with AE's warning when the comp is in use. */
export function deleteCompositionPrompt(PM: PMRegistry, id: string): void {
  const comp = PM.Comps.get(id);
  if (!comp) return;
  if (PM.Comps.list().length <= 1) { PM.toast?.('A project needs at least one composition'); return; }
  const uses = PM.Comps.users(id).length;
  if (!uses) { PM.Comps.remove(id); return; }
  const projectId = PM.proj.id;
  void PM.confirm({
    message: `Delete “${comp.name}”?`,
    detail: `It is used by ${uses} ${uses === 1 ? 'layer' : 'layers'} in other compositions, which will also be removed. You can undo this.`,
    confirmLabel: 'Delete',
  }).then((ok: boolean) => {
    if (!ok) return;
    if (PM.projectLoading || PM.proj.id !== projectId) {
      PM.toast?.('The project has changed or is still opening. Select the composition again before deleting it.');
      return;
    }
    if (!PM.Comps.exists(id)) { PM.toast?.('This composition is no longer available.'); return; }
    if (PM.Comps.list().length <= 1) { PM.toast?.('A project needs at least one composition'); return; }
    PM.Comps.remove(id);
  });
}
