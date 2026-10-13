<script lang="ts">
  import { X } from '@lucide/svelte';
  import { GITHUB } from '$lib/links';
  import { download, resolveDownload } from '$lib/download.svelte';

  // `onDark` is for the home page, where the bar sits on the hero's gradient.
  let { onDark = false }: { onDark?: boolean } = $props();

  const items = [
    { label: 'Editor', href: '/#editor' },
    { label: 'Features', href: '/#features' },
  ];

  let open = $state(false);

  $effect(() => { resolveDownload(); });

  $effect(() => {
    if (!open) return;
    const query = matchMedia('(max-width: 767px)');
    const apply = () => document.documentElement.classList.toggle('overflow-hidden', query.matches);
    apply();
    query.addEventListener('change', apply);
    return () => {
      query.removeEventListener('change', apply);
      document.documentElement.classList.remove('overflow-hidden');
    };
  });
</script>

<header class="site-header" data-on-dark={onDark ? '' : undefined} data-state={open ? 'active' : undefined}>
  <nav class="bar" aria-label="Main navigation">
    <a class="mark" href="/#top" aria-label="Powermove home"><span aria-hidden="true"></span></a>
    <ul class="links">
      {#each items as item (item.href)}
        <li><a class="quiet" href={item.href} onclick={() => (open = false)}>{item.label}</a></li>
      {/each}
      <li><a href={GITHUB} target="_blank" rel="noopener">GitHub</a></li>
      <li><a href={download.href}>Download</a></li>
    </ul>
    <button class="menu" type="button" aria-label={open ? 'Close menu' : 'Open menu'} aria-expanded={open} onclick={() => (open = !open)}>
      <span class="lines" aria-hidden="true"><i></i><i></i></span>
      <X class="close" size={20} aria-hidden="true" />
    </button>
  </nav>
  <div class="sheet">
    <ul>
      {#each items as item (item.href)}
        <li><a href={item.href} onclick={() => (open = false)}>{item.label}</a></li>
      {/each}
      <li><a href={GITHUB} target="_blank" rel="noopener">GitHub</a></li>
    </ul>
    <a class="pill dark" href={download.href}>Download for Mac</a>
  </div>
</header>
