/* After Effects composition dialogs: Composition Settings (new and existing)
   and Pre-compose. The model lives in core/compositions. */
import type { PMRegistry } from '../registry';
import { createNewProjectForm } from './project-settings';

/** Composition ▸ New Composition (⌘N) / Composition Settings (⌥⌘K). */
export function compositionSettingsDialog(PM: PMRegistry, id?: string | null): void {
  const existing = id ? PM.Comps.get(id) : null;
  if (id && !existing) return;
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
      { label: 'OK', pri: true, run: () => {
        const values = form.values();
        if (existing) PM.Comps.setSettings(existing.id, values);
        else PM.Comps.create(values);
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
  const radio = (value: string, checked: boolean, disabled: boolean) =>
    h('input', { type: 'radio', name: 'precompose-mode', value, checked, disabled });
  const leave = radio('leave', false, !canLeave);
  const move = radio('move', true, false);
  const adjust = h('input', { type: 'checkbox' });
  const openAfter = h('input', { type: 'checkbox' });
  const option = (input: HTMLElement, label: string, detail: string) =>
    h('label.precompose-option', input, h('span', h('b', label), h('small', detail)));
  const sync = () => {
    adjust.disabled = !move.checked;
    adjust.closest('label')?.classList.toggle('is-dim', adjust.disabled);
  };
  leave.addEventListener('change', sync); move.addEventListener('change', sync);
  const body = h('div.precompose-form',
    h('label.new-project-field', h('span', 'New composition name'), name),
    h('div.precompose-options', { role: 'radiogroup', 'aria-label': 'Pre-compose' },
      option(leave, `Leave all attributes in “${single ? single.name : 'layer'}”`,
        canLeave
          ? 'Creates a composition with only this layer’s source in it. The new composition becomes the source of the layer.'
          : 'Available for a single layer with a source.'),
      option(move, 'Move all attributes into the new composition',
        'Places the selected layers together into a new composition.')),
    h('label.precompose-check', adjust, h('span', 'Adjust composition duration to the time span of the selected layers')),
    h('label.precompose-check', openAfter, h('span', 'Open New Composition')));
  sync();
  PM.modal({
    title: 'Pre-compose', body, width: 460,
    actions: [
      { label: 'Cancel' },
      { label: 'OK', pri: true, run: () => {
        if (PM.proj.id !== projectId || PM.Comps.active() !== compositionId) {
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
          mode: leave.checked ? 'leave' : 'move',
          adjustDuration: move.checked && adjust.checked,
          open: openAfter.checked,
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
  void PM.confirm({
    message: `Delete “${comp.name}”?`,
    detail: `It is used by ${uses} ${uses === 1 ? 'layer' : 'layers'} in other compositions, which will also be removed. You can undo this.`,
    confirmLabel: 'Delete',
  }).then((ok: boolean) => { if (ok) PM.Comps.remove(id); });
}
