<script lang="ts">
  import { X } from '@lucide/svelte';
  import { GITHUB, PRODUCT_HUNT } from '$lib/links';
  import { download, resolveDownload } from '$lib/download.svelte';

  const items = [
    { label: 'Editor', href: '/#editor' },
    { label: 'Agent', href: '/#agent' },
    { label: 'Mods', href: '/#mods' },
    { label: 'Export', href: '/#export' },
  ];

  let open = $state(false);

  $effect(() => { resolveDownload(); });

  $effect(() => {
    if (!open) return;
    const query = matchMedia('(max-width: 1023px)');
    const apply = () => document.documentElement.classList.toggle('overflow-hidden', query.matches);
    apply();
    query.addEventListener('change', apply);
    return () => {
      query.removeEventListener('change', apply);
      document.documentElement.classList.remove('overflow-hidden');
    };
  });
</script>

<header class="site-header" data-state={open ? 'active' : undefined}>
  <nav aria-label="Main navigation">
    <div class="bar">
      <a class="logo" href="/#top" aria-label="Powermove home">
        <img src="/powermove-logo.svg" width="24" height="21" alt="" />
        <span>Powermove</span>
      </a>
      <ul class="links">
        {#each items as item (item.href)}
          <li><a href={item.href} onclick={() => (open = false)}>{item.label}</a></li>
        {/each}
      </ul>
      <div class="actions">
        <a class="icon-button" href={GITHUB} target="_blank" rel="noopener" aria-label="Powermove on GitHub"><svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.56v-2c-3.2.7-3.87-1.36-3.87-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.76 2.69 1.25 3.34.96.1-.75.4-1.25.73-1.54-2.55-.29-5.24-1.28-5.24-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.78 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.26 5.67.41.36.78 1.05.78 2.12v3.14c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z"/></svg></a>
        <a class="button" href={PRODUCT_HUNT} target="_blank" rel="noopener">Upvote on Product Hunt</a>
        <a class="button primary" href={download.href}>Download</a>
      </div>
      <button class="menu" type="button" aria-label={open ? 'Close menu' : 'Open menu'} aria-expanded={open} onclick={() => (open = !open)}>
        <span class="lines" aria-hidden="true"><i></i><i></i></span>
        <X class="close" size={22} aria-hidden="true" />
      </button>
    </div>
    <div class="sheet">
      <ul>
        {#each items as item (item.href)}
          <li><a href={item.href} onclick={() => (open = false)}>{item.label}</a></li>
        {/each}
      </ul>
      <div class="sheet-actions">
        <a class="button" href={GITHUB} target="_blank" rel="noopener">Star on GitHub</a>
        <a class="button" href={PRODUCT_HUNT} target="_blank" rel="noopener">Upvote on Product Hunt</a>
        <a class="button primary" href={download.href}>Download for macOS</a>
      </div>
    </div>
  </nav>
</header>
<div class="header-fade" aria-hidden="true"></div>
