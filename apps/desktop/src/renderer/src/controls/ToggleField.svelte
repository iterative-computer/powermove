<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

  /* On/off as a two-segment control with a gliding pill, after dialkit's
     Toggle: the state is read as a word, not a switch position, and the
     control shares the inspector's well material. Left/Right arrows move
     between the segments. */

  let {
    api,
    get,
    edit,
    label,
    mixed
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, !!get()));
  const isMixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, mixed?.(edit, value) ?? false));
  const gesture = $derived(new EditGesture(api, edit));

  function set(next: boolean): void {
    if (!isMixed && next === value) return;
    gesture.once(next);
    api.transport.invalidate();
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') { event.preventDefault(); set(false); }
    else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') { event.preventDefault(); set(true); }
    else if (event.key === ' ' || event.key === 'Enter') { event.preventDefault(); set(isMixed ? true : !value); }
  }
</script>

<div
  class="onoff"
  class:on={value}
  class:mixed={isMixed}
  role="radiogroup"
  tabindex="-1"
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  onpointerdown={(event) => event.stopPropagation()}
  onkeydown={onKey}
>
  {#if !isMixed}<span class="onoff-pill" aria-hidden="true"></span>{/if}
  <button type="button" role="radio" aria-checked={!isMixed && !value} tabindex={!value || isMixed ? 0 : -1} onclick={() => set(false)}>Off</button>
  <button type="button" role="radio" aria-checked={!isMixed && value} tabindex={value && !isMixed ? 0 : -1} onclick={() => set(true)}>On</button>
</div>
