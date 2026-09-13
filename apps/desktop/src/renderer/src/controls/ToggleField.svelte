<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

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
  const isMixed=$derived((sel.layers,doc.tick.values,doc.proj,transport.time,mixed?.(edit,value)??false));
  const gesture = $derived(new EditGesture(api, edit));

  function toggle(event: MouseEvent): void {
    event.stopPropagation();
    gesture.once(isMixed?true:!value);
    api.transport.invalidate();
  }
</script>

<button
  type="button"
  class:on={value}
  class="toggle"
  class:mixed={isMixed}
  aria-pressed={isMixed?'mixed':value}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={toggle}
><i aria-hidden="true"></i></button>

<style>.toggle.mixed{background:var(--tx-3)}.toggle.mixed i{transform:translateX(6px);border-radius:2px;height:3px}</style>
