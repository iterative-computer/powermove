<script lang="ts">
  import { inspectorContext } from './context';

  let { name }: { name: string } = $props();
  const { api } = inspectorContext();
</script>

{@html api.ui.icon(name)}
