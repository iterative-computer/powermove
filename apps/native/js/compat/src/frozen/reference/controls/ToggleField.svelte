<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { doc } from '../state/document.svelte';
  import { controlTime } from '../state/transport.svelte';
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
    mixed,
    variant = 'segmented'
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
    /** A 16px checkbox, as After Effects' Effect Controls show a switch. */
    variant?: 'segmented' | 'checkbox';
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, controlTime(), !!get()));
  const isMixed = $derived((sel.layers, doc.tick.values, doc.proj, controlTime(), mixed?.(edit, value) ?? false));
  const gesture = $derived(new EditGesture(api, edit));

  function set(next: boolean): void {
    const current = !!get();
    if (!(mixed?.(edit, current) ?? false) && next === current) return;
    gesture.once(next);
    api.transport.invalidate();
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') { event.preventDefault(); set(false); }
    else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') { event.preventDefault(); set(true); }
    else if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault();
      const current = !!get();
      set((mixed?.(edit, current) ?? false) ? true : !current);
    }
  }
</script>

{#if variant === 'checkbox'}
<button
  type="button"
  class="checkbox-field"
  class:on={!isMixed && value}
  class:mixed={isMixed}
  role="checkbox"
  aria-checked={isMixed ? 'mixed' : value}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={() => set(isMixed ? true : !get())}
>{#if isMixed}<i></i>{:else if value}<svg viewBox="0 0 256 256" aria-hidden="true"><path d="M229.66,77.66l-128,128a8,8,0,0,1-11.32,0l-56-56a8,8,0,0,1,11.32-11.32L96,188.69,218.34,66.34a8,8,0,0,1,11.32,11.32Z"/></svg>{/if}</button>
{:else}
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
{/if}
