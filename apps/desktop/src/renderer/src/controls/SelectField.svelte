<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';
  import { fancySelect } from './select/enhance';
  import type { PowermoveAPI } from '../kernel/api';

  export type SelectOption = string | { v: unknown; label: string };

  let {
    api,
    get,
    edit,
    options,
    label,
    onChange,
    mixed
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    options: SelectOption[];
    label?: string;
    onChange?: (value: unknown) => void;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const isMixed=$derived((sel.layers,doc.tick.values,doc.proj,transport.time,mixed?.(edit,value)??false));
  const gesture = $derived(new EditGesture(api, edit));
  const optionValue = (option: SelectOption): unknown => typeof option === 'string' ? option : option.v;
  const optionLabel = (option: SelectOption): string => typeof option === 'string' ? option : option.label;
  const selectedIndex = $derived(options.findIndex((option) => Object.is(optionValue(option), value)));

  function change(event: Event): void {
    event.stopPropagation();
    const option = options[Number((event.currentTarget as HTMLSelectElement).value)];
    if (!option) return;
    const next = optionValue(option);
    gesture.once(next);
    api.transport.invalidate();
    onChange?.(next);
  }
</script>

<select
  class="sel"
  use:fancySelect={isMixed ? -1 : selectedIndex}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  value={isMixed?-1:selectedIndex}
  onchange={change}
  onpointerdown={(event) => event.stopPropagation()}
>
  {#if isMixed}<option value="-1" hidden>Mixed</option>{:else if selectedIndex === -1}<option value="-1" hidden>{String(value)}</option>{/if}
  {#each options as option, index}
    <option value={index}>{optionLabel(option)}</option>
  {/each}
</select>
