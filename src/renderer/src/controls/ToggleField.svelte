<script lang="ts">
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';

  let {
    PM,
    get,
    edit,
    label
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, !!get()));
  const gesture = $derived(new EditGesture(PM, edit));

  function toggle(event: MouseEvent): void {
    event.stopPropagation();
    gesture.once(!value);
    PM.invalidate?.();
  }
</script>

<button
  type="button"
  class:on={value}
  class="toggle"
  aria-pressed={value}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={toggle}
><i aria-hidden="true"></i></button>
