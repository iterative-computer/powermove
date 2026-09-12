<script lang="ts">
  let { PM, name }: {
    PM: Record<string, any>;
    name: string;
  } = $props();

  const known = $derived(!!PM.ICONS?.[name]);
  const path = $derived(PM.ICONS?.[name] ?? PM.ICONS?.missing ?? '');

  $effect(() => {
    if (!known) window.console.warn('Unknown Powermove icon:', name);
  });
</script>

<svg
  class="pm-icon"
  viewBox="0 0 256 256"
  aria-hidden="true"
  focusable="false"
  data-icon={known ? name : 'missing'}
  data-icon-set="phosphor"
  style="fill:currentColor;stroke:none"
>{@html path}</svg>
