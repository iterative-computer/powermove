<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from '../../kernel/api';
  import { provideInspectorContext } from '../../../../extensions/inspector/context';
  import TypeAxis from '../../../../extensions/inspector/TypeAxis.svelte';
  let { api, layer }: { api: PowermoveAPI; layer: any } = $props();
  provideInspectorContext(untrack(() => api));
</script>

<TypeAxis {layer} axis={{ tag: 'wght', label: 'Weight', default: 400, min: 100, max: 900 }} />
