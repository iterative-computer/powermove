<script lang="ts">
  import type { GeneratedReadoutSource } from '../../core/types/workspace';
  import { generatedReadout } from '../../core/generated-bindings';
  import { doc } from '../../state/document.svelte';
  import { sel } from '../../state/selection.svelte';

  let {
    PM,
    source
  }: {
    PM: Record<string, any>;
    source: GeneratedReadoutSource;
  } = $props();

  const value = $derived((
    doc.tick.structure,
    doc.tick.values,
    sel.layers,
    sel.keys,
    generatedReadout(PM, source)
  ));
</script>

<span class="generated-tool-readout">{value}</span>
