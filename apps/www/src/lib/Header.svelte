<script lang="ts">
  import { X } from '@lucide/svelte';

  const items = [
    { label: 'Editor', href: '#editor' },
    { label: 'Agent', href: '#agent' },
    { label: 'Mods', href: '#mods' },
    { label: 'Export', href: '#export' },
  ];

  let open = $state(false);

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
      <a class="logo" href="#top" aria-label="Powermove home">
        <img src="/powermove-logo.svg" width="24" height="21" alt="" />
        <span>Powermove</span>
      </a>
      <ul class="links">
        {#each items as item (item.href)}
          <li><a href={item.href} onclick={() => (open = false)}>{item.label}</a></li>
        {/each}
      </ul>
      <div class="actions">
        <a class="button" href="#editor">See the editor</a>
        <a class="button primary" href="https://github.com/iterative-computer/powermove" target="_blank" rel="noopener">Star on GitHub</a>
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
        <a class="button" href="#editor" onclick={() => (open = false)}>See the editor</a>
        <a class="button primary" href="https://github.com/iterative-computer/powermove" target="_blank" rel="noopener">Star on GitHub</a>
      </div>
    </div>
  </nav>
</header>
<div class="header-fade" aria-hidden="true"></div>
