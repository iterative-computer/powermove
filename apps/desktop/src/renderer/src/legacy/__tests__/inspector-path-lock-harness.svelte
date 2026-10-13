<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from '../../kernel/api';
  import { provideInspectorContext } from '../../../../extensions/inspector/context';
  import StructuredSection from '../../../../extensions/inspector/StructuredSection.svelte';
  let { api, layer }: { api: PowermoveAPI; layer: any } = $props();
  provideInspectorContext(untrack(() => api));
</script>

<StructuredSection {layer} />
