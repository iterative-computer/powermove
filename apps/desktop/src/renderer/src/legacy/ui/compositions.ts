/* After Effects composition dialogs: Composition Settings (new and existing)
   and Pre-compose. The model lives in core/compositions. */
import type { PMRegistry } from '../registry';
import { createNewProjectForm } from './project-settings';

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
  let mode: 'move' | 'leave' = 'move';
  let adjustDuration = false;
  let openAfter = false;
  const layerName = single ? `“${single.name}”` : 'the layer';
  const modes = [
    { id: 'move', label: 'Move into new comp', note: 'Moves the selected layers, with their transforms and effects, into a new composition.' },
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
  const adjust = PM.toggleField(() => adjustDuration, (value: boolean) => { adjustDuration = value; }, { local: true, label: 'Fit duration to layers' });
  const openToggle = PM.toggleField(() => openAfter, (value: boolean) => { openAfter = value; }, { local: true, label: 'Open new composition' });
  const adjustRow = field('Fit duration to layers', adjust);
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
  }
  const body = h('div.new-project-form.precompose-form',
    h('label.new-project-field', h('span', 'Name'), name),
    field('Attributes', segmented),
    note,
    adjustRow,
    field('Open new composition', openToggle));
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
          open: openAfter,
        });
        if (!created) throw new Error('These layers could not be pre-composed');
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
