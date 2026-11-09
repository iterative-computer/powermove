<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from 'powermove';
  import ContentSection from './ContentSection.svelte';
  import { provideInspectorContext } from './context';

  let { api, layer }: { api: PowermoveAPI; layer: any } = $props();
  provideInspectorContext(untrack(() => api));
</script>

<ContentSection {layer} />
