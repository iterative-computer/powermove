import { mount, unmount } from 'svelte';

import { initialSelection, presetChoices, uniquePresetName, type PresetSelection } from './model';
import SavePresetSheet from './SavePresetSheet.svelte';

type PMLike = Record<string, any>;

export interface SavePresetRequest {
  layerId?: string;
  /** Effects the user pointed at (Effect Controls selection). */
  effectIds?: string[];
  /** Channels the user pointed at (a Properties row). */
  paths?: string[];
}

/** Save Animation Preset…: a compact sheet over the editor. Returns false when there is no layer to save from. */
export function openSavePresetSheet(PM: PMLike, request: SavePresetRequest = {}): boolean {
  const layer = request.layerId ? PM.L?.(request.layerId) : PM.firstSel?.();
  if (!layer) { PM.toast?.('Select a layer to save its effects and animation as a preset'); return false; }
  if (!PM.Presets) return false;
  const choices = presetChoices(PM, layer);
  const initial = initialSelection(choices, request);
  const firstEffect = choices.effects.find((effect) => initial.effectIds.includes(effect.id));
  const onlyEffect = initial.effectIds.length === 1 && !initial.propertyIds.length && firstEffect;
  const name = uniquePresetName(PM, onlyEffect ? `${layer.name} ${firstEffect.label}` : `${layer.name} Preset`);
  const bridge: { read?: () => { name: string; selection: PresetSelection; includeKeyframes: boolean } } = {};
  const target = document.createElement('div');
  const sheet = mount(SavePresetSheet, {
    target,
    props: { choices, initial, name, layerName: layer.name, bridge, toggleField: PM.toggleField },
  });
  // Capture reads values at the playhead the sheet opened on, whatever happens behind it.
  const time = PM.time;
  PM.modal({
    title: 'Save Animation Preset', width: 460, body: target,
    onClose: () => { void unmount(sheet); },
    actions: [
      { label: 'Cancel' },
      { label: 'Save Preset', pri: true, run: () => {
        const state = bridge.read?.();
        if (!state) return false;
        const current = PM.L?.(layer.id);
        if (!current) throw new Error(`${layer.name} is no longer in this composition.`);
        const paths = choices.properties.filter((row) => state.selection.propertyIds.includes(row.id)).flatMap((row) => row.paths);
        const saved = PM.Presets.save({
          name: state.name,
          layerId: current.id,
          effectIds: state.selection.effectIds,
          propertyPaths: paths,
          includeAnimation: state.includeKeyframes,
          time,
        });
        PM.toast?.(`Saved “${saved.name}” to My Presets`, 2600, { icon: 'diamond', key: 'preset-save' });
      } },
    ],
  });
  return true;
}
