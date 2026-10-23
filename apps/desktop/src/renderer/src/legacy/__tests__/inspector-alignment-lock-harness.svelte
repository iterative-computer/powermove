<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from '../../kernel/api';
  import { provideInspectorContext } from '../../../../extensions/inspector/context';
  import AlignmentStrip from '../../../../extensions/inspector/AlignmentStrip.svelte';
  let { api, layers }: { api: PowermoveAPI; layers: any[] } = $props();
  provideInspectorContext(untrack(() => api));
</script>

<AlignmentStrip {layers} />
