<script lang="ts">
  import BrandMark from '$lib/BrandMark.svelte';

  let { onDark = true }: { onDark?: boolean } = $props();
</script>

<header class="site-header" data-on-dark={onDark ? '' : undefined}>
  <nav class="bar" aria-label="Main navigation">
    <a class="brand-link" href="/#top" aria-label="Powermove home">
      <BrandMark variant="powermove" idPrefix="hdr" height={34} />
    </a>
  </nav>
</header>
