<script lang="ts">
  import Icon from '../Icon.svelte';

  let { PM, name }: {
    PM: Record<string, any>;
    name: string;
  } = $props();
</script>

<Icon {PM} {name} />
