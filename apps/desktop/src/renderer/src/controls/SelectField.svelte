<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';

  export type SelectOption = string | { v: unknown; label: string };

  let {
    PM,
    get,
    edit,
    options,
    label,
    onChange
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    options: SelectOption[];
    label?: string;
    onChange?: (value: unknown) => void;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const mixed=$derived((sel.layers,doc.tick.values,doc.proj,transport.time,PM.inspectorMixed?.(edit,value)??false));
  const gesture = $derived(new EditGesture(PM, edit));
  const optionValue = (option: SelectOption): unknown => typeof option === 'string' ? option : option.v;
  const optionLabel = (option: SelectOption): string => typeof option === 'string' ? option : option.label;
  const selectedIndex = $derived(options.findIndex((option) => Object.is(optionValue(option), value)));

  function change(event: Event): void {
    event.stopPropagation();
    const option = options[Number((event.currentTarget as HTMLSelectElement).value)];
    if (!option) return;
    const next = optionValue(option);
    gesture.once(next);
    PM.invalidate?.();
    onChange?.(next);
  }
</script>

<select
  class="sel"
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  value={mixed?-1:selectedIndex}
  onchange={change}
  onpointerdown={(event) => event.stopPropagation()}
>
  {#if mixed}<option value="-1" hidden>Mixed</option>{:else if selectedIndex === -1}<option value="-1" hidden>{String(value)}</option>{/if}
  {#each options as option, index}
    <option value={index}>{optionLabel(option)}</option>
  {/each}
</select>
