<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from '../../kernel/api';
  import { provideInspectorContext } from '../../../../extensions/inspector/context';
  import ChannelRow from '../../../../extensions/inspector/ChannelRow.svelte';
  let { api, layer }: { api: PowermoveAPI; layer: any } = $props();
  provideInspectorContext(untrack(() => api));
</script>

<ChannelRow {layer} channel="opacity" label="Opacity" />
